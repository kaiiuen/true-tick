# Session Marker Read and Classify Flow

Source path: `apps/true-tick/src/session.rs`

```mermaid
flowchart TD
    A["read_session_marker_bytes(state_directory)"] --> B["std::fs::read(session_marker_path)"]
    B --> C{"Read result"}
    C -->|"Ok(bytes)"| D["Ok(Some(bytes))"]
    C -->|"NotFound"| E["Ok(None)"]
    C -->|"Other io error"| F["Err(SessionError::Unreadable)"]
    D --> G["SessionMarker::new(bytes)"]
    E --> G
    F --> H["classify_session_read(result)"]
    G --> H
    H --> I{"Result kind"}
    I -->|"Ok(Some(marker))"| J["classify_previous_session_bytes(Some(bytes))"]
    I -->|"Ok(None)"| K["PreviousSession::FirstRun"]
    I -->|"Err(Unreadable)"| L["PreviousSession::Unreadable"]
    I -->|"Err(Corrupted or ClockError)"| M["PreviousSession::Corrupted"]
    J --> N{"Bytes classification"}
    N -->|"decode_bytes Ok"| O["UncleanShutdown (binary tombstone)"]
    N -->|"UTF8 clean text"| P["CleanExit"]
    N -->|"UTF8 legacy unclean text"| Q["UncleanShutdown"]
    N -->|"Non-UTF8 or other"| R["Corrupted"]
    S["write_session_bytes_atomic(state_directory, bytes)"] --> T["create_dir_all(state_directory)"]
    T --> U["File::create(temporary path)"]
    U --> V["file.write_all(bytes)"]
    V --> W["file.sync_all()"]
    W --> X["atomic_replace_file(temporary, target)"]
    X -->|"Windows"| Y["MoveFileExW(REPLACE_EXISTING, WRITE_THROUGH)"]
    X -->|"Non-Windows"| Z["std::fs::rename"]
    Y --> AA{"Replace result"}
    Z --> AA
    AA -->|"Success"| AB["Ok(())"]
    AA -->|"Failure"| AC["remove_file(temporary) then Err"]
    O --> AD{"Warning decision"}
    P --> AD
    Q --> AD
    R --> AD
    L --> AD
    M --> AD
    K --> AD
    AD -->|"UncleanShutdown"| AE["Warn: unclean previous session"]
    AD -->|"Corrupted"| AF["Warn: corrupted session marker"]
    AD -->|"Unreadable"| AG["Warn: unreadable session marker"]
    AD -->|"CleanExit or FirstRun"| AH["No warning"]
```

## Notes

- Marker filename is `session.state` inside the state directory.
- Binary tombstone layout: 4 byte magic `TTSS`, 2 byte version `v1`, 32 byte SHA256 checksum, 4 byte little endian pid, 8 byte little endian startup timestamp, total 50 bytes.
- Checksum covers magic plus version plus pid plus timestamp so decode rejects truncation, magic mismatch, version mismatch, and checksum mismatch.
- A missing marker file yields `Ok(None)` and classifies as `FirstRun`.
- Non missing io errors map to `Unreadable`, which classifies distinctly from `FirstRun`.
- Valid binary tombstone means the previous session was still running, so it classifies as `UncleanShutdown`.
- Legacy UTF8 text `clean` classifies as `CleanExit`. `running` and prefixes `state=running` or `state=aborted` classify as `UncleanShutdown`.
- Non UTF8 bytes or unrecognized UTF8 content classify as `Corrupted`.
- `classify_session_read` maps `ClockError` to `Corrupted` and `Unreadable` errors to `Unreadable`.
- Atomic write creates a `session.state.tmp.<pid>` file, writes bytes, calls `sync_all`, then replaces the target.
- On Windows the replace uses `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` and `MOVEFILE_WRITE_THROUGH`. Other platforms use `std::fs::rename`.
- `write_aborted_session_marker` appends a sanitized and truncated reason after the verified tombstone header so the next launch still decodes as unclean.
