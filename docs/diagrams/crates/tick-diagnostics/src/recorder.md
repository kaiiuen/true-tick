# tick-diagnostics src/recorder.rs

Source: `true-tick/crates/tick-diagnostics/src/recorder.rs`

```mermaid
flowchart TD
    A["recorder.rs anomaly snapshot recorder"]

    A --> B["Constants detail 1024 bytes events 64 snapshots 16 directory 8 MiB"]

    A --> C["build_snapshot vector detail environment events"]
    C --> C1["truncate_utf8 detail at 1024 byte boundary"]
    C --> C2["take last 64 events via saturating_sub"]
    C --> C3["serialize_snapshot payload"]
    C3 --> C4["sha256_hex digest over payload bytes"]
    C4 --> C5["AnomalySnapshot with digest"]

    A --> D["serialize_snapshot"]
    D --> D1["[snapshot] vector detail event_count"]
    D --> D2["[environment] os and cpu and battery and uptime lines"]
    D --> D3["[events] one line per event via format_event"]
    D --> D4["[digest] sha256 line"]

    A --> E["enforce_snapshot_retention telemetry_dir"]
    E --> E1["scan only regular files named anomaly star recorder"]
    E --> E2["sort by modified then name"]
    E --> E3{"over count 16 or over bytes 8 MiB?"}
    E3 -- "Yes" --> E4["remove oldest file subtract size"]
    E4 --> E3
    E3 -- "No" --> E5["RetentionReport examined removed bytes_removed retained_bytes"]

    A --> F["write_snapshot_with_retention snapshot target_dir"]
    F --> F1["join telemetry dir create_dir_all"]
    F --> F2["temp anomaly unix_ms recorder tmp"]
    F --> F3["write serialized bytes sync_all drop"]
    F --> F4["atomic rename to anomaly unix_ms recorder"]
    F4 --> F5["enforce_snapshot_retention ok becomes Option report"]
    F5 --> F6["tuple final_path Option RetentionReport"]

    A --> G["write_snapshot"]
    G --> G1["calls write_snapshot_with_retention keeps path discards report"]

    C -.-> D
    F3 -.-> D
```

## Notes

* A snapshot holds a FailureVector, a bounded detail String, an EnvironmentSnapshot and the last 64 DiagnosticEvent values
* EnvironmentSnapshot is a fixed set of u32 and u64 counters set to zero when the platform cannot collect a field
* build_snapshot truncates detail at a UTF-8 boundary so it never exceeds 1024 bytes
* Recent events are capped by slicing with saturating_sub so an empty source keeps an empty list
* The digest is lowercase hex SHA-256 over the fully serialized payload
* serialize_snapshot writes one key per line in a deterministic order with header, environment, events and digest blocks
* Retention only considers regular files directly inside telemetry_dir named anomaly star recorder, it never recurses and never touches directories
* Candidates are sorted by modification time ascending with the name as a deterministic tiebreak
* While count exceeds 16 or total bytes exceed 8 MiB the oldest file is removed and its size is subtracted
* write_snapshot_with_retention writes to a tmp file, syncs it to disk, drops the handle and then renames it atomically
* A retention failure is reported as None and does not remove or invalidate the freshly written snapshot
* write_snapshot wraps the same write and retention flow and returns only the final snapshot path
