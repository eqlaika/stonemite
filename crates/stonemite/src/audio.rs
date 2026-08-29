//! Trigger audio dispatcher: managed WAV/MP3 playback and Windows TTS.
//!
//! One background thread owns all trigger audio so speech and sounds
//! serialize instead of talking over each other. Requests carry the EQLP
//! priority (lower value = more important): a more important arrival
//! preempts queued — and, for TTS, in-flight — audio. The dispatcher only
//! ever plays managed assets, bundled sounds, or speaks sanitized text;
//! nothing here can touch input or game state.

use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use crate::diagnostics::debug_log;

/// EQLP volume code for "no change".
const VOLUME_NO_CHANGE: i32 = 4;
/// Bound queued audio so a trigger storm cannot backlog for minutes.
const MAX_QUEUE: usize = 32;

#[derive(Clone, Debug)]
enum Payload {
    /// A managed asset or bundled sound resolved to a concrete source.
    SoundFile(PathBuf),
    Builtin(&'static str),
    Speak {
        text: String,
        /// SAPI rate (-10..=10); `None` = system default.
        rate: Option<i32>,
    },
}

#[derive(Clone, Debug)]
struct Request {
    priority: i64,
    sequence: u64,
    volume: i32,
    payload: Payload,
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}
impl Eq for Request {}
impl PartialOrd for Request {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for Request {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // BinaryHeap is a max-heap: lower priority value (more important)
        // and lower sequence (older) must compare greater.
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

struct Dispatcher {
    queue: Mutex<(BinaryHeap<Request>, u64)>,
    wake: Condvar,
    /// Most important queued priority, for in-flight preemption checks.
    pending_priority: AtomicI64,
    stop: AtomicBool,
}

fn dispatcher() -> &'static Dispatcher {
    static DISPATCHER: OnceLock<&'static Dispatcher> = OnceLock::new();
    DISPATCHER.get_or_init(|| {
        let dispatcher: &'static Dispatcher = Box::leak(Box::new(Dispatcher {
            queue: Mutex::new((BinaryHeap::new(), 0)),
            wake: Condvar::new(),
            pending_priority: AtomicI64::new(i64::MAX),
            stop: AtomicBool::new(false),
        }));
        if let Err(error) = std::thread::Builder::new()
            .name("stonemite-trigger-audio".to_owned())
            .spawn(move || worker(dispatcher))
        {
            debug_log(&format!("trigger audio thread failed to start: {error}"));
        }
        dispatcher
    })
}

fn enqueue(priority: i64, volume: i32, payload: Payload) {
    let dispatcher = dispatcher();
    {
        let mut guard = dispatcher
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (queue, sequence) = &mut *guard;
        if queue.len() >= MAX_QUEUE {
            // Drop the least important queued item instead of the new one.
            let mut items: Vec<Request> = std::mem::take(queue).into_vec();
            items.sort();
            items.remove(0);
            *queue = items.into();
        }
        *sequence += 1;
        queue.push(Request {
            priority,
            sequence: *sequence,
            volume,
            payload,
        });
        let best = queue.peek().map(|r| r.priority).unwrap_or(i64::MAX);
        dispatcher.pending_priority.store(best, Ordering::Release);
    }
    dispatcher.wake.notify_one();
}

/// Play a trigger sound by reference: managed asset name first, then a
/// bundled notification sound id. Unknown references are logged and dropped.
pub fn play_trigger_sound(reference: &str, volume: i32, priority: i64) {
    let catalog = crate::log_watcher::sound_catalog();
    if let Some(path) = catalog
        .as_ref()
        .and_then(|catalog| catalog.get(&reference.to_ascii_lowercase()).cloned())
    {
        enqueue(priority, volume, Payload::SoundFile(path));
        return;
    }
    if let Some(builtin) = crate::sound::find_id(reference) {
        enqueue(priority, volume, Payload::Builtin(builtin));
        return;
    }
    debug_log(&format!(
        "trigger sound '{reference}' is not a managed asset or bundled sound"
    ));
}

/// Speak trigger text with the system voice.
pub fn speak(text: &str, rate: Option<i32>, volume: i32, priority: i64) {
    if text.trim().is_empty() {
        return;
    }
    enqueue(
        priority,
        volume,
        Payload::Speak {
            text: text.to_owned(),
            rate,
        },
    );
}

/// Preview a managed sound file (settings-process test bench / editor).
pub fn preview_file(path: &std::path::Path) {
    enqueue(0, VOLUME_NO_CHANGE, Payload::SoundFile(path.to_owned()));
}

/// Drop queued audio and stop in-flight speech (shutdown, library reload).
pub fn stop_all() {
    let dispatcher = dispatcher();
    {
        let mut guard = dispatcher
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.0.clear();
        dispatcher
            .pending_priority
            .store(i64::MAX, Ordering::Release);
    }
    dispatcher.stop.store(true, Ordering::Release);
    dispatcher.wake.notify_one();
}

/// Map the EQLP volume code (4 = no change, lower = louder) onto a SAPI/MCI
/// percentage. SAPI caps at 100, so only reductions apply exactly.
fn volume_percent(code: i32) -> u32 {
    if code <= VOLUME_NO_CHANGE {
        100
    } else {
        (100 - (code - VOLUME_NO_CHANGE) * 15).clamp(10, 100) as u32
    }
}

fn worker(dispatcher: &'static Dispatcher) {
    let tts = tts::Voice::new();
    loop {
        let request = {
            let mut guard = dispatcher
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                if dispatcher.stop.swap(false, Ordering::AcqRel) {
                    if let Some(voice) = &tts {
                        voice.purge();
                    }
                }
                if let Some(request) = guard.0.pop() {
                    let best = guard.0.peek().map(|r| r.priority).unwrap_or(i64::MAX);
                    dispatcher.pending_priority.store(best, Ordering::Release);
                    break request;
                }
                dispatcher
                    .pending_priority
                    .store(i64::MAX, Ordering::Release);
                let (next, _) = dispatcher
                    .wake
                    .wait_timeout(guard, Duration::from_millis(500))
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard = next;
            }
        };

        let interrupted_by_better =
            || dispatcher.pending_priority.load(Ordering::Acquire) < request.priority;

        match &request.payload {
            Payload::Builtin(id) => {
                // Bundled sounds are short; fire asynchronously.
                let _ = crate::sound::play(id);
            }
            Payload::SoundFile(path) => {
                play_file(path, volume_percent(request.volume), &interrupted_by_better);
            }
            Payload::Speak { text, rate } => {
                if let Some(voice) = &tts {
                    voice.speak(
                        text,
                        *rate,
                        volume_percent(request.volume),
                        &interrupted_by_better,
                    );
                }
            }
        }
    }
}

/// Play a WAV via `PlaySoundW` (async + bounded wait) or an MP3 via MCI,
/// polling so a more important request can preempt.
fn play_file(path: &std::path::Path, volume: u32, interrupted: &dyn Fn() -> bool) {
    let lower = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if lower == "wav" {
        wav::play(path, interrupted);
    } else {
        mci::play(path, volume, interrupted);
    }
}

mod wav {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Media::Audio::{
        PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT, SND_PURGE,
    };

    pub(super) fn play(path: &std::path::Path, interrupted: &dyn Fn() -> bool) {
        let wide = HSTRING::from(path.as_os_str());
        let started = unsafe {
            PlaySoundW(
                PCWSTR(wide.as_ptr()),
                HMODULE::default(),
                SND_ASYNC | SND_FILENAME | SND_NODEFAULT,
            )
        }
        .as_bool();
        if !started {
            return;
        }
        // PlaySound cannot report progress; wait a bounded slice so queued
        // audio stays serialized, preempting on a more important arrival.
        for _ in 0..100 {
            if interrupted() {
                unsafe {
                    let _ = PlaySoundW(PCWSTR::null(), HMODULE::default(), SND_PURGE);
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

mod mci {
    use windows::core::HSTRING;
    use windows::Win32::Media::Multimedia::mciSendStringW;

    use crate::diagnostics::debug_log;

    fn send(command: &str) -> Result<String, u32> {
        let wide = HSTRING::from(command);
        let mut buffer = [0u16; 256];
        let result = unsafe { mciSendStringW(&wide, Some(&mut buffer), None) };
        if result != 0 {
            return Err(result);
        }
        let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Ok(String::from_utf16_lossy(&buffer[..length]))
    }

    pub(super) fn play(path: &std::path::Path, volume: u32, interrupted: &dyn Fn() -> bool) {
        let alias = format!("smtrig{}", std::process::id());
        let open = format!("open \"{}\" type mpegvideo alias {alias}", path.display());
        if let Err(error) = send(&open) {
            debug_log(&format!("MCI open failed ({error}): {}", path.display()));
            return;
        }
        let _ = send(&format!("setaudio {alias} volume to {}", volume * 10));
        if send(&format!("play {alias}")).is_ok() {
            // Poll until playback stops or something more important arrives.
            for _ in 0..1_200 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if interrupted() {
                    break;
                }
                match send(&format!("status {alias} mode")) {
                    Ok(mode) if mode.trim() == "playing" => continue,
                    _ => break,
                }
            }
        }
        let _ = send(&format!("close {alias}"));
    }
}

mod tts {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::{ISpVoice, SpVoice, SPF_ASYNC, SPF_PURGEBEFORESPEAK};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    use crate::diagnostics::debug_log;

    /// The dispatcher thread's SAPI voice. Missing voices degrade to silence
    /// with a diagnostic instead of failing triggers.
    pub(super) struct Voice {
        voice: ISpVoice,
    }

    impl Voice {
        pub(super) fn new() -> Option<Self> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                match CoCreateInstance::<_, ISpVoice>(&SpVoice, None, CLSCTX_ALL) {
                    Ok(voice) => Some(Self { voice }),
                    Err(error) => {
                        debug_log(&format!(
                            "Windows TTS is unavailable; trigger speech disabled: {error}"
                        ));
                        None
                    }
                }
            }
        }

        pub(super) fn purge(&self) {
            unsafe {
                let _ = self.voice.Speak(None, SPF_PURGEBEFORESPEAK.0 as u32, None);
            }
        }

        pub(super) fn speak(
            &self,
            text: &str,
            rate: Option<i32>,
            volume: u32,
            interrupted: &dyn Fn() -> bool,
        ) {
            unsafe {
                if let Some(rate) = rate {
                    let _ = self.voice.SetRate(rate.clamp(-10, 10));
                }
                let _ = self.voice.SetVolume(volume.clamp(0, 100) as u16);
                let wide = HSTRING::from(text);
                if self
                    .voice
                    .Speak(&wide, (SPF_ASYNC.0 | SPF_PURGEBEFORESPEAK.0) as u32, None)
                    .is_err()
                {
                    return;
                }
                // Bounded wait with preemption: a more important request
                // purges the utterance mid-speech.
                for _ in 0..600 {
                    if self.voice.WaitUntilDone(50).is_ok() {
                        break;
                    }
                    if interrupted() {
                        self.purge();
                        break;
                    }
                }
                if rate.is_some() {
                    // Restore the system default for the next utterance.
                    let _ = self.voice.SetRate(0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_orders_by_priority_then_arrival() {
        let mut heap = BinaryHeap::new();
        for (priority, sequence) in [(3, 1), (1, 2), (3, 3), (2, 4)] {
            heap.push(Request {
                priority,
                sequence,
                volume: 4,
                payload: Payload::Builtin("tell.wav"),
            });
        }
        let order: Vec<(i64, u64)> = std::iter::from_fn(|| heap.pop())
            .map(|request| (request.priority, request.sequence))
            .collect();
        assert_eq!(order, vec![(1, 2), (2, 4), (3, 1), (3, 3)]);
    }

    #[test]
    fn volume_codes_map_onto_percentages() {
        assert_eq!(volume_percent(0), 100);
        assert_eq!(volume_percent(4), 100);
        assert_eq!(volume_percent(5), 85);
        assert_eq!(volume_percent(10), 10);
    }
}
