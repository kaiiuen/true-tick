# Tray ipc_queue bounded command queue

Source path: `true-tick/apps/true-tick/src/tray/ipc_queue.rs`

```mermaid
flowchart TD
    subgraph REQ["IpcCommandRequest"]
        R1["verb CommandVerb"]
        R2["payload Vec u8"]
        R3["responder mpsc Sender IpcResponse one shot"]
    end

    subgraph QUEUE["IpcCommandQueue over Mutex VecDeque"]
        Q0["IpcCommandQueue::new"] --> Q1["Mutex VecDeque empty"]
        Q1 --> Q2{"push request"}
        Q2 --> Q3["lock inner, recover into_inner on poison"]
        Q3 --> Q4{"len greater or equal MAX_PENDING_IPC_COMMANDS 64"}
        Q4 -->|yes| Q5["return false, caller surfaces bounded error"]
        Q4 -->|no| Q6["push_back request, return true"]
        Q6 --> Q7["pipe handler posts WM_APP_IPC"]
        Q7 --> Q8{"pop on UI thread"}
        Q8 --> Q9["lock inner, recover into_inner on poison"]
        Q9 --> Q10["pop_front Option request"]
        Q10 --> Q11["drain handler replies through responder"]
    end
```

## Notes

- `MAX_PENDING_IPC_COMMANDS` caps the backlog at 64 pending requests. A saturated queue returns `false` so the pipe handler answers with a bounded error instead of buffering without limit.
- Both `push` and `pop` recover the `VecDeque` through `into_inner` on a poisoned mutex, so a panicking thread cannot permanently wedge the queue between the server thread and the UI thread.
- The queue is FIFO through `push_back` and `pop_front`, which preserves client ordering across `WM_APP_IPC` drains.
- The responder is a one shot `mpsc::Sender` owned by the request, so the reply channel travels with the queued item and the pipe worker blocks on it until the UI thread answers or the wait times out.
