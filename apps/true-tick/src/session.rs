use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

pub(crate) const SESSION_MARKER_FILENAME: &str = "session.state";
pub(crate) const SESSION_STATE_RUNNING: &str = "running";
pub(crate) const SESSION_STATE_CLEAN: &str = "clean";

const TOMBSTONE_MAGIC: &[u8; 4] = b"TTSS";
const TOMBSTONE_VERSION: &[u8; 2] = b"v1";
const TOMBSTONE_CHECKSUM_LEN: usize = 32;
const TOMBSTONE_HEADER_LEN: usize = 4 + 2 + 32 + 4 + 8;
const MAX_ABORT_REASON_BYTES: usize = 160;

#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

/// Crash tombstone and session marker details written while a process is running.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionTombstone {
    pub(crate) pid: u32,
    pub(crate) startup_timestamp_secs: u64,
}

impl SessionTombstone {
    pub(crate) fn new(pid: u32, startup_timestamp_secs: u64) -> Self {
        Self {
            pid,
            startup_timestamp_secs,
        }
    }

    pub(crate) fn current() -> Result<Self, SessionError> {
        let pid = std::process::id();
        let startup_timestamp_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| SessionError::ClockError)?;
        Ok(Self::new(pid, startup_timestamp_secs))
    }

    #[allow(dead_code)]
    pub(crate) fn format_payload(&self) -> String {
        format!(
            "state={}\npid={}\nstartup_timestamp={}\n",
            SESSION_STATE_RUNNING, self.pid, self.startup_timestamp_secs
        )
    }

    fn checksum(&self) -> [u8; TOMBSTONE_CHECKSUM_LEN] {
        let mut hasher = Sha256::new();
        hasher.update(TOMBSTONE_MAGIC);
        hasher.update(TOMBSTONE_VERSION);
        hasher.update(self.pid.to_le_bytes());
        hasher.update(self.startup_timestamp_secs.to_le_bytes());
        let digest = hasher.finalize();
        let mut checksum = [0u8; TOMBSTONE_CHECKSUM_LEN];
        checksum.copy_from_slice(&digest);
        checksum
    }

    pub(crate) fn encode_bytes(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(TOMBSTONE_HEADER_LEN);
        output.extend_from_slice(TOMBSTONE_MAGIC);
        output.extend_from_slice(TOMBSTONE_VERSION);
        output.extend_from_slice(&self.checksum());
        output.extend_from_slice(&self.pid.to_le_bytes());
        output.extend_from_slice(&self.startup_timestamp_secs.to_le_bytes());
        output
    }

    pub(crate) fn decode_bytes(bytes: &[u8]) -> Result<Self, SessionError> {
        if bytes.len() < TOMBSTONE_HEADER_LEN {
            return Err(SessionError::Corrupted(String::from(
                "tombstone is truncated",
            )));
        }
        if &bytes[0..4] != TOMBSTONE_MAGIC {
            return Err(SessionError::Corrupted(String::from(
                "tombstone magic mismatch",
            )));
        }
        if &bytes[4..6] != TOMBSTONE_VERSION {
            return Err(SessionError::Corrupted(String::from(
                "tombstone version mismatch",
            )));
        }
        let mut stored = [0u8; TOMBSTONE_CHECKSUM_LEN];
        stored.copy_from_slice(&bytes[6..38]);
        let pid = u32::from_le_bytes([bytes[38], bytes[39], bytes[40], bytes[41]]);
        let timestamp = u64::from_le_bytes([
            bytes[42], bytes[43], bytes[44], bytes[45], bytes[46], bytes[47], bytes[48], bytes[49],
        ]);
        let candidate = Self::new(pid, timestamp);
        if candidate.checksum() != stored {
            return Err(SessionError::Corrupted(String::from(
                "tombstone checksum mismatch",
            )));
        }
        Ok(candidate)
    }
}

/// Failures that can occur while reading or interpreting the session marker.
#[derive(Debug)]
pub(crate) enum SessionError {
    Unreadable(String),
    Corrupted(String),
    ClockError,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(detail) => {
                write!(formatter, "session marker unreadable {detail}")
            }
            Self::Corrupted(detail) => {
                write!(formatter, "session marker corrupted {detail}")
            }
            Self::ClockError => formatter.write_str("system clock predates the epoch"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Raw session marker bytes read from disk with a parsed tombstone view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionMarker {
    bytes: Vec<u8>,
}

impl SessionMarker {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn tombstone(&self) -> Result<SessionTombstone, SessionError> {
        SessionTombstone::decode_bytes(&self.bytes)
    }

    pub(crate) fn describe(&self) -> String {
        match self.tombstone() {
            Ok(tombstone) => format!(
                "tombstone pid={} startup_timestamp={}",
                tombstone.pid, tombstone.startup_timestamp_secs
            ),
            Err(_) => match std::str::from_utf8(&self.bytes) {
                Ok(text) => {
                    let trimmed = text.trim();
                    let head = tick_diagnostics::truncate_utf8(trimmed, 64);
                    format!("legacy text {head}")
                }
                Err(_) => format!("binary {} bytes", self.bytes.len()),
            },
        }
    }
}

/// How the previous session ended, derived from the marker file left behind
/// in the state directory. The marker lives inside the app directory so the
/// self contained layout rule is preserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviousSession {
    /// No marker file was present. Either this is the first launch or the
    /// previous environment never recorded a session.
    FirstRun,
    /// The marker recorded a clean value, so the previous session exited through a
    /// deliberate shutdown path. Only legacy text markers can report this state.
    CleanExit,
    /// The marker recorded a running tombstone or any legacy unclean value, so the
    /// previous session never reached a clean shutdown path.
    UncleanShutdown,
    /// The marker file was present but failed magic or version or checksum checks.
    Corrupted,
    /// The marker file was present but could not be read because of access or
    /// encoding failures.
    Unreadable,
}

/// Pure classification of legacy text marker content. Any content other than
/// `clean` after trimming means the previous session did not record a
/// deliberate exit and is treated as unclean.
#[allow(dead_code)]
pub(crate) fn classify_previous_session(
    marker_content: Option<&str>,
) -> Result<PreviousSession, SessionError> {
    match marker_content.map(str::trim) {
        None => Ok(PreviousSession::FirstRun),
        Some(SESSION_STATE_CLEAN) => Ok(PreviousSession::CleanExit),
        Some(_) => Ok(PreviousSession::UncleanShutdown),
    }
}

/// Pure classification of raw marker bytes. A missing value means first run.
/// A valid binary tombstone means unclean shutdown. Legacy `clean` text means
/// clean exit. Anything else means corrupted or unclean.
pub(crate) fn classify_previous_session_bytes(bytes: Option<&[u8]>) -> PreviousSession {
    let bytes = match bytes {
        None => return PreviousSession::FirstRun,
        Some(bytes) => bytes,
    };
    if SessionTombstone::decode_bytes(bytes).is_ok() {
        return PreviousSession::UncleanShutdown;
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        if text.trim() == SESSION_STATE_CLEAN {
            return PreviousSession::CleanExit;
        }
        if is_legacy_unclean_text(text) {
            return PreviousSession::UncleanShutdown;
        }
    }
    PreviousSession::Corrupted
}

fn is_legacy_unclean_text(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == SESSION_STATE_RUNNING
        || trimmed.starts_with("state=running")
        || trimmed.starts_with("state=aborted")
}

fn classify_marker(marker: Option<&SessionMarker>) -> PreviousSession {
    match marker {
        None => PreviousSession::FirstRun,
        Some(marker) => classify_previous_session_bytes(Some(marker.bytes())),
    }
}

/// Maps a session read outcome to a previous session decision. Unreadable file
/// errors become the `Unreadable` classification so callers never lose the
/// distinction between a missing file and a file that could not be read.
pub(crate) fn classify_session_read(
    result: Result<Option<SessionMarker>, SessionError>,
) -> PreviousSession {
    match result {
        Ok(marker) => classify_marker(marker.as_ref()),
        Err(SessionError::Unreadable(_)) => PreviousSession::Unreadable,
        Err(SessionError::Corrupted(_)) => PreviousSession::Corrupted,
        Err(SessionError::ClockError) => PreviousSession::Corrupted,
    }
}

pub(crate) fn session_marker_path(state_directory: &Path) -> PathBuf {
    state_directory.join(SESSION_MARKER_FILENAME)
}

fn io_error_is_missing(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

fn session_error_for_read(path: &Path, error: std::io::Error) -> SessionError {
    if io_error_is_missing(&error) {
        return SessionError::Corrupted(String::from("missing path reported as read error"));
    }
    SessionError::Unreadable(format!("path={} error={error}", path.display()))
}

/// Reads raw marker bytes from the state directory. A missing file reports
/// `Ok(None)` for first run. Read or encoding failures report `Unreadable`.
pub(crate) fn read_session_marker_bytes(
    state_directory: &Path,
) -> Result<Option<Vec<u8>>, SessionError> {
    let path = session_marker_path(state_directory);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if io_error_is_missing(&error) => Ok(None),
        Err(error) => Err(session_error_for_read(&path, error)),
    }
}

/// Reads and parses the session marker from the state directory. A missing
/// file reports `Ok(None)` for first run. A present file reports the parsed
/// marker bytes. Access failures report `Unreadable`.
pub(crate) fn read_session_marker(
    state_directory: &Path,
) -> Result<Option<SessionMarker>, SessionError> {
    match read_session_marker_bytes(state_directory)? {
        None => Ok(None),
        Some(bytes) => Ok(Some(SessionMarker::new(bytes))),
    }
}

#[cfg(test)]
pub(crate) fn write_session_marker(state_directory: &Path, state: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(state_directory)?;
    std::fs::write(session_marker_path(state_directory), state)
}

fn temporary_tombstone_path(state_directory: &Path) -> PathBuf {
    state_directory.join(format!(
        "{}.tmp.{}",
        SESSION_MARKER_FILENAME,
        std::process::id()
    ))
}

fn atomic_replace_file(temporary: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        let ok = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(temporary, target)
    }
}

pub(crate) fn write_session_bytes_atomic(
    state_directory: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    std::fs::create_dir_all(state_directory)?;
    let target = session_marker_path(state_directory);
    let temporary = temporary_tombstone_path(state_directory);
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        atomic_replace_file(&temporary, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Writes a binary session tombstone with magic plus version plus checksum
/// plus pid plus timestamp through an atomic temporary file replacement.
pub(crate) fn write_session_tombstone(
    state_directory: &Path,
    tombstone: &SessionTombstone,
) -> std::io::Result<()> {
    write_session_bytes_atomic(state_directory, &tombstone.encode_bytes())
}

/// Writes a session tombstone containing the current process ID and startup timestamp.
pub(crate) fn create_session_tombstone(
    state_directory: &Path,
    tombstone: &SessionTombstone,
) -> std::io::Result<()> {
    write_session_tombstone(state_directory, tombstone)
}

/// Removes the session marker and tombstone file from disk if it exists.
pub(crate) fn remove_session_marker(state_directory: &Path) -> std::io::Result<()> {
    let path = session_marker_path(state_directory);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sanitize_abort_reason(reason: &str) -> String {
    let cleaned = reason.replace(['\n', '\r'], " ");
    tick_diagnostics::truncate_utf8(&cleaned, MAX_ABORT_REASON_BYTES)
}

/// Writes an explicit aborted startup marker in binary form so the next launch
/// classifies the session as unclean and runs the settle probe recovery path
/// instead of a clean start. The reason text is truncated on a UTF-8 boundary
/// and appended after the verified tombstone header.
pub(crate) fn write_aborted_session_marker(
    state_directory: &Path,
    reason: &str,
) -> std::io::Result<()> {
    let tombstone = SessionTombstone::current().map_err(std::io::Error::other)?;
    let mut bytes = tombstone.encode_bytes();
    let reason = sanitize_abort_reason(reason);
    bytes.extend_from_slice(reason.as_bytes());
    write_session_bytes_atomic(state_directory, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(label: &str) -> PathBuf {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn missing_marker_is_first_run() {
        assert_eq!(
            classify_previous_session(None).expect("classification succeeds"),
            PreviousSession::FirstRun
        );
        assert_eq!(
            classify_previous_session_bytes(None),
            PreviousSession::FirstRun
        );
    }

    #[test]
    fn clean_marker_is_clean_exit() {
        assert_eq!(
            classify_previous_session(Some("clean")).expect("classification succeeds"),
            PreviousSession::CleanExit
        );
        assert_eq!(
            classify_previous_session_bytes(Some(b"clean")),
            PreviousSession::CleanExit
        );
    }

    #[test]
    fn running_marker_is_unclean_shutdown() {
        assert_eq!(
            classify_previous_session(Some("running")).expect("classification succeeds"),
            PreviousSession::UncleanShutdown
        );
        assert_eq!(
            classify_previous_session_bytes(Some(b"running")),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn marker_content_is_trimmed_before_classification() {
        assert_eq!(
            classify_previous_session(Some("  clean\n")).expect("classification succeeds"),
            PreviousSession::CleanExit
        );
        assert_eq!(
            classify_previous_session(Some("\trunning \n")).expect("classification succeeds"),
            PreviousSession::UncleanShutdown
        );
        assert_eq!(
            classify_previous_session(Some("   ")).expect("classification succeeds"),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn unrecognized_marker_is_unclean_shutdown() {
        assert_eq!(
            classify_previous_session(Some("garbage")).expect("classification succeeds"),
            PreviousSession::UncleanShutdown
        );
        assert_eq!(
            classify_previous_session(Some("")).expect("classification succeeds"),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn binary_tombstone_round_trip_verifies_checksum() {
        let tombstone = SessionTombstone::new(1234, 1_700_000_000);
        let bytes = tombstone.encode_bytes();
        assert_eq!(bytes.len(), TOMBSTONE_HEADER_LEN);
        assert_eq!(&bytes[0..4], TOMBSTONE_MAGIC);
        assert_eq!(&bytes[4..6], TOMBSTONE_VERSION);
        let decoded = SessionTombstone::decode_bytes(&bytes).expect("valid tombstone decodes");
        assert_eq!(decoded, tombstone);
        assert_eq!(
            classify_previous_session_bytes(Some(&bytes)),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn binary_tombstone_rejects_bad_magic_version_and_checksum() {
        let tombstone = SessionTombstone::new(42, 1_700_000_001);
        let mut bytes = tombstone.encode_bytes();
        bytes[0] = b'X';
        assert_eq!(
            classify_previous_session_bytes(Some(&bytes)),
            PreviousSession::Corrupted
        );
        let mut bytes = tombstone.encode_bytes();
        bytes[4] = b'x';
        assert_eq!(
            classify_previous_session_bytes(Some(&bytes)),
            PreviousSession::Corrupted
        );
        let mut bytes = tombstone.encode_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert_eq!(
            classify_previous_session_bytes(Some(&bytes)),
            PreviousSession::Corrupted
        );
        assert_eq!(
            classify_previous_session_bytes(Some(b"\x00\x01\x02")),
            PreviousSession::Corrupted
        );
    }

    #[test]
    fn tombstone_current_reports_clock_value() {
        let tombstone = SessionTombstone::current().expect("system clock is usable");
        assert_eq!(tombstone.pid, std::process::id());
        assert!(tombstone.startup_timestamp_secs > 0);
    }

    #[test]
    fn marker_round_trip_inside_state_directory() {
        let directory = test_directory("session-marker");
        assert_eq!(
            read_session_marker(&directory).expect("read succeeds"),
            None
        );
        write_session_marker(&directory, SESSION_STATE_RUNNING).expect("running marker writes");
        let marker = read_session_marker(&directory)
            .expect("read succeeds")
            .expect("marker present");
        assert_eq!(
            classify_previous_session_bytes(Some(marker.bytes())),
            PreviousSession::UncleanShutdown
        );
        write_session_marker(&directory, SESSION_STATE_CLEAN).expect("clean marker writes");
        let marker = read_session_marker(&directory)
            .expect("read succeeds")
            .expect("marker present");
        assert_eq!(
            classify_previous_session_bytes(Some(marker.bytes())),
            PreviousSession::CleanExit
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn tombstone_creation_detection_and_removal() {
        let directory = test_directory("session-tombstone");

        assert_eq!(
            read_session_marker(&directory).expect("read succeeds"),
            None
        );
        assert!(!session_marker_path(&directory).exists());

        let tombstone = SessionTombstone::new(1234, 1_700_000_000);
        create_session_tombstone(&directory, &tombstone).expect("tombstone writes");
        write_session_tombstone(&directory, &tombstone).expect("binary tombstone writes");

        assert!(session_marker_path(&directory).exists());
        let marker = read_session_marker(&directory)
            .expect("read succeeds")
            .expect("tombstone present");
        let decoded = marker.tombstone().expect("binary tombstone parses");
        assert_eq!(decoded.pid, 1234);
        assert_eq!(decoded.startup_timestamp_secs, 1_700_000_000);
        assert_eq!(
            classify_previous_session_bytes(Some(marker.bytes())),
            PreviousSession::UncleanShutdown
        );
        assert!(marker.describe().contains("pid=1234"));

        remove_session_marker(&directory).expect("tombstone removal succeeds");
        assert!(!session_marker_path(&directory).exists());
        assert_eq!(
            read_session_marker(&directory).expect("read succeeds"),
            None
        );
        assert_eq!(
            classify_previous_session(None).expect("classification succeeds"),
            PreviousSession::FirstRun
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn aborted_marker_uses_binary_form_and_safe_truncation() {
        let directory = test_directory("session-aborted");
        let reason = String::from("стыковка ") + &"x".repeat(400);
        write_aborted_session_marker(&directory, &reason).expect("aborted marker writes");
        let marker = read_session_marker(&directory)
            .expect("read succeeds")
            .expect("marker present");
        assert_eq!(
            classify_previous_session_bytes(Some(marker.bytes())),
            PreviousSession::UncleanShutdown
        );
        assert!(marker.bytes().len() >= TOMBSTONE_HEADER_LEN);
        assert!(marker.bytes().len() <= TOMBSTONE_HEADER_LEN + MAX_ABORT_REASON_BYTES + 8);
        let decoded = marker.tombstone().expect("aborted header parses");
        assert_eq!(decoded.pid, std::process::id());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn unreadable_classification_is_distinct_from_first_run() {
        assert_eq!(
            classify_session_read(Err(SessionError::Unreadable(String::from("denied")))),
            PreviousSession::Unreadable
        );
        assert_eq!(
            classify_session_read(Err(SessionError::Corrupted(String::from("bad")))),
            PreviousSession::Corrupted
        );
        assert_eq!(classify_session_read(Ok(None)), PreviousSession::FirstRun);
    }
}
