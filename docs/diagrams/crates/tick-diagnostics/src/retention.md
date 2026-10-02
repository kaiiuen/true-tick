# tick-diagnostics retention architecture

Source: `crates/tick-diagnostics/src/retention.rs`

```mermaid
flowchart TD
    A["purge_expired_logs(log_dir, now_unix_seconds)"] --> B["read_dir(log_dir)"]
    B --> C["Iterate directory entries"]
    C --> D{"Entry is file?"}
    D -- "No" --> C
    D -- "Yes" --> E["file_name().to_str()"]
    E --> F{"UTF-8 name present?"}
    F -- "No" --> C
    F -- "Yes" --> G["daily_log_date_from_name(name)"]

    G --> H{"Exact true-tick-YYYY-MM-DD.csv?"}
    H -- "No" --> C
    H -- "Yes" --> I["Parse year, month, day digits"]
    I --> J{"Valid month 1..=12 and day 1..=31?"}
    J -- "No" --> C
    J -- "Yes" --> K["utc_day_to_unix_seconds(year, month, day)"]

    K --> L["tick_core::days_from_civil(year, month, day)"]
    L --> M["Compute day_unix = days * 86400"]
    M --> N["report.examined incremented"]
    N --> O["candidates.push((day_unix, path))"]
    O --> C

    C --> P["All entries checked"]
    P --> Q["candidates.sort_by_key(candidate.0)"]
    Q --> R["Compute cutoff = now.saturating_sub(LOG_RETENTION_DAYS * 86400)"]
    R --> S["Iterate candidates oldest first"]

    S --> T{"day_unix < retention_cutoff?"}
    T -- "Yes" --> U["fs::remove_file(path)"]
    U --> V{"remove succeeded?"}
    V -- "Yes" --> W["report.removed incremented"]
    V -- "No" --> X["retained.push((day_unix, path))"]
    T -- "No" --> X

    W --> S
    X --> S

    S --> Y["Check count cap overflow"]
    Y --> Z{"retained.len() > MAX_RETAINED_LOG_FILES?"}
    Z -- "No" --> AA["report.retained = retained.len()"]
    Z -- "Yes" --> AB["Calculate overflow = retained.len() - MAX_RETAINED_LOG_FILES"]
    AB --> AC["Iterate oldest overflow entries"]
    AC --> AD["fs::remove_file(path)"]
    AD --> AE{"remove succeeded?"}
    AE -- "Yes" --> AF["report.removed incremented, report.retained decremented"]
    AE -- "No" --> AG["Keep entry"]
    AF --> AC
    AG --> AC
    AC --> AH["retained.drain(..overflow)"]
    AH --> AA

    AA --> AI["Return Ok(report)"]
```

## Notes

* Source file lives at `crates/tick-diagnostics/src/retention.rs`
* `LOG_RETENTION_DAYS` is set to 30 days evaluated at 00:00:00 UTC granularity
* `MAX_RETAINED_LOG_FILES` is set to 64 files to enforce an upper bound count cap
* `daily_log_date_from_name` strictly matches pattern `true-tick-YYYY-MM-DD.csv` and rejects all deviations
* Non-matching file names, invalid date digits, or month and day out of range are skipped without incrementing examined count
* `utc_day_to_unix_seconds` converts civil year, month, and day into days via `tick_core::days_from_civil`
* Daily timestamp in seconds is obtained by multiplying civil days by 86400
* Candidates are sorted chronologically by timestamp so older log files are evaluated and purged first
* First purge pass deletes files whose daily timestamp is strictly older than the calculated retention cutoff
* Any file that fails deletion during the retention pass is preserved in the retained vector
* Second purge pass enforces the file count cap by removing the oldest overflow files beyond 64
* Returns `LogRetentionReport` tracking examined candidate files, removed files count, and final retained count
