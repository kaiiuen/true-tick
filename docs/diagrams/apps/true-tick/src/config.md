# Configuration Lifecycle and Atomic Persistence Flow

Source path: `apps/true-tick/src/config.rs`

```mermaid
flowchart TD
    A["load_with_migration(path)"] --> B["fs::metadata(path)"]
    B --> C{"File size <= 64 KiB"}
    C -->|"No"| D["Err(ConfigError::Invalid)"]
    C -->|"Yes"| E["fs::read(path)"]
    E --> F{"Read bytes empty?"}
    F -->|"Yes"| G["recover_corrupted(path)"]
    F -->|"No"| H{"String::from_utf8(bytes)"}
    H -->|"Err"| G
    H -->|"Ok(text)"| I["parse_with_migration(text)"]
    I --> J{"Parse result"}
    J -->|"Err"| G
    J -->|"Ok((config, migrated))"| K{"migrated == true?"}
    K -->|"Yes"| L["save_atomic(path, config)"]
    L --> M["LoadOutcome (migrated=true, recovered=false)"]
    K -->|"No"| N["LoadOutcome (migrated=false, recovered=false)"]

    subgraph SelfHealing ["Self-Healing Corruption Recovery"]
        G --> O["Compute UNIX epoch millis timestamp"]
        O --> P["Find candidate backup filename loop"]
        P --> Q{"Candidate exists? (attempt <= 1024)"}
        Q -->|"Yes"| R["Increment attempt counter"]
        R --> P
        Q -->|"No"| S["fs::rename(path, backup_path)"]
        S --> T["Config::default()"]
        T --> U["save_atomic(path, default_config)"]
        U --> V["LoadOutcome (migrated=false, recovered=true)"]
    end

    subgraph AtomicSave ["Atomic Persistence: save_atomic"]
        W["save_atomic(path, config)"] --> X["temporary_path_for(path)"]
        X --> Y["Format TOML config text"]
        Y --> Z["File::create(temporary_path)"]
        Z --> AA["file.write_all(text)"]
        AA --> AB["file.sync_all()"]
        AB --> AC["replace_file(temporary_path, path)"]
        AC -->|"Windows"| AD["MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)"]
        AC -->|"Non-Windows"| AE["fs::rename(temporary_path, path)"]
        AD --> AF{"Replace success?"}
        AE --> AF
        AF -->|"Yes"| AG["Ok(())"]
        AF -->|"No"| AH["fs::remove_file(temporary_path) then Err"]
    end

    subgraph Parser ["Parsing and Bounds Checking: parse_with_migration"]
        I --> BA{"Text length <= 64 KiB"}
        BA -->|"No"| BB["Err(Invalid)"]
        BA -->|"Yes"| BC["Iterate lines"]
        BC --> BD{"Line length <= 4 KiB"}
        BD -->|"No"| BB
        BD -->|"Yes"| BE["Strip comments and trim"]
        BE --> BF{"key length <= 64 and value length <= 1024"}
        BF -->|"No"| BB
        BF -->|"Yes"| BG{"Seen key in HashSet?"}
        BG -->|"Yes (duplicate)"| BB
        BG -->|"No"| BH{"Match key"}
        BH -->|"automatic / startup_enabled / auto_resume_on_ac / battery_lockout"| BI["Parse bool (true or false)"]
        BH -->|"request_interval_hns"| BJ{"Value == 10000 (legacy 1 ms)?"}
        BJ -->|"Yes"| BK["migrated = true, interval = 0 (AUTOMATIC)"]
        BJ -->|"No"| BL["interval = parsed u64 Hns"]
        BH -->|"schedule_presets_seconds"| BM["parse_schedule_presets"]
        BM --> BN{"Count 1..=12 and values 1..=86400"}
        BN -->|"Valid"| BO["Update config.schedule_presets_seconds"]
        BN -->|"Invalid"| BB
        BH -->|"Unknown key"| BB
    end
```

## Notes

- Maximum configuration file size is 64 KiB, line size is 4 KiB, key size is 64 bytes, value size is 1024 bytes.
- Default configuration enables startup with automatic request interval zero and factory presets 60, 300, 900, 1800, 3600 seconds.
- `auto_resume_on_ac` defaults to true and `battery_lockout` defaults to false.
- Zero represents `AUTOMATIC_REQUEST_INTERVAL` which selects the native timer boundary.
- Legacy interval of 10000 hectonanoseconds (1 millisecond) is detected during parsing, migrated to zero, and rewritten to disk atomically.
- Corrupted UTF8 or malformed syntax triggers self-healing recovery rather than crashing or terminating the application.
- Self-healing renames the corrupt file to a timestamped backup with format `true-tick.toml.corrupted.<timestamp>.<attempt>.bak` up to 1024 attempts.
- Following backup rotation, self-healing generates and writes clean factory default configuration via atomic save.
- Atomic file saving generates a unique sidecar temporary file using process ID and monotonic sequence counter.
- Atomic persistence flushes data to disk with `sync_all` before replacing the target file.
- On Windows systems replacement uses `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` and `MOVEFILE_WRITE_THROUGH` flags.
- Schedule preset arrays are bounded between 1 and 12 entries with duration between 1 second and 86400 seconds inclusive.
