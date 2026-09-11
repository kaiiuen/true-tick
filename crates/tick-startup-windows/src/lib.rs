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
    OpenKey { raw_status: u32 },
    SetValue { raw_status: u32 },
    RemoveValue { raw_status: u32 },
}

pub trait StartupRegistration {
    fn register(&mut self, executable: &Path) -> Result<(), StartupError>;
    fn remove(&mut self) -> Result<(), StartupError>;
}

#[derive(Debug, Default)]
pub struct WindowsUserStartup;

impl StartupRegistration for WindowsUserStartup {
    fn register(&mut self, executable: &Path) -> Result<(), StartupError> {
        #[cfg(windows)]
        {
            validate_launcher_path(executable)?;
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
}
