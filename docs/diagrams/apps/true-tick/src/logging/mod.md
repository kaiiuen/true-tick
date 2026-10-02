# true-tick daily CSV diagnostic logging and retention

Source: `apps/true-tick/src/logging/mod.rs`

```mermaid
flowchart TD
    subgraph PathResolution["Path and Filename Resolution"]
        Exe["executable path"] --> ResolveLog["resolve_log_directory(executable)"]
        ResolveLog --> PortableCheck{"portable_root_from_slot_executable"}
        PortableCheck -- "Ok(root)" --> PortableDir["root/Data/logs"]
        PortableCheck -- "Err" --> FallbackDir["executable.parent/logs"]
        Now["utc_date_now()"] --> DailyName["daily_log_filename(year, month, day)"]
        DailyName --> TargetFile["true-tick-YYYY-MM-DD.csv"]
    end

    subgraph RowBuilding["Row Construction and CSV Escaping"]
        EventSnapshot["diagnostics.snapshot()"] --> FilterEvents["filter sequence > baseline"]
        FilterEvents --> GridRow["diagnostic_grid_row(seq, event)"]
        GridRow --> CellEscape["log_csv_escape on 11 grid cells"]
        EventSnapshot --> HashCells["hex_hash_string for prev_hash and entry_hash"]
        HashCells --> EscapeHashes["log_csv_escape on hash strings"]
        CellEscape --> JoinRow["join with comma into CSV line"]
        EscapeHashes --> JoinRow
    end

    subgraph EscapingLogic["Formula Injection Neutralization"]
        RawCell["raw cell text"] --> Truncate["truncate_utf8(cell, 256)"]
        Truncate --> TriggerCheck{"starts with =, @, \\t, \\r, \\n or non-elapsed + or -"}
        TriggerCheck -- "Yes" --> PrefixQuote["wrap in quotes with leading single quote"]
        TriggerCheck -- "No" --> CharCheck{"contains comma, quote, \\n, or \\r"}
        CharCheck -- "Yes" --> StdQuote["wrap in quotes, double internal quotes"]
        CharCheck -- "No" --> RawPass["return raw field unchanged"]
    end

    subgraph FlushEntry["Flush Entry Points"]
        SyncCall["flush_diagnostic_events_to_disk_sync"] --> BuildSync["build_log_lines"]
        BuildSync --> AppendSync["append_log_lines_sync_with_diagnostics"]
        AsyncCall["flush_diagnostic_events_to_disk"] --> BuildAsync["build_log_lines"]
        BuildAsync --> CasWriter{"LOG_WRITER_ACTIVE compare_exchange(false, true)"}
        CasWriter -- "Err (busy)" --> DropAsync["coalesce: drop thread request"]
        CasWriter -- "Ok (acquired)" --> SpawnThread["thread::spawn worker"]
        SpawnThread --> AppendSync
    end

    subgraph AppendEngine["Append under Mutex Lock and Free Space Probe"]
        AppendSync --> CreateDir["fs::create_dir_all(directory)"]
        CreateDir --> ProbeSpace{"free_bytes_available >= 50 MiB?"}
        ProbeSpace -- "No (< 50 MiB)" --> SuppressRecord["record storage.write_suppressed"]
        SuppressRecord --> ErrSpace["Err InsufficientFreeSpace"]
        ProbeSpace -- "Yes or None" --> CheckNew["file_is_new = !path.exists()"]
        CheckNew --> LockMutex["LOG_APPEND_LOCK.lock()"]
        LockMutex --> OpenFile["OpenOptions append(true).create(true)"]
        OpenFile --> CheckEmpty{"file length == 0?"}
        CheckEmpty -- "Yes" --> WriteHeader["writeln LOG_HEADER"]
        CheckEmpty -- "No" --> WriteLines["writeln lines"]
        WriteHeader --> WriteLines
        WriteLines --> FlushSync["file.flush() and file.sync_all()"]
        FlushSync --> NewRetention{"file_is_new == true?"}
        NewRetention -- "Yes" --> PurgeLogs["purge_expired_logs_with_diagnostics"]
        NewRetention -- "No" --> CommitSeq["store last_persisted_event_sequence"]
        PurgeLogs --> CommitSeq
        CommitSeq --> DropGuard["LogWriterActiveGuard resets active flag"]
    end
```

## Notes

* Daily log directory resolves to Data/logs inside portable root when portable layout matches or falls back to logs adjacent to the executable
* Daily CSV file name is formatted as true-tick-YYYY-MM-DD.csv using UTC civil date components derived from epoch days
* CSV header LOG_HEADER is written exactly once when a log file is newly created or has zero byte length
* Mutex LOG_APPEND_LOCK serializes header inspection and writes across threads to prevent duplicate headers and torn writes
* Asynchronous flush uses atomic flag LOG_WRITER_ACTIVE with compare exchange to coalesce concurrent flush attempts into active writer passes
* Free space probe calls GetDiskFreeSpaceExW on Windows and suppresses writes if available space falls below 50 MiB threshold
* Suppressed writes record a storage.write_suppressed diagnostic entry without crashing or blocking the host application
* CSV field values are truncated to 256 UTF-8 bytes and formula injection trigger characters are escaped with a leading single quote
* Normal elapsed millisecond values beginning with plus or minus are recognized and bypass formula neutralization single quotes
* Log rows combine 11 standard diagnostic grid columns with hex formatted prev_hash and entry_hash integrity chain fields
* Expired log retention purge runs automatically when a new daily log file is first created and at startup via dedicated entry point
* Synchronous flush helper guarantees all queued diagnostic rows are written and synced to disk before process exit
