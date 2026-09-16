use std::path::{Path, PathBuf};

pub(crate) const SESSION_MARKER_FILENAME: &str = "session.state";
pub(crate) const SESSION_STATE_RUNNING: &str = "running";
pub(crate) const SESSION_STATE_CLEAN: &str = "clean";

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

pub(crate) fn write_session_marker(log_directory: &Path, state: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(log_directory)?;
    std::fs::write(session_marker_path(log_directory), state)
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
}
