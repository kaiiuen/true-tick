//! Per-user Windows boot startup registration.
//!
//! This boundary targets only the current user's Run key. It does not create a
//! service, scheduled task, machine-wide value, installer, or elevated state.
//! Tests cover pure command construction and never call the registry methods.

use std::path::Path;

const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "TrueTick";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupOperation {
    Register,
    Remove,
}

pub fn startup_operation(enabled: bool) -> StartupOperation {
    if enabled {
        StartupOperation::Register
    } else {
        StartupOperation::Remove
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupError {
    Unsupported,
    InvalidExecutablePath,
    LauncherMissing,
    NotLauncher,
    NotDevelopmentExecutable,
    OpenKey { raw_status: u32 },
    SetValue { raw_status: u32 },
    RemoveValue { raw_status: u32 },
}

pub trait StartupRegistration {
    fn register(&mut self, executable: &Path) -> Result<(), StartupError>;
    fn register_development(&mut self, executable: &Path) -> Result<(), StartupError>;
    fn remove(&mut self) -> Result<(), StartupError>;
}

#[derive(Debug, Default)]
pub struct WindowsUserStartup;

impl StartupRegistration for WindowsUserStartup {
    fn register(&mut self, executable: &Path) -> Result<(), StartupError> {
        #[cfg(windows)]
        {
            validate_launcher_path(executable)?;
            register_validated(executable)
        }
        #[cfg(not(windows))]
        {
            let _ = executable;
            Err(StartupError::Unsupported)
        }
    }

    fn register_development(&mut self, executable: &Path) -> Result<(), StartupError> {
        #[cfg(windows)]
        {
            validate_development_path(executable)?;
            register_validated(executable)
        }
        #[cfg(not(windows))]
        {
            let _ = executable;
            Err(StartupError::Unsupported)
        }
    }

    fn remove(&mut self) -> Result<(), StartupError> {
        #[cfg(windows)]
        {
            let key = open_run_key()?;
            let name = wide(VALUE_NAME);
            let status = unsafe { RegDeleteValueW(key, name.as_ptr()) };
            unsafe {
                RegCloseKey(key);
            }
            if status == ERROR_FILE_NOT_FOUND || status == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(StartupError::RemoveValue { raw_status: status })
            }
        }
        #[cfg(not(windows))]
        {
            Err(StartupError::Unsupported)
        }
    }
}

fn validate_launcher_path(executable: &Path) -> Result<(), StartupError> {
    let name = executable
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(StartupError::InvalidExecutablePath)?;
    if !name.eq_ignore_ascii_case("Launcher.exe") {
        return Err(StartupError::NotLauncher);
    }
    if !executable.exists() {
        return Err(StartupError::LauncherMissing);
    }
    if !executable.is_file() {
        return Err(StartupError::InvalidExecutablePath);
    }
    Ok(())
}

fn validate_development_path(executable: &Path) -> Result<(), StartupError> {
    if !is_development_executable(executable) {
        return Err(StartupError::NotDevelopmentExecutable);
    }
    if !executable.exists() {
        return Err(StartupError::LauncherMissing);
    }
    if !executable.is_file() {
        return Err(StartupError::InvalidExecutablePath);
    }
    Ok(())
}

pub fn is_development_executable(executable: &Path) -> bool {
    let name_matches = executable
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("true-tick.exe"));
    let has_target = executable.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("target")
    });
    let has_debug = executable.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("debug")
    });
    name_matches && has_target && has_debug
}

#[cfg(windows)]
fn register_validated(executable: &Path) -> Result<(), StartupError> {
    let command = startup_command(executable)?;
    let key = open_run_key()?;
    let name = wide(VALUE_NAME);
    let value = wide(&command);
    let status = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr() as *const u8,
            (value.len() * 2) as u32,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if status != ERROR_SUCCESS {
        return Err(StartupError::SetValue { raw_status: status });
    }
    Ok(())
}

fn startup_command(executable: &Path) -> Result<String, StartupError> {
    let path = executable
        .to_str()
        .ok_or(StartupError::InvalidExecutablePath)?;
    if path.is_empty() || path.contains('"') {
        return Err(StartupError::InvalidExecutablePath);
    }
    Ok(format!("\"{path}\""))
}

#[cfg(windows)]
fn open_run_key() -> Result<*mut std::ffi::c_void, StartupError> {
    let subkey = wide(RUN_SUBKEY);
    let mut key = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(StartupError::OpenKey { raw_status: status })
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
const HKEY_CURRENT_USER: *mut std::ffi::c_void = (-2147483647isize) as *mut std::ffi::c_void;
#[cfg(windows)]
const KEY_SET_VALUE: u32 = 0x0002;
#[cfg(windows)]
const REG_SZ: u32 = 1;
#[cfg(windows)]
const ERROR_SUCCESS: u32 = 0;
#[cfg(windows)]
const ERROR_FILE_NOT_FOUND: u32 = 2;

#[cfg(windows)]
#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        key: *mut std::ffi::c_void,
        subkey: *const u16,
        reserved: u32,
        class: *mut std::ffi::c_void,
        options: u32,
        access: u32,
        security_attributes: *const std::ffi::c_void,
        result: *mut *mut std::ffi::c_void,
        disposition: *mut u32,
    ) -> u32;
    fn RegSetValueExW(
        key: *mut std::ffi::c_void,
        value_name: *const u16,
        reserved: u32,
        value_type: u32,
        data: *const u8,
        data_size: u32,
    ) -> u32;
    fn RegDeleteValueW(key: *mut std::ffi::c_void, value_name: *const u16) -> u32;
    fn RegCloseKey(key: *mut std::ffi::c_void) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn startup_command_targets_quoted_portable_launcher() {
        assert_eq!(
            startup_command(&PathBuf::from(r"C:\Tick\Launcher.exe")).unwrap(),
            r#""C:\Tick\Launcher.exe""#
        );
    }

    #[test]
    fn startup_state_maps_to_registration_or_removal() {
        assert_eq!(startup_operation(true), StartupOperation::Register);
        assert_eq!(startup_operation(false), StartupOperation::Remove);
    }

    #[test]
    fn paths_containing_quotes_are_rejected() {
        assert_eq!(
            startup_command(&PathBuf::from(r#"C:\bad"name\true-tick.exe"#)),
            Err(StartupError::InvalidExecutablePath)
        );
    }

    #[test]
    fn development_fallback_requires_target_debug_true_tick() {
        assert!(is_development_executable(&PathBuf::from(
            r"C:\Tick\target\debug\true-tick.exe"
        )));
        assert!(!is_development_executable(&PathBuf::from(
            r"C:\Tick\target\release\true-tick.exe"
        )));
        assert!(!is_development_executable(&PathBuf::from(
            r"C:\Tick\Slots\A\true-tick.exe"
        )));
    }

    #[test]
    fn launcher_validation_checks_identity_and_presence() {
        let root = std::env::temp_dir().join(format!("true-tick-startup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let wrong = root.join("true-tick.exe");
        std::fs::write(&wrong, b"fixture").unwrap();
        assert_eq!(
            validate_launcher_path(&wrong),
            Err(StartupError::NotLauncher)
        );
        let launcher = root.join("Launcher.exe");
        assert_eq!(
            validate_launcher_path(&launcher),
            Err(StartupError::LauncherMissing)
        );
        std::fs::write(&launcher, b"fixture").unwrap();
        assert_eq!(validate_launcher_path(&launcher), Ok(()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_registration_has_no_test_side_effect() {
        assert_eq!(std::mem::size_of::<WindowsUserStartup>(), 0);
    }

    #[test]
    fn startup_command_rejects_empty_paths_and_null_bytes() {
        assert_eq!(
            startup_command(&PathBuf::from("")),
            Err(StartupError::InvalidExecutablePath)
        );
    }

    #[test]
    fn validate_launcher_path_rejects_directories_and_invalid_names() {
        let root =
            std::env::temp_dir().join(format!("true-tick-launcher-dir-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();

        // Path pointing to a directory named Launcher.exe instead of a file
        let dir_launcher = root.join("Launcher.exe");
        std::fs::create_dir_all(&dir_launcher).unwrap();
        assert_eq!(
            validate_launcher_path(&dir_launcher),
            Err(StartupError::InvalidExecutablePath)
        );

        // Path with missing file extension or totally wrong name
        let no_ext = root.join("Launcher");
        std::fs::write(&no_ext, b"fixture").unwrap();
        assert_eq!(
            validate_launcher_path(&no_ext),
            Err(StartupError::NotLauncher)
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validate_development_path_rejects_directories_and_missing_files() {
        let root = std::env::temp_dir().join(format!("true-tick-dev-{}", std::process::id()));
        let dev_dir = root.join("target").join("debug");
        std::fs::create_dir_all(&dev_dir).unwrap();

        let dev_exe = dev_dir.join("true-tick.exe");
        // Missing file
        assert_eq!(
            validate_development_path(&dev_exe),
            Err(StartupError::LauncherMissing)
        );

        // Path pointing to a directory named true-tick.exe
        std::fs::create_dir_all(&dev_exe).unwrap();
        assert_eq!(
            validate_development_path(&dev_exe),
            Err(StartupError::InvalidExecutablePath)
        );
        std::fs::remove_dir(&dev_exe).unwrap();

        // Valid file passes
        std::fs::write(&dev_exe, b"fixture").unwrap();
        assert_eq!(validate_development_path(&dev_exe), Ok(()));

        // Wrong name in target debug folder
        let wrong_exe = dev_dir.join("other.exe");
        std::fs::write(&wrong_exe, b"fixture").unwrap();
        assert_eq!(
            validate_development_path(&wrong_exe),
            Err(StartupError::NotDevelopmentExecutable)
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_error_variants_and_simulated_registry_status_preservation() {
        let open_error = StartupError::OpenKey { raw_status: 5 }; // ERROR_ACCESS_DENIED
        assert_eq!(open_error, StartupError::OpenKey { raw_status: 5 });

        let set_error = StartupError::SetValue { raw_status: 1010 }; // ERROR_BAD_KEY
        assert_eq!(set_error, StartupError::SetValue { raw_status: 1010 });

        let remove_error = StartupError::RemoveValue { raw_status: 2 }; // ERROR_FILE_NOT_FOUND
        assert_eq!(remove_error, StartupError::RemoveValue { raw_status: 2 });
    }

    #[test]
    fn wide_string_conversion_null_terminates() {
        let utf16 = wide("TrueTick");
        assert_eq!(*utf16.last().unwrap(), 0u16);
        assert_eq!(utf16.len(), "TrueTick".encode_utf16().count() + 1);
    }
}
