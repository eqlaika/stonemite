//! Event-driven EQ log ingestion.
//!
//! `notify` provides low-latency directory wake-ups, while `LogTailer` remains
//! authoritative for byte offsets and complete-line framing. A dedicated
//! worker feeds canonical `RawLogLine` records through composable domain
//! parsers, a typed event model, the telemetry reducer, and the passive-only
//! trigger hook. Bounded batches return to the Win32 owner thread; a periodic
//! reconciliation pass makes filesystem notification loss recoverable.

mod diagnostic;
mod pipeline;
mod tailer;
mod triggers;
mod watcher;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

pub use diagnostic::{DiagnosticKind, LogDiagnostic};
#[allow(unused_imports)]
pub use eqlog::{
    CharacterEvent, CharacterKey, CharacterTelemetry, ChatEvent, DecodedRawLogLine, EqTimestamp,
    IdentityEvent, IncomingTell, LogEvent, LogEventDomain, LogSource, LogSourceId,
    NotificationEvent, ParsedLogEvent, PetEvent, RawLogLine, TelemetryChange, WhoResult,
};
pub use pipeline::LogEnvelope;
#[allow(unused_imports)]
pub use triggers::{
    PresentationAction, TimerRequest, TriggerActivation, TriggerDefinition, TriggerDefinitionError,
    TriggerMatcher, TriggerScope,
};

use pipeline::LogPipeline;
use tailer::{LogTailer, PathKey, ReadBudget, SourceSpec};
use watcher::DirectoryWatcher;

pub const WM_LOG_READY: u32 = WM_USER + 21;

// Native filesystem notifications remain the primary low-latency path. Keep
// the bounded metadata/offset fallback short enough that a coalesced or early
// write notification cannot make one box visibly lag the others.
const RECONCILIATION_INTERVAL: Duration = Duration::from_millis(500);
const BACKPRESSURE_RETRY: Duration = Duration::from_millis(10);
const OUTPUT_QUEUE_CAPACITY: usize = 32;
const EVENT_BUS_CAPACITY: usize = 512;
const MAX_UI_DRAIN_BATCHES: usize = 8;
const MAX_PENDING_EVENT_PATHS: usize = 1024;

#[derive(Default)]
pub(crate) struct LogBatch {
    pub envelopes: Vec<Arc<LogEnvelope>>,
    pub diagnostics: Vec<LogDiagnostic>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct DesiredSources {
    logs_dir: Option<PathBuf>,
    sources: Vec<LogSource>,
}

pub(crate) struct WorkerWake {
    signal_sender: mpsc::SyncSender<()>,
    changed_paths: Mutex<HashMap<PathKey, PathBuf>>,
    force_reconcile: AtomicBool,
    watcher_error: Mutex<Option<String>>,
    sources_changed: AtomicBool,
    stop: AtomicBool,
}

impl WorkerWake {
    fn new(signal_sender: mpsc::SyncSender<()>) -> Self {
        Self {
            signal_sender,
            changed_paths: Mutex::new(HashMap::new()),
            force_reconcile: AtomicBool::new(false),
            watcher_error: Mutex::new(None),
            sources_changed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        }
    }

    pub(crate) fn record_paths(&self, paths: Vec<PathBuf>) {
        let mut overflowed = false;
        let mut changed = lock(&self.changed_paths);
        for path in paths {
            if changed.len() >= MAX_PENDING_EVENT_PATHS {
                changed.clear();
                overflowed = true;
                break;
            }
            changed.insert(PathKey::new(&path), path);
        }
        drop(changed);
        if overflowed {
            self.record_watcher_error(
                "pending filesystem notification paths overflowed; falling back to reconciliation"
                    .to_owned(),
            );
        } else {
            self.signal();
        }
    }

    pub(crate) fn request_reconciliation(&self) {
        self.force_reconcile.store(true, Ordering::Release);
        self.signal();
    }

    pub(crate) fn record_watcher_error(&self, error: String) {
        *lock(&self.watcher_error) = Some(error);
        self.force_reconcile.store(true, Ordering::Release);
        self.signal();
    }

    fn signal(&self) {
        let _ = self.signal_sender.try_send(());
    }
}

struct OutputWake {
    hwnd: usize,
    posted: AtomicBool,
}

impl OutputWake {
    fn post(&self) {
        if self.posted.swap(true, Ordering::AcqRel) {
            return;
        }
        let posted = unsafe {
            PostMessageW(
                HWND(self.hwnd as *mut _),
                WM_LOG_READY,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if posted.is_err() {
            self.posted.store(false, Ordering::Release);
        }
    }

    fn acknowledge(&self) {
        // Clear before draining. A producer racing with the drain will post a
        // fresh message rather than leaving queued output without a wake-up.
        self.posted.store(false, Ordering::Release);
    }
}

struct RuntimeState {
    wake: Arc<WorkerWake>,
    desired: Arc<Mutex<DesiredSources>>,
    output_receiver: mpsc::Receiver<LogBatch>,
    output_wake: Arc<OutputWake>,
    event_bus: broadcast::Sender<Arc<LogEnvelope>>,
    worker: Option<JoinHandle<()>>,
}

static RUNTIME: Mutex<Option<RuntimeState>> = Mutex::new(None);

pub fn start(hwnd: HWND) -> Result<(), String> {
    let mut runtime = lock(&RUNTIME);
    if runtime.is_some() {
        return Ok(());
    }

    let (signal_sender, signal_receiver) = mpsc::sync_channel(1);
    let (output_sender, output_receiver) = mpsc::sync_channel(OUTPUT_QUEUE_CAPACITY);
    let (event_bus, _) = broadcast::channel(EVENT_BUS_CAPACITY);
    let wake = Arc::new(WorkerWake::new(signal_sender));
    let desired = Arc::new(Mutex::new(DesiredSources::default()));
    let output_wake = Arc::new(OutputWake {
        hwnd: hwnd.0 as usize,
        posted: AtomicBool::new(false),
    });

    let worker_wake = wake.clone();
    let worker_desired = desired.clone();
    let worker_output_wake = output_wake.clone();
    let worker_event_bus = event_bus.clone();
    let worker = std::thread::Builder::new()
        .name("stonemite-eq-logs".to_owned())
        .spawn(move || {
            worker_main(
                worker_wake,
                worker_desired,
                signal_receiver,
                output_sender,
                worker_output_wake,
                worker_event_bus,
            );
        })
        .map_err(|error| format!("could not start EQ log worker: {error}"))?;

    *runtime = Some(RuntimeState {
        wake,
        desired,
        output_receiver,
        output_wake,
        event_bus,
        worker: Some(worker),
    });
    Ok(())
}

/// Last-write-wins source publication from the UI owner thread. It performs no
/// filesystem I/O and does not accumulate commands when the window poll repeats
/// an unchanged identity snapshot.
pub fn replace_sources(logs_dir: PathBuf, mut sources: Vec<LogSource>) {
    let runtime = lock(&RUNTIME);
    let Some(state) = runtime.as_ref() else {
        return;
    };
    sources.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    let replacement = DesiredSources {
        logs_dir: Some(logs_dir),
        sources,
    };
    let mut desired = lock(&state.desired);
    if *desired == replacement {
        return;
    }
    *desired = replacement;
    drop(desired);
    state.wake.sources_changed.store(true, Ordering::Release);
    state.wake.signal();
}

pub(crate) fn drain_ready() -> Vec<LogBatch> {
    let runtime = lock(&RUNTIME);
    let Some(state) = runtime.as_ref() else {
        return Vec::new();
    };
    state.output_wake.acknowledge();
    let mut batches = Vec::new();
    for _ in 0..MAX_UI_DRAIN_BATCHES {
        match state.output_receiver.try_recv() {
            Ok(batch) => batches.push(batch),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
    if batches.len() == MAX_UI_DRAIN_BATCHES {
        state.output_wake.post();
    }
    batches
}

#[allow(dead_code)]
pub fn subscribe() -> Option<broadcast::Receiver<Arc<LogEnvelope>>> {
    lock(&RUNTIME)
        .as_ref()
        .map(|state| state.event_bus.subscribe())
}

pub fn stop() {
    let state = lock(&RUNTIME).take();
    let Some(mut state) = state else {
        return;
    };
    state.wake.stop.store(true, Ordering::Release);
    state.wake.signal();
    if let Some(worker) = state.worker.take() {
        let _ = worker.join();
    }
}

fn worker_main(
    wake: Arc<WorkerWake>,
    desired: Arc<Mutex<DesiredSources>>,
    signal_receiver: mpsc::Receiver<()>,
    output_sender: mpsc::SyncSender<LogBatch>,
    output_wake: Arc<OutputWake>,
    event_bus: broadcast::Sender<Arc<LogEnvelope>>,
) {
    let mut worker = LogWorker {
        wake,
        desired,
        signal_receiver,
        output_sender,
        output_wake,
        watcher: None,
        tailer: LogTailer::new(),
        pipeline: LogPipeline::new(event_bus),
        active_desired: DesiredSources::default(),
        ready_order: VecDeque::new(),
        ready: HashMap::new(),
        deferred_diagnostics: Vec::new(),
        pending_output: None,
        next_reconciliation: Instant::now() + RECONCILIATION_INTERVAL,
    };
    worker.run();
}

struct LogWorker {
    wake: Arc<WorkerWake>,
    desired: Arc<Mutex<DesiredSources>>,
    signal_receiver: mpsc::Receiver<()>,
    output_sender: mpsc::SyncSender<LogBatch>,
    output_wake: Arc<OutputWake>,
    watcher: Option<DirectoryWatcher>,
    tailer: LogTailer,
    pipeline: LogPipeline,
    active_desired: DesiredSources,
    ready_order: VecDeque<PathKey>,
    ready: HashMap<PathKey, WorkItem>,
    deferred_diagnostics: Vec<LogDiagnostic>,
    pending_output: Option<LogBatch>,
    next_reconciliation: Instant,
}

impl LogWorker {
    fn run(&mut self) {
        loop {
            if self.wake.stop.load(Ordering::Acquire) {
                return;
            }

            self.apply_source_changes();
            self.collect_watcher_activity();
            self.maybe_reconcile();

            if !self.flush_pending_output() {
                return;
            }
            if self.pending_output.is_some() {
                let _ = self.signal_receiver.recv_timeout(BACKPRESSURE_RETRY);
                continue;
            }

            if !self.deferred_diagnostics.is_empty() {
                self.pending_output = Some(LogBatch {
                    envelopes: Vec::new(),
                    diagnostics: std::mem::take(&mut self.deferred_diagnostics),
                });
                continue;
            }

            if let Some(work) = self.pop_work() {
                let batch = process_work_item(
                    &mut self.tailer,
                    &mut self.pipeline,
                    &work,
                    ReadBudget::default(),
                );
                let more_work = batch.1;
                if more_work {
                    self.schedule(
                        work.path.clone(),
                        work.origin.after_first_read(!batch.0.envelopes.is_empty()),
                    );
                }
                if !batch.0.envelopes.is_empty() || !batch.0.diagnostics.is_empty() {
                    self.pending_output = Some(batch.0);
                }
                continue;
            }

            let timeout = self
                .next_reconciliation
                .saturating_duration_since(Instant::now());
            let _ = self.signal_receiver.recv_timeout(timeout);
        }
    }

    fn flush_pending_output(&mut self) -> bool {
        let Some(batch) = self.pending_output.take() else {
            return true;
        };
        match self.output_sender.try_send(batch) {
            Ok(()) => {
                self.output_wake.post();
                true
            }
            Err(mpsc::TrySendError::Full(batch)) => {
                self.pending_output = Some(batch);
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        }
    }

    fn apply_source_changes(&mut self) {
        if !self.wake.sources_changed.swap(false, Ordering::AcqRel) {
            return;
        }
        let replacement = lock(&self.desired).clone();
        let specs = replacement
            .logs_dir
            .as_ref()
            .map(|logs_dir| {
                replacement
                    .sources
                    .iter()
                    .cloned()
                    .map(|source| SourceSpec {
                        path: logs_dir
                            .join(format!("eqlog_{}_{}.txt", source.character, source.server)),
                        source,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let outcome = self.tailer.sync_sources(specs);
        for source in outcome.removed_sources {
            self.pipeline.reset_source(&source);
        }
        self.deferred_diagnostics.extend(outcome.diagnostics);
        self.active_desired = replacement;
        self.ensure_watcher();
    }

    fn ensure_watcher(&mut self) {
        let Some(logs_dir) = self.active_desired.logs_dir.clone() else {
            return;
        };
        let watcher = self
            .watcher
            .get_or_insert_with(|| DirectoryWatcher::new(self.wake.clone()));
        self.deferred_diagnostics
            .extend(watcher.ensure_directory(&logs_dir));
    }

    fn collect_watcher_activity(&mut self) {
        if let Some(error) = lock(&self.wake.watcher_error).take() {
            // A backend error can mean more than one coalesced event was lost.
            // Drop and recreate the adapter while the forced offset
            // reconciliation remains the correctness path.
            self.watcher = None;
            self.deferred_diagnostics.push(LogDiagnostic::new(
                DiagnosticKind::WatcherError,
                self.active_desired.logs_dir.clone(),
                format!(
                    "filesystem notification error: {error}; restarting watcher and scheduling reconciliation"
                ),
            ));
        }

        let paths = std::mem::take(&mut *lock(&self.wake.changed_paths));
        for (_, path) in paths {
            if self.tailer.is_tracked(&path) {
                self.schedule(path, WorkOrigin::Notification);
            }
        }
    }

    fn maybe_reconcile(&mut self) {
        let forced = self.wake.force_reconcile.swap(false, Ordering::AcqRel);
        if !forced && Instant::now() < self.next_reconciliation {
            return;
        }
        self.ensure_watcher();
        for path in self.tailer.tracked_paths() {
            self.schedule(path, WorkOrigin::Reconciliation { reported: false });
        }
        self.next_reconciliation = Instant::now() + RECONCILIATION_INTERVAL;
    }

    fn schedule(&mut self, path: PathBuf, origin: WorkOrigin) {
        let key = PathKey::new(&path);
        if let Some(existing) = self.ready.get_mut(&key) {
            existing.origin.merge(origin);
            return;
        }
        self.ready_order.push_back(key.clone());
        self.ready.insert(key, WorkItem { path, origin });
    }

    fn pop_work(&mut self) -> Option<WorkItem> {
        while let Some(key) = self.ready_order.pop_front() {
            if let Some(work) = self.ready.remove(&key) {
                if self.tailer.is_tracked(&work.path) {
                    return Some(work);
                }
            }
        }
        None
    }
}

#[derive(Clone)]
struct WorkItem {
    path: PathBuf,
    origin: WorkOrigin,
}

#[derive(Clone, Copy)]
enum WorkOrigin {
    Notification,
    Reconciliation { reported: bool },
}

impl WorkOrigin {
    fn merge(&mut self, other: Self) {
        match (*self, other) {
            (Self::Notification, _) | (_, Self::Notification) => *self = Self::Notification,
            (
                Self::Reconciliation { reported: current },
                Self::Reconciliation { reported: other },
            ) => {
                *self = Self::Reconciliation {
                    reported: current || other,
                }
            }
        }
    }

    fn after_first_read(self, emitted: bool) -> Self {
        match self {
            Self::Reconciliation { reported } => Self::Reconciliation {
                reported: reported || emitted,
            },
            Self::Notification => Self::Notification,
        }
    }
}

fn process_work_item(
    tailer: &mut LogTailer,
    pipeline: &mut LogPipeline,
    work: &WorkItem,
    budget: ReadBudget,
) -> (LogBatch, bool) {
    let outcome = tailer.read_path(&work.path, budget);
    if let Some(source) = &outcome.generation_reset {
        pipeline.reset_source(source);
    }
    let mut batch = LogBatch {
        envelopes: Vec::with_capacity(outcome.lines.len()),
        diagnostics: outcome.diagnostics,
    };
    for line in outcome.lines {
        let processed = pipeline.process(line);
        batch.envelopes.push(processed.envelope);
        batch.diagnostics.extend(processed.diagnostics);
    }
    if matches!(work.origin, WorkOrigin::Reconciliation { reported: false })
        && !batch.envelopes.is_empty()
    {
        batch.diagnostics.push(LogDiagnostic::new(
            DiagnosticKind::Reconciliation,
            Some(work.path.clone()),
            format!(
                "recovered {} complete log record(s) without relying on a filesystem event",
                batch.envelopes.len()
            ),
        ));
    }
    (batch, outcome.more_work)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use super::*;

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("stonemite-runtime-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct RuntimeGuard;

    impl Drop for RuntimeGuard {
        fn drop(&mut self) {
            stop();
        }
    }

    #[test]
    fn native_filesystem_notification_delivers_before_fallback_reconciliation() {
        stop();
        let directory = TempDirectory::new();
        let path = directory.0.join("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        start(HWND::default()).unwrap();
        let _runtime = RuntimeGuard;
        replace_sources(
            directory.0.clone(),
            vec![LogSource::new("client-1", "Bilka", "teek")],
        );

        // Source replacement and watcher setup happen on the worker. This is
        // setup synchronization only; delivery must still be attributed to a
        // native wake rather than the periodic reconciliation fallback.
        std::thread::sleep(Duration::from_millis(250));
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "[now] native notify").unwrap();
        file.flush().unwrap();
        drop(file);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let batches = drain_ready();
            if batches
                .iter()
                .flat_map(|batch| &batch.envelopes)
                .any(|envelope| {
                    envelope.raw.body.as_ref() == "native notify"
                        && envelope.raw.source.id.as_str() == "client-1"
                })
            {
                assert!(
                    !batches
                        .iter()
                        .flat_map(|batch| &batch.diagnostics)
                        .any(|diagnostic| diagnostic.kind == DiagnosticKind::Reconciliation),
                    "native test record was recovered only by reconciliation"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "native filesystem notification did not deliver before reconciliation"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn reconciliation_recovers_a_write_without_a_notification() {
        let directory = TempDirectory::new();
        let path = directory.0.join("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        let source = LogSource::new("client-1", "Bilka", "teek");
        let mut tailer = LogTailer::new();
        let sync = tailer.sync_sources(vec![SourceSpec {
            path: path.clone(),
            source,
        }]);
        assert!(sync.diagnostics.is_empty());

        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "[now] reconciled").unwrap();
        file.flush().unwrap();

        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let (batch, more_work) = process_work_item(
            &mut tailer,
            &mut pipeline,
            &WorkItem {
                path: path.clone(),
                origin: WorkOrigin::Reconciliation { reported: false },
            },
            ReadBudget::default(),
        );
        assert!(!more_work);
        assert_eq!(batch.envelopes.len(), 1);
        assert_eq!(batch.envelopes[0].raw.body.as_ref(), "reconciled");
        assert!(batch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::Reconciliation));
    }
}
