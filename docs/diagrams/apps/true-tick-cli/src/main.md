# true-tick-cli main.rs

Source: `true-tick/apps/true-tick-cli/src/main.rs`

```mermaid
flowchart TD
    A["main: collect argv, current_exe"] --> B["extract_root: pull --root"]
    B -->|"parse error"| PE["stderr error"] --> E1["exit 1"]
    B --> C["parse_args: dispatch subcommand"]
    C -->|"missing subcommand"| MS["print usage to stderr"] --> E1
    C -->|"unknown or invalid"| UE["stderr error + hint"] --> E1
    C --> D["resolve_root: override else exe dir"]
    D --> H{"request"}
    H -->|"help"| HP["print usage"] --> E0["exit 0"]
    H -->|"version"| VP["print VERSION"] --> E0
    H -->|"logs"| LG["run_logs: read Data/logs csv"]
    LG -->|"missing file"| LF["stderr error"] --> E1
    LG -->|"print tail lines"| E0
    H -->|"daemon"| DM["run_daemon"]
    H -->|"status"| ST["exchange QueryStatus"]
    H -->|"start"| SA["acquire_payload then exchange RequestAcquire"]
    H -->|"stop"| SP["exchange RequestRelease"]
    H -->|"schedule"| SC["encode_schedule_payload then exchange ScheduleAction"]
    H -->|"cancel"| CA["payload: id as u32 LE or empty"]
    CA --> CX["exchange CancelSchedule"]

    ST -->|"status text"| SJ{"--json?"}
    SJ -->|"yes"| SJP["status_json print"] --> E0
    SJ -->|"no"| STP["print text"] --> E0
    ST -->|"other response"| RR
    ST -->|"run error"| RE
    SA --> RR
    SA --> RE
    SP --> RR
    SP --> RE
    SC --> RR
    SC --> RE
    CX --> RR
    CX --> RE

    subgraph PIPE["exchange pipe path"]
        X1["read ipc-token file"] -->|"read fails"| UN["RunError::Unreachable"]
        X1 --> X2["connect named pipe"]
        X2 -->|"connect fails"| UN
        X2 --> X3["client.exchange verb + payload"]
        X3 -->|"io fails"| FA["RunError::Failure"]
    end

    RR["report_response"] -->|"ok variant"| RRO["stdout result"] --> E0
    RR -->|"IpcResponse::Error"| RRE["stderr error"] --> E1
    RE["report_error"] --> REMAP{"RunError kind"}
    REMAP -->|"Failure"| E1
    REMAP -->|"Unreachable"| E2["exit 2"]

    subgraph DAEMON["run_daemon"]
        D1["connect pipe"] -->|"connected"| D2["verify_server_pid"]
        D2 -->|"match"| D3["already running"] --> E0D["exit 0"]
        D2 -->|"mismatch"| DF["failure"]
        D1 -->|"connect fails"| D4["resolve_daemon_target"]
        D4 -->|"no engine found"| DF
        D4 -->|"target"| D5["clear stale token + pid marker"]
        D5 --> D6["spawn detached, cwd = root, null stdio"]
        D6 -->|"spawn fails"| DF
        D6 --> D7["poll: token file + connect + pid verify"]
        D7 -->|"verified before 10s"| D3
        D7 -->|"timeout"| DU["unreachable"]
        DF --> E1D["exit 1"]
        DU --> E2D["exit 2"]
    end
```

## Notes

- Exit codes: 0 success, 1 command/parse/protocol failure, 2 tray unreachable or token unreadable.
- Missing subcommand prints usage to stderr and exits 1.
- `cancel` with no id sends an empty payload, clearing all pending actions, with an id it sends the u32 little endian encoded id.
- `schedule` accepts `start-in`, `stop-in`, `pause` plus a seconds value validated by the shared encoder.
- `logs` reads `Data/logs/true-tick-<date>.csv` directly from disk, never touches the pipe, default tail is 20 lines, missing file exits 1.
- `status --json` converts the space separated `key=value` status text into a JSON object, dropping tokens without `=` or with `unknown` values.
- Daemon path first checks a live pipe, and verifies the serving process PID against the `ipc-server-pid` marker before claiming the daemon is already running.
- Spawn candidates in order: sibling `true-tick.exe`, root `true-tick.exe`, `Slots/A` and `Slots/B` engines, then `Launcher.exe` beside the cli and at root.
- Stale `ipc-token` and `ipc-server-pid` files are cleared before spawn so a crashed engine cannot satisfy the readiness probe.
- Spawn forwards the resolved root as the child working directory, nulls stdio, and on Windows applies detached creation flags.
- Readiness poll waits up to 10 s in 50 ms steps, requiring the token file, a successful connect, and a PID match, timeout maps to exit 2.
- `--root` may appear before or after the subcommand, otherwise root defaults to the executable directory.
