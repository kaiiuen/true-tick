use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const SESSION_MARKER_FILENAME: &str = "session.state";
pub(crate) const SESSION_STATE_RUNNING: &str = "running";
pub(crate) const SESSION_STATE_CLEAN: &str = "clean";

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

    pub(crate) fn current() -> Self {
        let pid = std::process::id();
        let startup_timestamp_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        Self::new(pid, startup_timestamp_secs)
    }

    pub(crate) fn format_payload(&self) -> String {
        format!(
            "state={}\npid={}\nstartup_timestamp={}\n",
            SESSION_STATE_RUNNING, self.pid, self.startup_timestamp_secs
        )
    }
}

/// How the previous session ended, derived from the marker file left behind
/// in the log directory. The marker lives inside the app directory so the
/// self contained layout rule is preserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviousSession {
    /// No marker file was present. Either this is the first launch or the
    /// previous environment never recorded a session.
    FirstRun,
    /// The marker recorded `clean`, so the previous session exited through a
    /// deliberate shutdown path.
    CleanExit,
    /// The marker recorded `running` or any unrecognized content, so the
    /// previous session never reached a clean shutdown path.
    UncleanShutdown,
}

/// Pure classification of the marker file content. Any content other than
/// `clean` (after trimming) means the previous session did not record a
/// deliberate exit and is treated as unclean.
pub(crate) fn classify_previous_session(marker_content: Option<&str>) -> PreviousSession {
    match marker_content.map(str::trim) {
        None => PreviousSession::FirstRun,
        Some(SESSION_STATE_CLEAN) => PreviousSession::CleanExit,
        Some(_) => PreviousSession::UncleanShutdown,
    }
}

pub(crate) fn session_marker_path(log_directory: &Path) -> PathBuf {
    log_directory.join(SESSION_MARKER_FILENAME)
}

pub(crate) fn read_session_marker(log_directory: &Path) -> Option<String> {
    std::fs::read_to_string(session_marker_path(log_directory)).ok()
}

#[cfg(test)]
pub(crate) fn write_session_marker(log_directory: &Path, state: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(log_directory)?;
    std::fs::write(session_marker_path(log_directory), state)
}

/// Writes a session tombstone containing the current process ID and startup timestamp.
pub(crate) fn create_session_tombstone(
    log_directory: &Path,
    tombstone: &SessionTombstone,
) -> std::io::Result<()> {
    std::fs::create_dir_all(log_directory)?;
    std::fs::write(
        session_marker_path(log_directory),
        tombstone.format_payload(),
    )
}

/// Removes the session marker / tombstone file from disk if it exists.
pub(crate) fn remove_session_marker(log_directory: &Path) -> std::io::Result<()> {
    let path = session_marker_path(log_directory);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_marker_is_first_run() {
        assert_eq!(classify_previous_session(None), PreviousSession::FirstRun);
    }

    #[test]
    fn clean_marker_is_clean_exit() {
        assert_eq!(
            classify_previous_session(Some("clean")),
            PreviousSession::CleanExit
        );
    }

    #[test]
    fn running_marker_is_unclean_shutdown() {
        assert_eq!(
            classify_previous_session(Some("running")),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn marker_content_is_trimmed_before_classification() {
        assert_eq!(
            classify_previous_session(Some("  clean\n")),
            PreviousSession::CleanExit
        );
        assert_eq!(
            classify_previous_session(Some("\trunning \n")),
            PreviousSession::UncleanShutdown
        );
        assert_eq!(
            classify_previous_session(Some("   ")),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn unrecognized_marker_is_unclean_shutdown() {
        assert_eq!(
            classify_previous_session(Some("garbage")),
            PreviousSession::UncleanShutdown
        );
        assert_eq!(
            classify_previous_session(Some("")),
            PreviousSession::UncleanShutdown
        );
    }

    #[test]
    fn marker_round_trip_inside_log_directory() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("session-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        assert_eq!(read_session_marker(&directory), None);
        write_session_marker(&directory, SESSION_STATE_RUNNING).expect("running marker writes");
        assert_eq!(
            classify_previous_session(read_session_marker(&directory).as_deref()),
            PreviousSession::UncleanShutdown
        );
        write_session_marker(&directory, SESSION_STATE_CLEAN).expect("clean marker writes");
        assert_eq!(
            classify_previous_session(read_session_marker(&directory).as_deref()),
            PreviousSession::CleanExit
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn tombstone_creation_detection_and_removal() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("session-tombstone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        assert_eq!(read_session_marker(&directory), None);
        assert!(!session_marker_path(&directory).exists());

        let tombstone = SessionTombstone::new(1234, 1700000000);
        create_session_tombstone(&directory, &tombstone).expect("tombstone writes");

        assert!(session_marker_path(&directory).exists());
        let content = read_session_marker(&directory).expect("tombstone reads");
        assert!(content.contains("pid=1234"));
        assert!(content.contains("startup_timestamp=1700000000"));
        assert_eq!(
            classify_previous_session(Some(&content)),
            PreviousSession::UncleanShutdown
        );

        remove_session_marker(&directory).expect("tombstone removal succeeds");
        assert!(!session_marker_path(&directory).exists());
        assert_eq!(read_session_marker(&directory), None);
        assert_eq!(classify_previous_session(None), PreviousSession::FirstRun);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
