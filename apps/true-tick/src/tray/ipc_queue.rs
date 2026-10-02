use std::collections::VecDeque;
use std::sync::Mutex;

/// One inbound IPC request waiting for the UI thread. The pipe handler
/// pushes the verb, wire payload, and a one shot responder, then posts
/// `WM_APP_IPC` so the message loop drains the queue.
pub(crate) struct IpcCommandRequest {
    pub(crate) verb: tick_ipc::CommandVerb,
    pub(crate) payload: Vec<u8>,
    pub(crate) responder: std::sync::mpsc::Sender<tick_ipc::IpcResponse>,
}

/// Upper bound on pending IPC commands waiting for the UI thread. A
/// client that authenticates cannot enqueue faster than the message loop
/// drains, so pushes past this ceiling are rejected instead of growing
/// memory without bound.
pub(crate) const MAX_PENDING_IPC_COMMANDS: usize = 64;

/// Thread-safe queue between the IPC server thread and the UI thread.
pub(crate) struct IpcCommandQueue {
    inner: Mutex<VecDeque<IpcCommandRequest>>,
}

impl IpcCommandQueue {
    /// Creates an empty command queue.
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueues a request from the IPC server thread. Returns `true`
    /// when the request was accepted, or `false` when the queue already
    /// holds `MAX_PENDING_IPC_COMMANDS` pending requests so the pipe
    /// handler can surface a bounded error instead of buffering without
    /// limit.
    pub(crate) fn push(&self, request: IpcCommandRequest) -> bool {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.len() >= MAX_PENDING_IPC_COMMANDS {
            return false;
        }
        inner.push_back(request);
        true
    }

    /// Pops the oldest pending request, if any, on the UI thread.
    pub(crate) fn pop(&self) -> Option<IpcCommandRequest> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.pop_front()
    }
}
