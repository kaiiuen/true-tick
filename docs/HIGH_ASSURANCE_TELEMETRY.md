# High-Assurance Telemetry and Forensic Logging

## 1. Regulated Logging Philosophy
True Tick implements mission-critical audit logging inspired by DO-178C avionics and IEC 62304 medical device software standards:
- **Nothing goes unseen**: Every discrete platform interaction, window message, user command, and kernel return emits structured telemetry.
- **Closed-loop confirmation**: Command dispatches are paired with explicit verification events (`phase: Verify`) confirming observed platform state rather than inferring success.
- **Tamper-evident chain of custody**: Every recorded event participates in a cryptographic hash chain.

## 2. 13-Column Forensic Record Schema
All runtime diagnostic events are serialized into daily rolling CSV files under `Data/logs/true-tick-YYYY-MM-DD.csv` adhering to RFC 4180:

| Column | Field Name | Description | Example |
| :--- | :--- | :--- | :--- |
| 1 | `Row` | Monotonic 1-based index within the daily log file | `1` |
| 2 | `Sequence` | Global monotonic sequence counter from process boot | `42` |
| 3 | `Elapsed` | High-precision time delta from application startup | `+1542ms` |
| 4 | `Operation` | Macro-operation identifier grouping related sub-steps | `14` |
| 5 | `Parent` | Parent operation identifier establishing causality | `12` (or `none`) |
| 6 | `Correlation` | Correlation token spanning cross-thread boundaries | `12` |
| 7 | `Phase` | Finite lifecycle phase | `Begin`, `Verify`, `Complete` |
| 8 | `Source` | Event generation domain | `Startup`, `TrayCommand`, `PowerEvent` |
| 9 | `Outcome` | Terminal outcome status | `Completed`, `Failed`, `InProgress` |
| 10 | `Event` | Qualified event name | `native.NtSetTimerResolution.request` |
| 11 | `Details` | Sanitized key-value operational attributes | `requested_hns=5000 raw_status=0` |
| 12 | `PrevHash` | SHA-256 digest of the immediately preceding entry | `e43a47b8f88a901...` |
| 13 | `EntryHash` | SHA-256 digest over current event attributes and `PrevHash` | `7885185527bf0dd...` |

## 3. SHA-256 Cryptographic Chain of Custody
The in-memory event store (`tick-diagnostics`) seeds its initial chain link from a deterministic genesis digest (`TrueTick-Genesis-v1`). For entry $N$, the hash is calculated as:

$$\text{EntryHash}_N = \text{SHA256}(\text{PrevHash}_N \,\|\, \text{Sequence}_N \,\|\, \text{Elapsed}_N \,\|\, \text{Event}_N \,\|\, \text{Details}_N)$$

If any byte, timestamp, sequence number, or detail attribute in a historical log file is modified on disk, replaying the hash chain calculation immediately flags the exact row where verification fails.

## 4. Synchronous Shutdown Disk Flush
To prevent data loss on process termination, the shutdown sequence in `apps/true-tick/src/logging/mod.rs` bypasses the asynchronous background writer and performs a synchronous disk flush (`flush_diagnostic_events_to_disk_sync`), guaranteeing that all events preceding process termination are committed to disk before handle closure.
