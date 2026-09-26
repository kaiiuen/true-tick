//! Daily log retention shared between the `true-tick` binary and its
//! integration tests. A single implementation here removes the previous split
//! where tests validated a copy of the purge logic instead of the production
//! code path.

use std::path::{Path, PathBuf};

/// Daily log files older than this many days at 00:00:00 UTC granularity are
/// deleted during each retention pass.
pub const LOG_RETENTION_DAYS: u64 = 30;

/// Maximum number of daily log files retained after a retention pass. Older
/// files are deleted first when the cap is exceeded.
pub const MAX_RETAINED_LOG_FILES: usize = 64;

/// Summary of one retention pass over the daily log directory.
#[derive(Debug, Default)]
pub struct LogRetentionReport {
    pub examined: usize,
    pub removed: usize,
    pub retained: usize,
}

/// Parses a `true-tick-YYYY-MM-DD.csv` filename and returns the calendar day
/// digits. Any deviation from the exact pattern returns `None`.
pub fn daily_log_date_from_name(name: &str) -> Option<(u16, u16, u16)> {
    let stem = name.strip_prefix("true-tick-")?.strip_suffix(".csv")?;
    if stem.len() != 10 {
        return None;
    }
    let bytes = stem.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits = |slice: &[u8]| -> Option<u16> {
        if !slice.iter().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut value = 0u16;
        for byte in slice {
            value = value.checked_mul(10)?.checked_add(u16::from(byte - b'0'))?;
        }
        Some(value)
    };
    let year = digits(&bytes[0..4])?;
    let month = digits(&bytes[5..7])?;
    let day = digits(&bytes[8..10])?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// Converts a UTC calendar day to unix seconds at 00:00:00. Days outside the
/// representable range return `None` so malformed dates never feed the purge.
pub fn utc_day_to_unix_seconds(year: u16, month: u16, day: u16) -> Option<u64> {
    let month = u32::from(month);
    let day = u32::from(day);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = tick_core::days_from_civil(i64::from(year), month, day);
    Some(days as u64 * 86_400)
}

/// Deletes expired or over-cap daily CSV logs in `log_dir`. Only filenames
/// matching `true-tick-YYYY-MM-DD.csv` are considered, and a failure to read
/// the directory returns the IO error without blocking the caller.
pub fn purge_expired_logs(
    log_dir: &Path,
    now_unix_seconds: u64,
) -> Result<LogRetentionReport, std::io::Error> {
    let mut report = LogRetentionReport::default();
    let entries = std::fs::read_dir(log_dir)?;
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some((year, month, day)) = daily_log_date_from_name(name) else {
            continue;
        };
        let Some(day_unix) = utc_day_to_unix_seconds(year, month, day) else {
            continue;
        };
        report.examined = report.examined.saturating_add(1);
        candidates.push((day_unix, path));
    }
    candidates.sort_by_key(|candidate| candidate.0);
    let retention_cutoff = now_unix_seconds.saturating_sub(LOG_RETENTION_DAYS * 86_400);
    let mut retained: Vec<(u64, PathBuf)> = Vec::new();
    for (day_unix, path) in candidates {
        if day_unix < retention_cutoff {
            if std::fs::remove_file(&path).is_ok() {
                report.removed = report.removed.saturating_add(1);
            } else {
                retained.push((day_unix, path));
            }
        } else {
            retained.push((day_unix, path));
        }
    }
    if retained.len() > MAX_RETAINED_LOG_FILES {
        let overflow = retained.len() - MAX_RETAINED_LOG_FILES;
        for (_, path) in retained.iter().take(overflow) {
            if std::fs::remove_file(path).is_ok() {
                report.removed = report.removed.saturating_add(1);
                report.retained = report.retained.saturating_sub(1);
            }
        }
        retained.drain(..overflow);
    }
    report.retained = retained.len();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{
        daily_log_date_from_name, purge_expired_logs, utc_day_to_unix_seconds, LOG_RETENTION_DAYS,
        MAX_RETAINED_LOG_FILES,
    };

    /// Builds a clean temporary directory inside the crate target directory so
    /// test artifacts never escape the workspace and never collide between runs.
    fn temporary_log_directory(label: &str) -> std::path::PathBuf {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn daily_log_date_from_name_accepts_exact_pattern() {
        assert_eq!(
            daily_log_date_from_name("true-tick-2026-09-16.csv"),
            Some((2026, 9, 16))
        );
        assert_eq!(
            daily_log_date_from_name("true-tick-2031-01-05.csv"),
            Some((2031, 1, 5))
        );
    }

    #[test]
    fn daily_log_date_from_name_rejects_non_matching_names() {
        for name in [
            "other-2020-01-01.csv",
            "true-tick-2020-01-01.txt",
            "true-tick-2020-01-01.csv.bak",
            "true-tick-2020-1-1.csv",
            "true-tick-abcd-01-01.csv",
            "true-tick-2020-ab-01.csv",
            "true-tick-2020-01-ab.csv",
            "true-tick-2020-13-01.csv",
            "true-tick-2020-00-01.csv",
            "true-tick-2020-01-32.csv",
            "notes.txt",
        ] {
            assert_eq!(daily_log_date_from_name(name), None, "name={name}");
        }
    }

    #[test]
    fn utc_day_to_unix_seconds_maps_epoch_day() {
        assert_eq!(utc_day_to_unix_seconds(1970, 1, 1), Some(0));
        assert_eq!(utc_day_to_unix_seconds(2027, 1, 15), Some(1_799_971_200));
        assert_eq!(utc_day_to_unix_seconds(2027, 13, 1), None);
        assert_eq!(utc_day_to_unix_seconds(2027, 1, 32), None);
    }

    #[test]
    fn purge_deletes_files_beyond_retention_window() {
        let directory = temporary_log_directory("purge-retention");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let now = 1_800_000_000u64;
        let old_day = now.saturating_sub((LOG_RETENTION_DAYS + 1) * 86_400);
        let old_year = 1970 + (old_day / (365 * 86_400)) as u16;
        let old_name = format!("true-tick-{old_year:04}-01-01.csv");
        let fresh_name = "true-tick-2027-01-15.csv";
        std::fs::write(directory.join(&old_name), "Row\n1,old\n").expect("old log written");
        std::fs::write(directory.join(fresh_name), "Row\n1,fresh\n").expect("fresh log written");

        let report = purge_expired_logs(&directory, now).expect("purge succeeds");
        assert_eq!(report.examined, 2);
        assert_eq!(report.removed, 1);
        assert_eq!(report.retained, 1);
        assert!(!directory.join(&old_name).exists());
        assert!(directory.join(fresh_name).exists());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn purge_deletes_oldest_beyond_file_cap() {
        let directory = temporary_log_directory("purge-cap");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        // All fixture dates sit inside the retention window so only the file
        // count cap drives deletion. Days are distributed across three months
        // to stay a valid calendar date.
        // 2027-01-31T00:00:00Z, thirty days after the oldest fixture date, so
        // every fixture file sits inside the retention window.
        let now = 1_801_353_600u64;
        let mut names = Vec::new();
        for day in 1..=31 {
            names.push(format!("true-tick-2027-01-{day:02}.csv"));
        }
        for day in 1..=28 {
            names.push(format!("true-tick-2027-02-{day:02}.csv"));
        }
        for day in 1..=11 {
            names.push(format!("true-tick-2027-03-{day:02}.csv"));
        }
        assert_eq!(names.len(), 70);
        for name in &names {
            std::fs::write(directory.join(name), "Row\n1,x\n").expect("log written");
        }
        let report = purge_expired_logs(&directory, now).expect("purge succeeds");
        assert_eq!(report.examined, 70);
        assert_eq!(report.removed, 70 - MAX_RETAINED_LOG_FILES);
        assert_eq!(report.retained, MAX_RETAINED_LOG_FILES);
        assert!(directory.join("true-tick-2027-03-11.csv").exists());
        assert!(directory.join("true-tick-2027-01-07.csv").exists());
        assert!(!directory.join("true-tick-2027-01-01.csv").exists());
        assert!(!directory.join("true-tick-2027-01-06.csv").exists());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn purge_ignores_files_that_do_not_match_the_daily_pattern() {
        let directory = temporary_log_directory("purge-ignore");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let now = 1_800_000_000u64;
        let ignored = [
            "other-2020-01-01.csv",
            "true-tick-2020-01-01.txt",
            "true-tick-2020-01-01.csv.bak",
            "true-tick-2020-1-1.csv",
            "notes.txt",
        ];
        for name in ignored {
            std::fs::write(directory.join(name), "x").expect("file written");
        }
        let report = purge_expired_logs(&directory, now).expect("purge succeeds");
        assert_eq!(report.examined, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.retained, 0);
        for name in ignored {
            assert!(directory.join(name).exists());
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn purge_skips_unparseable_dates() {
        let directory = temporary_log_directory("purge-unparseable");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let now = 1_800_000_000u64;
        let unparseable = [
            "true-tick-abcd-01-01.csv",
            "true-tick-2020-ab-01.csv",
            "true-tick-2020-01-ab.csv",
            "true-tick-2020-13-01.csv",
            "true-tick-2020-00-01.csv",
            "true-tick-2020-01-32.csv",
        ];
        for name in unparseable {
            std::fs::write(directory.join(name), "x").expect("file written");
        }
        let report = purge_expired_logs(&directory, now).expect("purge succeeds");
        assert_eq!(report.examined, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.retained, 0);
        for name in unparseable {
            assert!(directory.join(name).exists());
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn purge_reports_accurate_counts() {
        let directory = temporary_log_directory("purge-counts");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let now = 1_800_000_000u64;
        let fresh = ["true-tick-2027-01-15.csv", "true-tick-2027-01-16.csv"];
        let expired = ["true-tick-2020-01-01.csv", "true-tick-2020-01-02.csv"];
        for name in fresh.iter().chain(expired.iter()) {
            std::fs::write(directory.join(name), "x").expect("file written");
        }
        let report = purge_expired_logs(&directory, now).expect("purge succeeds");
        assert_eq!(report.examined, 4);
        assert_eq!(report.removed, 2);
        assert_eq!(report.retained, 2);
        for name in expired {
            assert!(!directory.join(name).exists());
        }
        for name in fresh {
            assert!(directory.join(name).exists());
        }

        let _ = std::fs::remove_dir_all(&directory);
    }
}
