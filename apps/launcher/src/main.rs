#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Slot {
    A,
    B,
}

impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

#[derive(Debug)]
enum SelectionError {
    MetadataRead(std::io::Error),
    InvalidMetadata(String),
    ExecutableMissing(PathBuf),
    ExecutableNotFile(PathBuf),
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetadataRead(error) => {
                write!(formatter, "active slot metadata cannot be read: {error}")
            }
            Self::InvalidMetadata(value) => {
                write!(formatter, "active slot metadata is invalid: {value}")
            }
            Self::ExecutableMissing(path) => write!(
                formatter,
                "selected slot executable is missing: {}",
                path.display()
            ),
            Self::ExecutableNotFile(path) => write!(
                formatter,
                "selected slot executable is not a file: {}",
                path.display()
            ),
        }
    }
}

fn select(root: &Path) -> Result<(Slot, PathBuf), SelectionError> {
    let metadata = root.join("active-slot.txt");
    let value = fs::read_to_string(&metadata).map_err(SelectionError::MetadataRead)?;
    let value = value.trim();
    let slot = match value {
        "A" => Slot::A,
        "B" => Slot::B,
        other => return Err(SelectionError::InvalidMetadata(other.to_owned())),
    };
    let executable = root.join("Slots").join(slot.name()).join("true-tick.exe");
    if !executable.exists() {
        return Err(SelectionError::ExecutableMissing(executable));
    }
    if !executable.is_file() {
        return Err(SelectionError::ExecutableNotFile(executable));
    }
    Ok((slot, executable))
}

fn workspace_root(launcher: &Path) -> Option<&Path> {
    launcher.parent()
}

fn repair_reason(root: &Path, error: &str) -> String {
    format!(
        "True\u{2122} Tick launcher repair required: {error}\n\nPackage root: {}",
        root.display()
    )
}

fn run() -> Result<i32, String> {
    let launcher = std::env::current_exe().map_err(|error| error.to_string())?;
    let root = workspace_root(&launcher)
        .ok_or_else(|| "Launcher.exe has no workspace root".to_owned())?
        .to_owned();
    let (slot, executable) =
        select(&root).map_err(|error| repair_reason(&root, &error.to_string()))?;
    Command::new(&executable)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|error| {
            repair_reason(
                &root,
                &format!("selected slot {slot:?} could not be launched: {error}"),
            )
        })?;
    Ok(0)
}

#[cfg(windows)]
mod dialog {
    use std::ffi::c_void;

    const MB_OK: u32 = 0x0000_0000;
    const MB_ICONERROR: u32 = 0x0000_0010;
    const MB_SETFOREGROUND: u32 = 0x0001_0000;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, title: *const u16, kind: u32) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(crate) fn show_error(reason: &str) {
        let text = wide(reason);
        let title = wide("True Tick launcher");
        // SAFETY: MessageBoxW accepts a null owner and valid null terminated
        // UTF-16 buffers that outlive the call
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("{error}");
            #[cfg(windows)]
            dialog::show_error(&error);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("true-tick-launcher-{name}-{}", std::process::id()))
    }

    #[test]
    fn repair_reason_includes_the_package_root_and_cause() {
        let path = Path::new("C:\\packages\\true-tick");
        let reason = repair_reason(path, "active slot metadata is invalid: C");
        assert!(reason.contains("repair required: active slot metadata is invalid: C"));
        assert!(reason.contains("Package root: C:\\packages\\true-tick"));
    }

    #[test]
    fn missing_metadata_is_an_error() {
        let path = root("missing-metadata");
        fs::create_dir_all(&path).unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::MetadataRead(_))
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn invalid_metadata_is_an_error() {
        let path = root("invalid-metadata");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("active-slot.txt"), "C\n").unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::InvalidMetadata(_))
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn selected_slot_must_contain_the_expected_executable() {
        let path = root("missing-executable");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::ExecutableMissing(_))
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn valid_metadata_selects_only_the_active_slot() {
        let path = root("valid");
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::write(path.join("active-slot.txt"), "B\n").unwrap();
        fs::write(
            path.join("Slots").join("B").join("true-tick.exe"),
            b"fixture",
        )
        .unwrap();
        assert_eq!(select(&path).unwrap().0, Slot::B);
        fs::remove_dir_all(path).unwrap();
    }
}
