use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tick_core::Hns;

use crate::pause::{MAX_PRESETS, MAX_PRESET_SECONDS, MIN_PRESET_SECONDS};

pub const MAX_CONFIG_FILE_BYTES: usize = 64 * 1024;
const MAX_CONFIG_LINE_BYTES: usize = 4 * 1024;
const MAX_CONFIG_KEY_BYTES: usize = 64;
const MAX_CONFIG_VALUE_BYTES: usize = 1024;
const MAX_CONFIG_ERROR_BYTES: usize = 256;

/// Zero is the persisted sentinel for selecting the current native boundary.
pub const AUTOMATIC_REQUEST_INTERVAL: Hns = Hns::ZERO;
/// Legacy one-millisecond config value accepted only during migration.
pub const LEGACY_ONE_MILLISECOND_REQUEST_INTERVAL: Hns = Hns::new(10_000);

/// Factory interval presets offered by the pause and resume menus.
pub const FACTORY_SCHEDULE_PRESETS_SECONDS: [u32; 5] = [60, 300, 900, 1800, 3600];

/// Default setting for whether a manual Start reacquires timing upon returning to AC.
pub const DEFAULT_AUTO_RESUME_ON_AC: bool = true;

/// Default setting for whether DC power and Battery Saver block timing requests.
pub const DEFAULT_BATTERY_LOCKOUT: bool = false;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub automatic: bool,
    pub startup_enabled: bool,
    pub auto_resume_on_ac: bool,
    pub battery_lockout: bool,
    pub request_interval: Hns,
    pub schedule_presets_seconds: Vec<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            automatic: false,
            startup_enabled: true,
            auto_resume_on_ac: DEFAULT_AUTO_RESUME_ON_AC,
            battery_lockout: DEFAULT_BATTERY_LOCKOUT,
            request_interval: AUTOMATIC_REQUEST_INTERVAL,
            schedule_presets_seconds: FACTORY_SCHEDULE_PRESETS_SECONDS.to_vec(),
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
    load_with_migration(path).map(|outcome| outcome.config)
}

#[cfg(test)]
pub fn load_with_recovery(path: &Path) -> Result<LoadOutcome, ConfigError> {
    load_with_migration(path)
}

/// Result of loading the on-disk configuration.
/// `migrated` marks a legacy value that was rewritten in place.
/// `recovered_from_corruption` marks a damaged file that was preserved as a
/// timestamped backup and replaced with factory defaults.
pub struct LoadOutcome {
    pub config: Config,
    pub migrated: bool,
    pub recovered_from_corruption: bool,
}

pub fn load_with_migration(path: &Path) -> Result<LoadOutcome, ConfigError> {
    let metadata = fs::metadata(path).map_err(ConfigError::Read)?;
    if metadata.len() > MAX_CONFIG_FILE_BYTES as u64 {
        return Err(invalid_reason("configuration file exceeds maximum size"));
    }
    let bytes = fs::read(path).map_err(ConfigError::Read)?;
    if bytes.len() > MAX_CONFIG_FILE_BYTES {
        return Err(invalid_reason("configuration file exceeds maximum size"));
    }
    let outcome = if bytes.is_empty() {
        recover_corrupted(path)?
    } else {
        match String::from_utf8(bytes) {
            Ok(text) => match parse_with_migration(&text) {
                Ok((config, migrated)) => {
                    if migrated {
                        save_atomic(path, &config)?;
                    }
                    LoadOutcome {
                        config,
                        migrated,
                        recovered_from_corruption: false,
                    }
                }
                Err(_) => recover_corrupted(path)?,
            },
            Err(_) => recover_corrupted(path)?,
        }
    };
    Ok(outcome)
}

/// Preserves the damaged file beside the original as
/// `true-tick.toml.corrupted.<timestamp>.bak` then atomically rewrites a clean
/// factory default configuration so the next start loads normally.
/// When two recoveries land inside the same millisecond the suffix walks a
/// monotonic counter so each backup name stays unique instead of failing.
fn recover_corrupted(path: &Path) -> Result<LoadOutcome, ConfigError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let base = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("true-tick.toml"))
        .to_os_string();
    let mut attempt = 0u32;
    let backup = loop {
        let mut name = base.clone();
        if attempt == 0 {
            name.push(format!(".corrupted.{timestamp}.bak"));
        } else {
            name.push(format!(".corrupted.{timestamp}.{attempt}.bak"));
        }
        let candidate = parent.join(name);
        if !candidate.exists() {
            break candidate;
        }
        attempt += 1;
        if attempt > 1024 {
            return Err(ConfigError::Write(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique corrupted config backup name",
            )));
        }
    };
    fs::rename(path, &backup).map_err(ConfigError::Write)?;
    let config = Config::default();
    save_atomic(path, &config)?;
    Ok(LoadOutcome {
        config,
        migrated: false,
        recovered_from_corruption: true,
    })
}

/// Per process sequence that keeps concurrent saves from sharing a temporary file.
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Builds a temporary name beside the target so the replacement stays on one volume.
/// The name keeps the `*.toml.tmp` ignore pattern and adds the process id plus a
/// monotonic counter for uniqueness.
fn temporary_path_for(path: &Path) -> PathBuf {
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_stem()
        .unwrap_or_else(|| OsStr::new("true-tick"))
        .to_os_string();
    name.push(format!(".{}.{}.toml.tmp", std::process::id(), sequence));
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

pub fn save_atomic(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let temporary = temporary_path_for(path);
    let presets = config
        .schedule_presets_seconds
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!(
        "automatic = {}\nstartup_enabled = {}\nauto_resume_on_ac = {}\nbattery_lockout = {}\nrequest_interval_hns = {}\nschedule_presets_seconds = [{}]\n",
        config.automatic,
        config.startup_enabled,
        config.auto_resume_on_ac,
        config.battery_lockout,
        config.request_interval.value(),
        presets
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
            "auto_resume_on_ac" => {
                config.auto_resume_on_ac = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(invalid_reason(format!(
                            "invalid auto_resume_on_ac value {other}"
                        )))
                    }
                }
            }
            "battery_lockout" => {
                config.battery_lockout = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(invalid_reason(format!(
                            "invalid battery_lockout value {other}"
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
            "schedule_presets_seconds" => {
                config.schedule_presets_seconds = parse_schedule_presets(value, line_number + 1)?;
            }
            other => return Err(invalid_reason(format!("unknown configuration key {other}"))),
        }
    }
    Ok((config, migrated))
}

fn parse_schedule_presets(value: &str, line_number: usize) -> Result<Vec<u32>, ConfigError> {
    let invalid_line = |reason: &str| invalid_reason(format!("line {line_number}: {reason}"));
    let inner = value
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
        .ok_or_else(|| invalid_line("schedule_presets_seconds must be an integer array"))?;
    let mut presets = Vec::with_capacity(MAX_PRESETS);
    for element in inner.split(',') {
        let element = element.trim();
        if element.is_empty() {
            continue;
        }
        let seconds = element
            .parse::<u32>()
            .map_err(|_| invalid_line("schedule preset must be an integer"))?;
        if !(MIN_PRESET_SECONDS..=MAX_PRESET_SECONDS).contains(&seconds) {
            return Err(invalid_line("schedule preset out of bounds"));
        }
        if presets.len() >= MAX_PRESETS {
            return Err(invalid_line("schedule presets exceed maximum count"));
        }
        presets.push(seconds);
    }
    if presets.is_empty() {
        return Err(invalid_line("schedule_presets_seconds must not be empty"));
    }
    Ok(presets)
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
                auto_resume_on_ac: true,
                battery_lockout: false,
                request_interval: AUTOMATIC_REQUEST_INTERVAL,
                schedule_presets_seconds: vec![60, 300, 900, 1800, 3600]
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
                auto_resume_on_ac: true,
                battery_lockout: false,
                request_interval: AUTOMATIC_REQUEST_INTERVAL,
                schedule_presets_seconds: FACTORY_SCHEDULE_PRESETS_SECONDS.to_vec()
            }
        );
    }

    #[test]
    fn parses_explicit_schedule_presets() {
        let config =
            parse("automatic = false\nschedule_presets_seconds = [30, 120, 600, 7200]\n").unwrap();
        assert_eq!(config.schedule_presets_seconds, vec![30, 120, 600, 7200]);
        assert_eq!(
            config,
            Config {
                schedule_presets_seconds: vec![30, 120, 600, 7200],
                ..Config::default()
            }
        );
    }

    #[test]
    fn parses_schedule_presets_with_whitespace_and_single_entry() {
        let config = parse("schedule_presets_seconds = [ 15 ]\n").unwrap();
        assert_eq!(config.schedule_presets_seconds, vec![15]);
        let padded = parse("schedule_presets_seconds = [10,  20 ,30]\n").unwrap();
        assert_eq!(padded.schedule_presets_seconds, vec![10, 20, 30]);
    }

    #[test]
    fn missing_schedule_presets_fall_back_to_factory_defaults() {
        let config = parse("automatic = true\nstartup_enabled = false\n").unwrap();
        assert_eq!(
            config.schedule_presets_seconds,
            vec![60, 300, 900, 1800, 3600]
        );
        assert_eq!(
            config.schedule_presets_seconds,
            FACTORY_SCHEDULE_PRESETS_SECONDS.to_vec()
        );
    }

    #[test]
    fn rejects_out_of_bounds_schedule_presets() {
        assert!(parse("schedule_presets_seconds = [9]\n").is_err());
        assert!(parse("schedule_presets_seconds = [10]\n").is_ok());
        assert!(parse("schedule_presets_seconds = [86400]\n").is_ok());
        assert!(parse("schedule_presets_seconds = [86401]\n").is_err());
    }

    #[test]
    fn rejects_malformed_schedule_presets() {
        assert!(parse("schedule_presets_seconds = 60\n").is_err());
        assert!(parse("schedule_presets_seconds = [60\n").is_err());
        assert!(parse("schedule_presets_seconds = []\n").is_err());
        assert!(parse("schedule_presets_seconds = [60, abc]\n").is_err());
        assert!(parse("schedule_presets_seconds = [-60]\n").is_err());
    }

    #[test]
    fn caps_schedule_presets_at_twelve_items() {
        let twelve: Vec<u32> = (1..=12).map(|step| step * 10).collect();
        let text = format!(
            "schedule_presets_seconds = [{}]\n",
            twelve
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(parse(&text).unwrap().schedule_presets_seconds, twelve);
        let thirteen: Vec<u32> = (1..=13).map(|step| step * 10).collect();
        let text = format!(
            "schedule_presets_seconds = [{}]\n",
            thirteen
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(parse(&text).is_err());
    }

    #[test]
    fn rejects_duplicate_schedule_presets_key() {
        assert!(
            parse("schedule_presets_seconds = [60]\nschedule_presets_seconds = [120]\n").is_err()
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
        let outcome = load_with_recovery(&path).unwrap();
        assert!(outcome.recovered_from_corruption);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_auto_resume_on_ac_boolean_values() {
        let parsed_true = parse("auto_resume_on_ac = true\n").unwrap();
        assert!(parsed_true.auto_resume_on_ac);
        let parsed_false = parse("auto_resume_on_ac = false\n").unwrap();
        assert!(!parsed_false.auto_resume_on_ac);
        let absent = parse("automatic = false\n").unwrap();
        assert!(absent.auto_resume_on_ac);
    }

    #[test]
    fn battery_lockout_defaults_to_false() {
        let absent = parse("automatic = false\n").unwrap();
        assert!(!absent.battery_lockout);
        assert!(!Config::default().battery_lockout);
    }

    #[test]
    fn parses_explicit_battery_lockout_values() {
        let parsed_true = parse("battery_lockout = true\n").unwrap();
        assert!(parsed_true.battery_lockout);
        let parsed_false = parse("battery_lockout = false\n").unwrap();
        assert!(!parsed_false.battery_lockout);
        assert!(parse("battery_lockout = maybe\n").is_err());
        assert!(parse("battery_lockout = true\nbattery_lockout = false\n").is_err());
    }

    #[test]
    fn consecutive_saves_leave_the_second_content() {
        let root = std::env::temp_dir().join(format!(
            "true-tick-config-consecutive-{}",
            std::process::id()
        ));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        let first = Config {
            automatic: false,
            startup_enabled: true,
            auto_resume_on_ac: true,
            battery_lockout: false,
            request_interval: AUTOMATIC_REQUEST_INTERVAL,
            schedule_presets_seconds: FACTORY_SCHEDULE_PRESETS_SECONDS.to_vec(),
        };
        let second = Config {
            automatic: true,
            startup_enabled: false,
            auto_resume_on_ac: false,
            battery_lockout: true,
            request_interval: Hns::new(2_000_000),
            schedule_presets_seconds: vec![15, 45, 7200],
        };
        save_atomic(&path, &first).unwrap();
        save_atomic(&path, &second).unwrap();
        assert_eq!(load(&path).unwrap(), second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_save_leaves_no_temporary_file() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-cleanup-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        save_atomic(&path, &Config::default()).unwrap();
        let leftovers: Vec<PathBuf> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".toml.tmp"))
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files remained: {leftovers:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn temporary_names_differ_for_consecutive_saves() {
        let path = Path::new("true-tick.toml");
        let first = temporary_path_for(path);
        let second = temporary_path_for(path);
        assert_ne!(first, second);
        assert_eq!(first.parent(), path.parent());
        assert_eq!(second.parent(), path.parent());
        for name in [&first, &second] {
            let file_name = name.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                file_name.ends_with(".toml.tmp"),
                "unexpected name {file_name}"
            );
            assert!(file_name.contains(&std::process::id().to_string()));
        }
    }

    #[test]
    fn serializes_configuration_for_persistence() {
        let root = std::env::temp_dir().join(format!("true-tick-config-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        let config = Config {
            automatic: true,
            startup_enabled: true,
            auto_resume_on_ac: false,
            battery_lockout: true,
            request_interval: AUTOMATIC_REQUEST_INTERVAL,
            schedule_presets_seconds: FACTORY_SCHEDULE_PRESETS_SECONDS.to_vec(),
        };
        save_atomic(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serializes_schedule_presets_for_persistence() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-presets-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        let config = Config {
            automatic: false,
            startup_enabled: true,
            auto_resume_on_ac: true,
            battery_lockout: false,
            request_interval: AUTOMATIC_REQUEST_INTERVAL,
            schedule_presets_seconds: vec![45, 600, 7200],
        };
        save_atomic(&path, &config).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("schedule_presets_seconds = [45, 600, 7200]"),
            "unexpected serialized text {text}"
        );
        assert!(
            text.contains("battery_lockout = false"),
            "unexpected serialized text {text}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupted_config_triggers_backup_and_self_healing_regeneration() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-corrupted-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "automatic = maybe this is not toml\n").unwrap();
        let outcome = load_with_recovery(&path).unwrap();
        assert!(outcome.recovered_from_corruption);
        assert_eq!(outcome.config, Config::default());
        let backups: Vec<PathBuf> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with("true-tick.toml.corrupted.") && name.ends_with(".bak")
                })
            })
            .collect();
        assert_eq!(backups.len(), 1, "expected one backup: {backups:?}");
        assert_eq!(load(&path).unwrap(), Config::default());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn truncated_zero_byte_config_triggers_self_healing() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-truncated-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "").unwrap();
        let outcome = load_with_recovery(&path).unwrap();
        assert!(outcome.recovered_from_corruption);
        assert_eq!(outcome.config, Config::default());
        assert!(path.exists());
        assert_eq!(load(&path).unwrap(), Config::default());
        let backups: Vec<PathBuf> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with("true-tick.toml.corrupted.") && name.ends_with(".bak")
                })
            })
            .collect();
        assert_eq!(backups.len(), 1, "expected one backup: {backups:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupted_utf8_triggers_self_healing() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-utf8-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, vec![0xff, 0xfe, b'a', b'=']).unwrap();
        let outcome = load_with_recovery(&path).unwrap();
        assert!(outcome.recovered_from_corruption);
        assert_eq!(outcome.config, Config::default());
        assert_eq!(load(&path).unwrap(), Config::default());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupted_backup_collision_allocates_a_unique_name() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-collision-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "automatic = not valid toml\n").unwrap();
        let first = load_with_recovery(&path).unwrap();
        assert!(first.recovered_from_corruption);

        fs::write(&path, "automatic = still not valid\n").unwrap();
        let second = load_with_recovery(&path).unwrap();
        assert!(second.recovered_from_corruption);

        let mut backups: Vec<PathBuf> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with("true-tick.toml.corrupted.") && name.ends_with(".bak")
                })
            })
            .collect();
        backups.sort();
        assert_eq!(backups.len(), 2, "expected two backups: {backups:?}");
        assert_ne!(backups[0], backups[1]);
        assert_eq!(load(&path).unwrap(), Config::default());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn valid_config_loads_without_recovery() {
        let root =
            std::env::temp_dir().join(format!("true-tick-config-valid-{}", std::process::id()));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "automatic = true\nstartup_enabled = false\n").unwrap();
        let outcome = load_with_recovery(&path).unwrap();
        assert!(!outcome.recovered_from_corruption);
        assert!(outcome.config.automatic);
        assert!(!outcome.config.startup_enabled);
        let backups: Vec<PathBuf> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(".corrupted."))
            })
            .collect();
        assert!(backups.is_empty(), "unexpected backup files: {backups:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schedule_presets_round_trip_through_persistence() {
        let root = std::env::temp_dir().join(format!(
            "true-tick-config-presets-round-trip-{}",
            std::process::id()
        ));
        let path = root.join("true-tick.toml");
        fs::create_dir_all(&root).unwrap();
        let config = Config {
            automatic: true,
            startup_enabled: false,
            auto_resume_on_ac: false,
            battery_lockout: true,
            request_interval: Hns::new(5_000_000),
            schedule_presets_seconds: vec![10, 60, 900, 86400],
        };
        save_atomic(&path, &config).unwrap();
        let reloaded = load(&path).unwrap();
        assert_eq!(reloaded, config);
        assert_eq!(reloaded.schedule_presets_seconds, vec![10, 60, 900, 86400]);
        save_atomic(&path, &reloaded).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        fs::remove_dir_all(root).unwrap();
    }
}
