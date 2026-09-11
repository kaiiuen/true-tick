use std::fs;
use std::path::{Path, PathBuf};
use tick_core::Hns;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub automatic: bool,
    pub startup_enabled: bool,
    pub request_interval: Hns,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            automatic: false,
            startup_enabled: true,
            request_interval: Hns::new(10_000),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Write(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "configuration read error: {error}"),
            Self::Write(error) => write!(formatter, "configuration write error: {error}"),
            Self::Invalid(reason) => write!(formatter, "invalid configuration: {reason}"),
        }
    }
}

impl std::error::Error for ConfigError {}

pub fn path_from_executable(executable: &Path) -> PathBuf {
    executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("true-tick.toml")
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(path).map_err(ConfigError::Read)?;
    parse(&text)
}

pub fn save_atomic(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let temporary = path.with_extension("toml.tmp");
    let text = format!(
        "automatic = {}\nstartup_enabled = {}\nrequest_interval_hns = {}\n",
        config.automatic,
        config.startup_enabled,
        config.request_interval.value()
    );
    let result = (|| {
        let mut file = fs::File::create(&temporary).map_err(ConfigError::Write)?;
        use std::io::Write;
        file.write_all(text.as_bytes())
            .map_err(ConfigError::Write)?;
        file.sync_all().map_err(ConfigError::Write)?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), ConfigError> {
    fs::rename(temporary, path).map_err(ConfigError::Write)
}

#[cfg(windows)]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), ConfigError> {
    use std::os::windows::ffi::OsStrExt;
    let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(ConfigError::Write(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

#[cfg(windows)]
extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let mut config = Config::default();
    for (line_number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            ConfigError::Invalid(format!("line {} needs key = value", line_number + 1))
        })?;
        match key.trim() {
            "automatic" => {
                config.automatic = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(ConfigError::Invalid(format!(
                            "invalid automatic value {other}"
                        )))
                    }
                }
            }
            "startup_enabled" => {
                config.startup_enabled = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(ConfigError::Invalid(format!(
                            "invalid startup_enabled value {other}"
                        )))
                    }
                }
            }
            "request_interval_hns" => {
                let value = value.trim().parse::<u64>().map_err(|_| {
                    ConfigError::Invalid(format!(
                        "invalid request interval on line {}",
                        line_number + 1
                    ))
                })?;
                config.request_interval = Hns::new(value);
            }
            other => {
                return Err(ConfigError::Invalid(format!(
                    "unknown configuration key {other}"
                )))
            }
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_startup_without_automatic_timing() {
        assert_eq!(
            Config::default(),
            Config {
                automatic: false,
                startup_enabled: true,
                request_interval: Hns::new(10_000)
            }
        );
    }

    #[test]
    fn parses_explicit_local_configuration() {
        let config =
            parse("automatic = true\nstartup_enabled = true\nrequest_interval_hns = 10000\n")
                .unwrap();
        assert_eq!(
            config,
            Config {
                automatic: true,
                startup_enabled: true,
                request_interval: Hns::new(10_000)
            }
        );
    }

    #[test]
    fn preserves_explicit_false_values_during_migration() {
        let config = parse("automatic = false\nstartup_enabled = false\n").unwrap();
        assert!(!config.automatic);
        assert!(!config.startup_enabled);
        assert_eq!(config.request_interval, Hns::new(10_000));
    }

    #[test]
    fn rejects_unknown_configuration() {
        assert!(parse("profile = game").is_err());
    }

    #[test]
    fn serializes_configuration_for_persistence() {
        let root = std::env::temp_dir().join(format!("true-tick-config-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        let config = Config {
            automatic: true,
            startup_enabled: true,
            request_interval: Hns::new(10_000),
        };
        save_atomic(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        fs::remove_dir_all(root).unwrap();
    }
}
