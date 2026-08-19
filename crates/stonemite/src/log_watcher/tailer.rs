use std::collections::{HashMap, HashSet};
use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};

use eqlog::{LogSource, RawLogLine};

use super::diagnostic::{DiagnosticKind, LogDiagnostic};

const READ_CHUNK_BYTES: usize = 64 * 1024;
const SIGNATURE_BYTES: usize = 64;
const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadBudget {
    pub bytes: usize,
    pub records: usize,
}

impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            bytes: 256 * 1024,
            records: 256,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SourceSpec {
    pub path: PathBuf,
    pub source: LogSource,
}

#[derive(Default)]
pub(crate) struct SyncOutcome {
    pub removed_sources: Vec<LogSource>,
    pub diagnostics: Vec<LogDiagnostic>,
}

#[derive(Default)]
pub(crate) struct ReadOutcome {
    pub lines: Vec<RawLogLine>,
    pub diagnostics: Vec<LogDiagnostic>,
    pub more_work: bool,
    pub generation_reset: Option<LogSource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume_serial: Option<u32>,
    file_index: Option<u64>,
    creation_time: u64,
}

impl FileIdentity {
    fn from_file(file: &File) -> Self {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        let available = unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information).is_ok()
        };
        if available {
            let creation_time = (u64::from(information.ftCreationTime.dwHighDateTime) << 32)
                | u64::from(information.ftCreationTime.dwLowDateTime);
            let file_index = (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow);
            Self {
                volume_serial: Some(information.dwVolumeSerialNumber),
                file_index: Some(file_index),
                creation_time,
            }
        } else {
            // The offset-boundary signature still detects path replacement and
            // truncate/regrow when a filesystem cannot provide a stable ID.
            Self {
                volume_serial: None,
                file_index: None,
                creation_time: 0,
            }
        }
    }
}

struct FileState {
    path: PathBuf,
    source: LogSource,
    initialized: bool,
    identity: Option<FileIdentity>,
    offset: u64,
    pending: Vec<u8>,
    discard_until_newline: bool,
    signature: Vec<u8>,
    generation: u64,
}

impl FileState {
    fn absent(path: PathBuf, source: LogSource) -> Self {
        Self {
            path,
            source,
            initialized: false,
            identity: None,
            offset: 0,
            pending: Vec::new(),
            discard_until_newline: false,
            signature: Vec::new(),
            generation: 0,
        }
    }
}

/// Authoritative byte-offset tailer. Filesystem notifications only decide
/// when its methods are called; all content and exactly-once framing decisions
/// live here.
pub(crate) struct LogTailer {
    files: HashMap<PathKey, FileState>,
}

impl LogTailer {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    pub fn sync_sources(&mut self, sources: Vec<SourceSpec>) -> SyncOutcome {
        let mut outcome = SyncOutcome::default();
        let mut expected = HashMap::new();
        for spec in sources {
            expected.entry(PathKey::new(&spec.path)).or_insert(spec);
        }

        let expected_keys: HashSet<_> = expected.keys().cloned().collect();
        self.files.retain(|key, state| {
            if expected_keys.contains(key) {
                true
            } else {
                outcome.removed_sources.push(state.source.clone());
                false
            }
        });

        for (key, spec) in expected {
            if let Some(state) = self.files.get_mut(&key) {
                if state.source != spec.source {
                    outcome.removed_sources.push(state.source.clone());
                    state.source = spec.source;
                    // Framing remains tied to the file path and byte offset.
                    // Preserve an incomplete record across a client/process
                    // reassociation so source publication cannot lose bytes.
                }
                state.path = spec.path;
                continue;
            }

            let mut state = FileState::absent(spec.path, spec.source);
            match File::open(&state.path) {
                Ok(mut file) => match file.metadata() {
                    Ok(metadata) => {
                        if let Err(error) = initialize_at_end(&mut state, &mut file, &metadata) {
                            outcome.diagnostics.push(file_diagnostic(
                                DiagnosticKind::FileRead,
                                &state.path,
                                format!("could not initialize at EOF: {error}"),
                            ));
                        }
                    }
                    Err(error) => outcome.diagnostics.push(file_diagnostic(
                        DiagnosticKind::FileRead,
                        &state.path,
                        format!("could not read metadata: {error}"),
                    )),
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => outcome.diagnostics.push(file_diagnostic(
                    DiagnosticKind::FileRead,
                    &state.path,
                    format!("could not open log: {error}"),
                )),
            }
            self.files.insert(key, state);
        }

        outcome
    }

    pub fn tracked_paths(&self) -> Vec<PathBuf> {
        self.files
            .values()
            .map(|state| state.path.clone())
            .collect()
    }

    pub fn is_tracked(&self, path: &Path) -> bool {
        self.files.contains_key(&PathKey::new(path))
    }

    pub fn read_path(&mut self, path: &Path, budget: ReadBudget) -> ReadOutcome {
        let Some(state) = self.files.get_mut(&PathKey::new(path)) else {
            return ReadOutcome::default();
        };
        read_file(state, budget)
    }
}

fn initialize_at_end(
    state: &mut FileState,
    file: &mut File,
    metadata: &Metadata,
) -> io::Result<()> {
    let file_len = metadata.len();
    state.initialized = true;
    state.identity = Some(FileIdentity::from_file(file));
    state.offset = file_len;
    state.pending.clear();
    state.signature = read_suffix(file, file_len)?;
    state.discard_until_newline = file_len > 0 && state.signature.last() != Some(&b'\n');
    Ok(())
}

fn read_file(state: &mut FileState, budget: ReadBudget) -> ReadOutcome {
    let mut outcome = ReadOutcome::default();
    let mut file = match File::open(&state.path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return outcome,
        Err(error) => {
            outcome.diagnostics.push(file_diagnostic(
                DiagnosticKind::FileRead,
                &state.path,
                format!("could not open log: {error}"),
            ));
            return outcome;
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            outcome.diagnostics.push(file_diagnostic(
                DiagnosticKind::FileRead,
                &state.path,
                format!("could not read metadata: {error}"),
            ));
            return outcome;
        }
    };

    if !state.initialized {
        if let Err(error) = initialize_at_end(state, &mut file, &metadata) {
            outcome.diagnostics.push(file_diagnostic(
                DiagnosticKind::FileRead,
                &state.path,
                format!("could not initialize at EOF: {error}"),
            ));
        }
        return outcome;
    }

    let current_identity = FileIdentity::from_file(&file);
    let recreated = state
        .identity
        .as_ref()
        .is_some_and(|identity| identity != &current_identity);
    let shrank = metadata.len() < state.offset;
    let boundary_changed = !recreated
        && !shrank
        && !state.signature.is_empty()
        && !signature_matches(&mut file, state.offset, &state.signature).unwrap_or(false);

    if recreated || shrank || boundary_changed {
        let reason = if recreated {
            "file was recreated"
        } else if shrank {
            "file was truncated"
        } else {
            "file contents changed before the stored offset"
        };
        reset_generation(state, current_identity.clone());
        outcome.generation_reset = Some(state.source.clone());
        outcome.diagnostics.push(file_diagnostic(
            DiagnosticKind::FileReset,
            &state.path,
            format!("{reason}; restarting at byte zero"),
        ));
    } else {
        state.identity = Some(current_identity);
    }

    extract_complete_lines(state, budget.records, &mut outcome);
    if outcome.lines.len() >= budget.records {
        outcome.more_work = pending_has_complete_line(state) || metadata.len() > state.offset;
        return outcome;
    }

    if file.seek(SeekFrom::Start(state.offset)).is_err() {
        outcome.diagnostics.push(file_diagnostic(
            DiagnosticKind::FileRead,
            &state.path,
            format!("could not seek to byte {}", state.offset),
        ));
        return outcome;
    }

    let mut bytes_read = 0usize;
    let mut buffer = vec![0u8; READ_CHUNK_BYTES.min(budget.bytes.max(1))];
    while bytes_read < budget.bytes && outcome.lines.len() < budget.records {
        let remaining = budget.bytes - bytes_read;
        let read_len = remaining.min(buffer.len());
        let count = match file.read(&mut buffer[..read_len]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                outcome.diagnostics.push(file_diagnostic(
                    DiagnosticKind::FileRead,
                    &state.path,
                    format!("read failed at byte {}: {error}", state.offset),
                ));
                break;
            }
        };
        let bytes = &buffer[..count];
        state.offset += count as u64;
        bytes_read += count;
        extend_signature(&mut state.signature, bytes);
        append_bytes(state, bytes);
        extract_complete_lines(state, budget.records - outcome.lines.len(), &mut outcome);
    }

    let latest_len = file
        .metadata()
        .map(|value| value.len())
        .unwrap_or(metadata.len());
    outcome.more_work = pending_has_complete_line(state) || latest_len > state.offset;
    outcome
}

fn reset_generation(state: &mut FileState, identity: FileIdentity) {
    state.identity = Some(identity);
    state.offset = 0;
    state.pending.clear();
    state.discard_until_newline = false;
    state.signature.clear();
    state.generation = state.generation.wrapping_add(1);
}

fn append_bytes(state: &mut FileState, bytes: &[u8]) {
    if !state.discard_until_newline {
        state.pending.extend_from_slice(bytes);
        return;
    }

    if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
        state.discard_until_newline = false;
        state.pending.extend_from_slice(&bytes[newline + 1..]);
    }
}

fn extract_complete_lines(state: &mut FileState, max_records: usize, outcome: &mut ReadOutcome) {
    if max_records == 0 || state.pending.is_empty() {
        return;
    }

    let mut line_start = 0usize;
    let mut consumed = 0usize;
    let mut emitted = 0usize;
    while emitted < max_records {
        let Some(relative_newline) = state.pending[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
        else {
            break;
        };
        let newline = line_start + relative_newline;
        let mut line_end = newline;
        if line_end > line_start && state.pending[line_end - 1] == b'\r' {
            line_end -= 1;
        }

        if line_end - line_start > MAX_LINE_BYTES {
            outcome.diagnostics.push(file_diagnostic(
                DiagnosticKind::FileRead,
                &state.path,
                format!("discarded a log record larger than {MAX_LINE_BYTES} bytes"),
            ));
        } else {
            let decoded =
                RawLogLine::decode(state.source.clone(), &state.pending[line_start..line_end]);
            if decoded.had_invalid_utf8 {
                outcome.diagnostics.push(file_diagnostic(
                    DiagnosticKind::FileRead,
                    &state.path,
                    "replaced invalid UTF-8 in one complete log record",
                ));
            }
            outcome.lines.push(decoded.line);
            emitted += 1;
        }
        consumed = newline + 1;
        line_start = consumed;
    }

    if consumed > 0 {
        state.pending.drain(..consumed);
    }
    if state.pending.len() > MAX_LINE_BYTES && !state.pending.contains(&b'\n') {
        state.pending.clear();
        state.discard_until_newline = true;
        outcome.diagnostics.push(file_diagnostic(
            DiagnosticKind::FileRead,
            &state.path,
            format!(
                "discarding bytes until newline after a record exceeded {MAX_LINE_BYTES} bytes"
            ),
        ));
    }
}

fn pending_has_complete_line(state: &FileState) -> bool {
    state.pending.contains(&b'\n')
}

fn read_suffix(file: &mut File, offset: u64) -> io::Result<Vec<u8>> {
    let length = usize::try_from(offset.min(SIGNATURE_BYTES as u64)).unwrap_or(SIGNATURE_BYTES);
    if length == 0 {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(offset - length as u64))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn signature_matches(file: &mut File, offset: u64, expected: &[u8]) -> io::Result<bool> {
    if expected.is_empty() {
        return Ok(true);
    }
    file.seek(SeekFrom::Start(offset - expected.len() as u64))?;
    let mut actual = vec![0; expected.len()];
    file.read_exact(&mut actual)?;
    Ok(actual == expected)
}

fn extend_signature(signature: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() >= SIGNATURE_BYTES {
        signature.clear();
        signature.extend_from_slice(&bytes[bytes.len() - SIGNATURE_BYTES..]);
        return;
    }
    let overflow = signature
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(SIGNATURE_BYTES);
    if overflow > 0 {
        signature.drain(..overflow);
    }
    signature.extend_from_slice(bytes);
}

fn file_diagnostic(kind: DiagnosticKind, path: &Path, message: impl Into<String>) -> LogDiagnostic {
    LogDiagnostic::new(kind, Some(path.to_path_buf()), message)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PathKey(String);

impl PathKey {
    pub fn new(path: &Path) -> Self {
        Self(
            path.to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use super::*;

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("stonemite-log-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn source(client_id: u32, character: &str, server: &str) -> LogSource {
        LogSource::new(format!("client-{client_id}"), character, server)
    }

    fn track(tailer: &mut LogTailer, path: &Path, source: LogSource) {
        let outcome = tailer.sync_sources(vec![SourceSpec {
            path: path.to_path_buf(),
            source,
        }]);
        assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    }

    fn append(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.flush().unwrap();
    }

    fn read(tailer: &mut LogTailer, path: &Path) -> ReadOutcome {
        tailer.read_path(path, ReadBudget::default())
    }

    fn bodies(outcome: &ReadOutcome) -> Vec<&str> {
        outcome
            .lines
            .iter()
            .map(|line| line.body.as_ref())
            .collect()
    }

    #[test]
    fn initial_discovery_skips_history_then_reads_one_append() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, b"[old] historical\n").unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        assert!(read(&mut tailer, &path).lines.is_empty());
        append(&path, b"[now] current\n");
        assert_eq!(bodies(&read(&mut tailer, &path)), vec!["current"]);
    }

    #[test]
    fn reads_multiple_lines_in_order() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        append(&path, b"[one] first\n[two] second\n[three] third\n");
        assert_eq!(
            bodies(&read(&mut tailer, &path)),
            vec!["first", "second", "third"]
        );
    }

    #[test]
    fn split_line_waits_for_its_newline() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        append(&path, b"[now] part");
        assert!(read(&mut tailer, &path).lines.is_empty());
        append(&path, b"ial line\n");
        assert_eq!(bodies(&read(&mut tailer, &path)), vec!["partial line"]);
    }

    #[test]
    fn utf8_codepoint_can_be_split_across_writes() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        append(&path, b"[now] caf\xc3");
        assert!(read(&mut tailer, &path).lines.is_empty());
        append(&path, b"\xa9\n");
        let outcome = read(&mut tailer, &path);
        assert_eq!(bodies(&outcome), vec!["caf\u{e9}"]);
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn crlf_is_removed_without_leaving_a_carriage_return() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        append(&path, b"[one] first\r\n[two] second\r\n");
        assert_eq!(bodies(&read(&mut tailer, &path)), vec!["first", "second"]);
    }

    #[test]
    fn truncation_restarts_at_zero_and_resets_the_generation() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, b"[old] a deliberately long historical record\n").unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        fs::write(&path, b"[new] reset\n").unwrap();
        let outcome = read(&mut tailer, &path);
        assert_eq!(bodies(&outcome), vec!["reset"]);
        assert_eq!(
            outcome
                .generation_reset
                .as_ref()
                .map(|source| source.id.as_str()),
            Some("client-1")
        );
        assert!(outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::FileReset));
    }

    #[test]
    fn truncate_and_regrow_past_the_old_offset_is_detected_by_the_boundary_signature() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, b"[old] historical boundary\n").unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        fs::write(
            &path,
            b"[new] replacement begins at zero and is longer than history\n",
        )
        .unwrap();
        let outcome = read(&mut tailer, &path);
        assert_eq!(
            bodies(&outcome),
            vec!["replacement begins at zero and is longer than history"]
        );
        assert!(outcome.generation_reset.is_some());
    }

    #[test]
    fn recreation_is_read_as_a_new_generation() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, b"[old] historical generation\n").unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        fs::remove_file(&path).unwrap();
        fs::write(&path, b"[new] replacement generation\n").unwrap();
        let outcome = read(&mut tailer, &path);
        assert_eq!(bodies(&outcome), vec!["replacement generation"]);
        assert!(outcome.generation_reset.is_some());
    }

    #[test]
    fn duplicate_wakes_do_not_duplicate_records() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, []).unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        append(&path, b"[now] once\n");
        assert_eq!(bodies(&read(&mut tailer, &path)), vec!["once"]);
        assert!(read(&mut tailer, &path).lines.is_empty());
    }

    #[test]
    fn two_logs_keep_their_source_attribution() {
        let directory = TempDirectory::new();
        let bilka_path = directory.path("eqlog_Bilka_teek.txt");
        let saabra_path = directory.path("eqlog_Saabra_teek.txt");
        fs::write(&bilka_path, []).unwrap();
        fs::write(&saabra_path, []).unwrap();
        let mut tailer = LogTailer::new();
        let outcome = tailer.sync_sources(vec![
            SourceSpec {
                path: bilka_path.clone(),
                source: source(11, "Bilka", "teek"),
            },
            SourceSpec {
                path: saabra_path.clone(),
                source: source(22, "Saabra", "teek"),
            },
        ]);
        assert!(outcome.diagnostics.is_empty());

        append(&bilka_path, b"[now] bard line\n");
        append(&saabra_path, b"[now] mage line\n");
        let bilka = read(&mut tailer, &bilka_path);
        let saabra = read(&mut tailer, &saabra_path);
        assert_eq!(bilka.lines[0].source.id.as_str(), "client-11");
        assert_eq!(&*bilka.lines[0].source.character, "Bilka");
        assert_eq!(saabra.lines[0].source.id.as_str(), "client-22");
        assert_eq!(&*saabra.lines[0].source.character, "Saabra");
    }

    #[test]
    fn unfinished_history_is_discarded_through_its_next_newline() {
        let directory = TempDirectory::new();
        let path = directory.path("eqlog_Bilka_teek.txt");
        fs::write(&path, b"[old] unfinished history").unwrap();
        let mut tailer = LogTailer::new();
        track(&mut tailer, &path, source(1, "Bilka", "teek"));

        append(&path, b" suffix\n[now] first live record\n");
        assert_eq!(bodies(&read(&mut tailer, &path)), vec!["first live record"]);
    }
}
