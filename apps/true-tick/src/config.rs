use std::fs;
use std::path::{Path, PathBuf};
use tick_core::Hns;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub automatic: bool,
    pub request_interval: Hns,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            automatic: false,
            request_interval: Hns::new(10_000),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "configuration read error: {error}"),
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
    fn parses_explicit_local_configuration() {
        let config = parse("automatic = true\nrequest_interval_hns = 10000\n").unwrap();
        assert_eq!(
            config,
            Config {
                automatic: true,
                request_interval: Hns::new(10_000)
            }
        );
    }

    #[test]
    fn rejects_unknown_configuration() {
        assert!(parse("profile = game").is_err());
    }
}
