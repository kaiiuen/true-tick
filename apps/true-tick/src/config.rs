use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tick_core::Hns;

pub const MAX_CONFIG_FILE_BYTES: usize = 64 * 1024;
const MAX_CONFIG_LINE_BYTES: usize = 4 * 1024;
const MAX_CONFIG_KEY_BYTES: usize = 64;
const MAX_CONFIG_VALUE_BYTES: usize = 1024;
const MAX_CONFIG_ERROR_BYTES: usize = 256;

/// Zero is the persisted sentinel for selecting the current native boundary.
pub const AUTOMATIC_REQUEST_INTERVAL: Hns = Hns::ZERO;
/// Legacy one-millisecond config value accepted only during migration.
const LEGACY_ONE_MILLISECOND_REQUEST_INTERVAL: Hns = Hns::new(10_000);

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
            request_interval: AUTOMATIC_REQUEST_INTERVAL,
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
        let message = match self {
            Self::Read(error) => format!("configuration read error: {error}"),
            Self::Write(error) => format!("configuration write error: {error}"),
            Self::Invalid(reason) => format!("invalid configuration: {reason}"),
        };
        formatter.write_str(&tick_diagnostics::truncate_utf8(
            &message,
            MAX_CONFIG_ERROR_BYTES,
        ))
    }
}

impl std::error::Error for ConfigError {}

pub fn path_from_executable(executable: &Path) -> PathBuf {
    executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("true-tick.toml")
}

#[cfg(test)]
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    load_with_migration(path).map(|(config, _)| config)
}

pub fn load_with_migration(path: &Path) -> Result<(Config, bool), ConfigError> {
    let metadata = fs::metadata(path).map_err(ConfigError::Read)?;
    if metadata.len() > MAX_CONFIG_FILE_BYTES as u64 {
        return Err(invalid_reason("configuration file exceeds maximum size"));
    }
    let bytes = fs::read(path).map_err(ConfigError::Read)?;
    if bytes.len() > MAX_CONFIG_FILE_BYTES {
        return Err(invalid_reason("configuration file exceeds maximum size"));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| invalid_reason("configuration is not valid UTF-8"))?;
    let (config, migrated) = parse_with_migration(&text)?;
    if migrated {
        save_atomic(path, &config)?;
    }
    Ok((config, migrated))
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
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

#[cfg(test)]
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    parse_with_migration(text).map(|(config, _)| config)
}

fn parse_with_migration(text: &str) -> Result<(Config, bool), ConfigError> {
    if text.len() > MAX_CONFIG_FILE_BYTES {
        return Err(invalid_reason("configuration file exceeds maximum size"));
    }
    let mut config = Config::default();
    let mut migrated = false;
    let mut seen_keys = HashSet::new();
    for (line_number, raw) in text.lines().enumerate() {
        if raw.len() > MAX_CONFIG_LINE_BYTES {
            return Err(invalid_reason(format!(
                "line {} exceeds maximum size",
                line_number + 1
            )));
        }
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = line
            .split_once('=')
            .ok_or_else(|| invalid_reason(format!("line {} needs key = value", line_number + 1)))?;
        let key = raw_key.trim();
        let value = raw_value.trim();
        if key.len() > MAX_CONFIG_KEY_BYTES {
            return Err(invalid_reason(format!(
                "line {} key exceeds maximum size",
                line_number + 1
            )));
        }
        if value.len() > MAX_CONFIG_VALUE_BYTES {
            return Err(invalid_reason(format!(
                "line {} value exceeds maximum size",
                line_number + 1
            )));
        }
        if !seen_keys.insert(key) {
            return Err(invalid_reason(format!("duplicate configuration key {key}")));
        }
        match key {
            "automatic" => {
                config.automatic = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(invalid_reason(format!("invalid automatic value {other}")))
                    }
                }
            }
            "startup_enabled" => {
                config.startup_enabled = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(invalid_reason(format!(
                            "invalid startup_enabled value {other}"
                        )))
                    }
                }
            }
            "request_interval_hns" => {
                let value = value.parse::<u64>().map_err(|_| {
                    invalid_reason(format!(
                        "invalid request interval on line {}",
                        line_number + 1
                    ))
                })?;
                config.request_interval =
                    if Hns::new(value) == LEGACY_ONE_MILLISECOND_REQUEST_INTERVAL {
                        migrated = true;
                        AUTOMATIC_REQUEST_INTERVAL
                    } else {
                        Hns::new(value)
                    };
            }
            other => return Err(invalid_reason(format!("unknown configuration key {other}"))),
        }
    }
    Ok((config, migrated))
}

fn invalid_reason(reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(tick_diagnostics::truncate_utf8(
        &reason.into(),
        MAX_CONFIG_ERROR_BYTES,
    ))
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
                request_interval: AUTOMATIC_REQUEST_INTERVAL
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
                request_interval: AUTOMATIC_REQUEST_INTERVAL
            }
        );
    }

    #[test]
    fn preserves_explicit_false_values_during_migration() {
        let config = parse("automatic = false\nstartup_enabled = false\n").unwrap();
        assert!(!config.automatic);
        assert!(!config.startup_enabled);
        assert_eq!(config.request_interval, AUTOMATIC_REQUEST_INTERVAL);
    }

    #[test]
    fn migrates_the_legacy_one_millisecond_request_to_automatic_selection() {
        let (config, migrated) = parse_with_migration(
            "automatic = false\nstartup_enabled = true\nrequest_interval_hns = 10000\n",
        )
        .unwrap();
        assert!(migrated);
        assert_eq!(config.request_interval, AUTOMATIC_REQUEST_INTERVAL);
    }

    #[test]
    fn rejects_unknown_configuration() {
        assert!(parse("profile = game").is_err());
    }

    #[test]
    fn rejects_duplicate_keys_and_invalid_scalar_values() {
        assert!(parse("automatic = true\nautomatic = false\n").is_err());
        assert!(parse("automatic = yes\n").is_err());
        assert!(parse("request_interval_hns = nope\n").is_err());
    }

    #[test]
    fn rejects_oversized_lines_and_values() {
        assert!(parse(&format!(
            "automatic = {}\n",
            "x".repeat(MAX_CONFIG_VALUE_BYTES + 1)
        ))
        .is_err());
        assert!(parse(&"x".repeat(MAX_CONFIG_LINE_BYTES + 1)).is_err());
    }

    #[test]
    fn rejects_oversized_files_and_invalid_utf8() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-bounds-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, vec![b'x'; MAX_CONFIG_FILE_BYTES + 1]).unwrap();
        assert!(load(&path).is_err());
        fs::write(&path, *b"a\xff").unwrap();
        assert!(load(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serializes_configuration_for_persistence() {
        let root = std::env::temp_dir().join(format!("true-tick-config-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        let config = Config {
            automatic: true,
            startup_enabled: true,
            request_interval: AUTOMATIC_REQUEST_INTERVAL,
        };
        save_atomic(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        fs::remove_dir_all(root).unwrap();
    }
}
