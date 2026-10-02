# IPC and Configuration

This guide is the written reference for the True Tick inter-process communication protocol and the runtime configuration file. It covers the named pipe endpoint and its security model, the wire frame layout, the command verbs and their payload shapes, the error model, how the tray engine dispatches commands, and the persisted settings file.

Per-file Mermaid diagrams are kept in `docs/diagrams/` as an alternate visual view. The diagrams at `docs/diagrams/crates/tick-ipc/src/lib.md` and `docs/diagrams/apps/true-tick/src/tray/ipc.md` describe the same protocol in graphical form and are kept in sync with the code and this prose.

## Named pipe endpoint and security model

The tray process hosts a Windows named pipe server at `\\.\pipe\TrueTick-Ipc-v1`. The endpoint exists so that the command line client and other local tools can query status, acquire or release the timer resolution, and schedule timed actions without driving the GUI directly.

The pipe is created with an explicit security descriptor built from the SDDL string `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)`. This DACL grants generic-all access to the local SYSTEM account, the Administrators group, and the pipe owner, and nothing else. Earlier grants to authenticated users and restricted code were removed so only the owning desktop session and elevated service accounts can open the pipe.

Opening the pipe is not sufficient to issue a command. Every frame must carry a 32 byte session token that the server generates at startup with OS entropy, using `BCryptGenRandom` on Windows and `/dev/urandom` elsewhere. The token is published as 64 lowercase hexadecimal characters in a file named `ipc-token` inside the application state directory, and the file inherits the owner scoped ACL of that directory. A client reads the token file, embeds the token in the frame header, and the server validates it with a constant time comparison before dispatching. When the server shuts down the token file and the PID marker are retracted so a stopped server does not leave credentials behind.

To defeat name squatting, the server also writes a marker file named `ipc-server-pid` beside the token file. It contains the decimal process ID of the tray process and is written through the same restricted helper that applies the pipe SDDL to the file object. A client can compare the marker PID against the process that actually owns the pipe to confirm it is talking to the genuine tray process rather than a stale or hostile endpoint.

The server enforces additional resource bounds. A counted semaphore caps the concurrent worker pool at 8, and acquisition is non blocking so a saturated pool rejects the connection outright. A per PID rate limiter allows at most 10 requests per second in a one second sliding window and tracks at most 64 distinct caller PIDs, evicting the least recently seen record when the table fills.

## Wire frame layout

All requests and responses on the pipe use a fixed binary frame format. The header is 44 bytes, followed by a variable payload and a 4 byte CRC-32 trailer.

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 4 | Magic | ASCII `TTIP`, bytes `54 54 49 50` |
| 4 | 2 | Protocol version | `0x0001` little endian |
| 6 | 2 | Payload length | `u16` little endian, maximum 256 |
| 8 | 32 | Session token | 32 byte ephemeral token |
| 40 | 4 | Command verb | `u32` little endian discriminant |
| 44 | 0 to 256 | Payload | verb specific bytes |
| 44 + len | 4 | CRC-32 | CRC of all preceding bytes, little endian |

The CRC-32 uses the standard IEEE polynomial `0xEDB88320` and covers the entire frame from the magic through the last payload byte. The minimum legal frame is 48 bytes when the payload is empty.

Validation is zero trust. The deserializer checks the magic first, then the version, then the declared payload length against the 256 byte bound, then that the total frame length is exactly header plus payload plus CRC. A frame that is short is `PayloadTruncated`. A frame that is long is `TrailingBytes` and is rejected rather than silently truncated. Only after the CRC matches is the verb decoded into one of the five known values.

## Command verbs and payload shapes

Five verbs are defined. The `u32` discriminant is what appears on the wire.

| Verb | Code | Request payload | Response |
|---|---|---|---|
| `QueryStatus` | `0x01` | Empty | `Status` with UTF-8 text |
| `RequestAcquire` | `0x02` | Empty for automatic, or 8 bytes little endian `u64` interval in HNS | `Acquired` with 8 bytes little endian effective interval, or `Error` |
| `RequestRelease` | `0x03` | Empty | `Released`, or `Error` |
| `ScheduleAction` | `0x04` | 1 byte tag plus 4 bytes little endian `u32` delay in seconds | `Scheduled` with 4 bytes little endian action id, or `Error` |
| `CancelSchedule` | `0x05` | Empty, or 4 bytes little endian `u32` action id | `Cancelled`, or `Error` |

`RequestAcquire` accepts either an empty payload, which selects the configured automatic interval, or an 8 byte little endian value produced by `encode_interval_payload`. The valid interval range is 5,000 HNS to 156,250 HNS, which corresponds to 0.5 ms through 15.625 ms. The value 5,000 is also accepted as a symbolic resolution preset. Any other value is rejected with `IntervalOutOfBounds` carrying the requested number.

`ScheduleAction` uses a 5 byte payload. Byte 0 is a tag: 1 for Start, 2 for Stop, 3 for Pause. Bytes 1 through 4 are a `u32` little endian delay in seconds. The server validates the delay against the same bounds used by the presets manager, 10 through 86,400 seconds, and reports `ScheduleOutOfBounds` with the provided seconds when the value is out of range. On success the reply `Scheduled` carries the action id, which the server mints as a monotonically increasing counter.

`CancelSchedule` accepts either an empty payload or a 4 byte little endian action id. When an id is present it must match the most recently minted schedule id, otherwise the server replies `ScheduleIdMismatch` with the provided id.

## Response wire format

The server answers every request with a single reply payload whose first byte is a tag.

| Tag | Response | Payload after tag |
|---|---|---|
| 1 | `Status` | UTF-8 text of `status={} ownership={} effective_hns={} requested_hns={}` |
| 2 | `Acquired` | 8 bytes little endian `u64` effective interval in HNS |
| 3 | `Released` | None |
| 4 | `Scheduled` | 4 bytes little endian `u32` action id |
| 5 | `Cancelled` | None |
| 6 | `Error` | 8 bytes little endian: 4 byte error code then 4 byte wire parameter |

Any other tag, a truncated payload, trailing bytes, or a `Status` payload that is not valid UTF-8 is rejected as `MalformedResponse`.

## Error model

`IpcError` is a shared enum with a stable numeric code for every variant. The code and a single `u32` wire parameter travel together inside an `Error` response so the client can reconstruct the actionable part of the failure.

| Code | Variant | Wire parameter | Meaning |
|---|---|---|---|
| 1 | `InvalidMagic` | 0 | Magic bytes are not `TTIP` |
| 2 | `UnsupportedVersion` | `u32` of the `u16` found | Protocol version is not 1 |
| 3 | `PayloadTooLarge` | Declared length in bytes | Payload length exceeds 256 |
| 4 | `PayloadTruncated` | 0 | Frame ended before expected length |
| 5 | `InvalidCommandVerb` | Raw verb value | Verb does not map to a known variant |
| 6 | `IntervalOutOfBounds` | Requested interval in HNS | Interval is outside 5,000 to 156,250 |
| 7 | `CrcMismatch` | Expected CRC | Computed CRC does not match trailer |
| 8 | `UnauthorizedToken` | 0 | Session token does not match |
| 9 | `RateLimitExceeded` | Client PID | Caller exceeded 10 requests per second |
| 10 | `MalformedToken` | 0 | Token file or hex encoding is invalid |
| 11 | `MalformedResponse` | 0 | Server reply is malformed |
| 12 | `TransportIo` | Raw OS error code | Pipe or channel I/O failed |
| 13 | `Unsupported` | 0 | Operation is not supported on this platform |
| 14 | `ScheduleOutOfBounds` | Delay in seconds | Schedule delay is outside 10 to 86400 |
| 15 | `TrailingBytes` | Expected frame length | Frame carries extra bytes |
| 16 | `ScheduleIdMismatch` | Provided action id | Cancel id does not match active schedule |

`ScheduleOutOfBounds`, `ScheduleIdMismatch`, and `TrailingBytes` are the three schedule and framing errors added for the current protocol revision. Each carries its offending value on the wire so the client can display the real number instead of a generic failure string.

## Engine handling

The IPC server thread never touches `App` state directly. Mutating commands are pushed into a bounded `IpcCommandQueue` capped at 64 pending requests, then a `WM_APP_IPC` private window message is posted to the tray window. The UI thread drains the queue on receipt of that message and executes each request through the same engine path as the tray menu, keeping all `App` state changes serialized on one thread.

`QueryStatus` is answered on the server thread from a lockless snapshot. The UI thread refreshes an `IpcStatusSnapshot` after every publish, so a status request reads the last committed view without contending on application state.

For mutating commands the pipe handler performs an early payload validation pass so obvious wire errors return immediately. Valid requests are forwarded through the queue and the caller blocks on a one shot reply channel for up to 5,000 ms. A queue that is already full, a tray window that is already gone, or a timeout all surface as a `TransportIo` error with the raw code `WAIT_TIMEOUT`.

## Configuration file

The application reads a TOML file named `true-tick.toml` that lives beside the executable. The file is parsed with a strict subset parser that enforces per line, per key, per value, and whole file size caps. The maximum file size is 64 KiB.

The persisted settings are:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `automatic` | `true` or `false` | `false` | Automatic timing selection is active |
| `startup_enabled` | `true` or `false` | `true` | True Tick starts with Windows |
| `auto_resume_on_ac` | `true` or `false` | `true` | A manual Start reacquires timing on AC restore |
| `battery_lockout` | `true` or `false` | `false` | DC power and Battery Saver block timing requests |
| `request_interval_hns` | `u64` | `0` | Requested timer interval, `0` means automatic |
| `schedule_presets_seconds` | array of `u32` | `[60, 300, 900, 1800, 3600]` | Duration presets for the schedule submenu |

`schedule_presets_seconds` accepts integers from 10 through 86,400 seconds, inclusive, and at most 12 entries. The array must be non empty. A legacy `request_interval_hns` value of 10,000, the old one millisecond request, is migrated in place to `0` which selects automatic.

The file is written atomically through a temporary file and `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` and `MOVEFILE_WRITE_THROUGH`, so a crash during save cannot leave a truncated configuration.

When the file is missing, corrupted, truncated, or not valid UTF-8, the loader backs up the damaged file beside the original as `true-tick.toml.corrupted.<timestamp>.bak` and atomically restores factory defaults. If two recoveries land inside the same millisecond a monotonic counter is appended so each backup name stays unique. This self heal lets the application start normally while preserving the broken file for inspection.
