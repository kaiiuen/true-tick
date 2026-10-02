# tray/ipc.rs

Source path: `apps/true-tick/src/tray/ipc.rs`

```mermaid
flowchart TD
    A["IpcServer::new"] --> B["create_pipe_instance probe"]
    B --> C["CloseHandle probe handle"]
    C --> D["spawn server_loop thread"]
    D --> E["Arc bridge, rate limiter, worker pool of 8"]
    E --> F["SessionToken::generate_ephemeral"]
    F --> G{"token generated"}
    G -->|no| G1["log session_token.generation_failed"]
    G -->|yes| H["publish_session_token writes TOKEN_FILE_NAME"]
    H --> I["publish_server_pid_marker writes ipc-server-pid"]
    I --> J{"running set"}
    J -->|no| Z1["remove token file and pid marker"]
    J -->|yes| K["create_pipe_instance"]
    K -->|err| K1["sleep 100 ms"] --> J
    K -->|ok| L{"running still set"}
    L -->|no| L1["PipeGuard closes new pipe then break"] --> Z1
    L -->|yes| M["dispatch_connection"]
    M --> N{"worker_pool try_acquire"}
    N -->|none| N1["increment POOL_REJECT_COUNT, log on first and 64th, close pipe"] --> J
    N -->|some permit| O["build WorkerJob with permit and spawn worker"]
    O -->|spawn err| O1["close pipe and log worker_spawn.failed"] --> J
    O -->|ok| P["handle_pipe_connection with permit held"]
    P --> Q["wait_for_client overlapped connect"]
    Q -->|timeout| Q1["CancelIoEx then drain pending IO"] --> V["disconnect_pipe"]
    Q -->|connected| R["get_client_pid or 0"]
    R --> S{"rate_limiter lock"}
    S -->|poisoned| S1["send_error_response TransportIo code 13"] --> V
    S -->|ok| T{"check_and_record"}
    T -->|over limit| T1["send_error_response and disconnect"] --> V
    T -->|allowed| U["read_frame overlapped read"]
    U -->|err| U1["send_error_response"] --> V
    U -->|frame| W{"token matches"}
    W -->|no| W1["send_error_response UnauthorizedToken"] --> V
    W -->|yes| X["dispatch_command"]
    X --> Y{"verb"}
    Y -->|QueryStatus| Y1["handle_query_status reads snapshot"]
    Y -->|RequestAcquire| Y2["handle_request_acquire payload check"]
    Y -->|ScheduleAction| Y3["handle_schedule_action payload check"]
    Y -->|RequestRelease| Z["forward_to_ui_thread"]
    Y -->|CancelSchedule| Z
    Y2 -->|invalid| U1
    Y2 -->|valid| Z
    Y3 -->|invalid| U1
    Y3 -->|valid| Z
    Z --> Z2{"IPC_TEARDOWN set"}
    Z2 -->|yes| Z3["TransportIo WAIT_TIMEOUT"]
    Z2 -->|no| Z4["push into IpcCommandQueue"]
    Z4 -->|queue full| Z3
    Z4 -->|enqueued| Z5["PostMessageW WM_APP_IPC"]
    Z5 --> Z6{"reply recv_timeout 5000 ms"}
    Z6 -->|reply| Z7["IpcResponse from UI thread"]
    Z6 -->|timeout| Z3
    Z6 -->|disconnected| Z8["TransportIo raw -1"]
    Y1 --> Z9["send_response encode_response"]
    Z7 --> Z9
    Z3 --> Z9
    Z8 --> Z9
    Z9 --> ZA["write_pipe overlapped write with deadline"]
    ZA --> ZB["FlushFileBuffers"]
    ZB --> V
    V --> ZC["PipeGuard drop closes handle, permit returns to pool"]
    ZC --> J
    Z1 --> ZD["shutdown joins thread"]
    ZD --> ZE["Drop removes token file and drains queue via drain_ipc_command_queue_fail"]
```

## Notes

- Worker pool cap is `MAX_CONCURRENT_WORKERS` at 8. Acquisition is non blocking so a saturated pool rejects the connection instead of queueing a waiter.
- `PoolPermit` is RAII and returns the slot on drop, with a debug assertion that the count stays below the cap.
- A saturated rejection increments `POOL_REJECT_COUNT` and logs only on the first rejection and every 64th so a flood cannot drown stderr.
- `IPC_TEARDOWN` is a global `AtomicBool` set through `set_ipc_teardown`. When set, `forward_to_ui_thread` fails immediately with `TransportIo` and `WAIT_TIMEOUT`.
- The PID marker file name is `ipc-server-pid`, written beside the token file through `write_restricted_bytes` with the same SDDL DACL.
- The token file uses `TOKEN_FILE_NAME` inside the state directory and is retracted both on accept loop exit and in `IpcServer::drop`.
- A poisoned rate limiter mutex is fatal for that connection, which replies with `TransportIo` and the raw code `ERROR_INVALID_DATA` of 13.
- `read_frame` bounds its overlapped read by `PIPE_READ_DEADLINE_MS` at 500 ms and `write_pipe` by `PIPE_WRITE_DEADLINE_MS`.
- `forward_to_ui_thread` blocks on a one shot reply channel for at most `IPC_COMMAND_WAIT_MS` at 5000 ms.
- The rate limiter prunes idle callers every 8 accepted connections to bound the records table.
- `PipeGuard` closes the raw handle exactly once on every path, including pool rejection and worker spawn failure.
- `IpcServer::new` probes one pipe instance and closes it before spawning so a persistent creation failure reaches the caller.
