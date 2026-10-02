use std::path::Path;

/// What the Settings Reset command removed from disk. Each field stays a plain
/// count or flag so the diagnostic chain can serialize the report directly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResetCleanup {
    pub(crate) log_entries_removed: usize,
    pub(crate) log_entries_failed: usize,
    pub(crate) config_removed: bool,
    pub(crate) session_marker_removed: bool,
}

/// Removes a single file when it exists. Missing files are not an error because
/// reset is expected to run against partially populated directories.
pub(crate) fn remove_file_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Deletes every entry inside `directory` without removing the directory
/// itself. Returns the number of removed entries plus the number of entries
/// that failed to delete so partial wipes remain visible to the caller.
pub(crate) fn clear_directory_contents(directory: &Path) -> (usize, usize) {
    let mut removed = 0usize;
    let mut failed = 0usize;
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return (0, 0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_directory = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let outcome = if is_directory {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match outcome {
            Ok(()) => removed += 1,
            Err(_) => failed += 1,
        }
    }
    (removed, failed)
}

/// Wipes the log directory contents, the config file, and the session marker.
/// The session marker lives in the state directory next to the logs, and it
/// is removed explicitly so the report reflects the tombstone deletion even
/// when the log directory was already empty or missing.
pub(crate) fn reset_cleanup(
    log_directory: &Path,
    state_directory: &Path,
    config_path: &Path,
) -> ResetCleanup {
    let session_marker = crate::session::session_marker_path(state_directory);
    let session_marker_removed = remove_file_if_present(&session_marker).unwrap_or(false);
    let (log_entries_removed, log_entries_failed) = clear_directory_contents(log_directory);
    let config_removed = remove_file_if_present(config_path).unwrap_or(false);
    ResetCleanup {
        log_entries_removed,
        log_entries_failed,
        config_removed,
        session_marker_removed,
    }
}
