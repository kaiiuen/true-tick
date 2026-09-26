//! Named pipe IPC endpoint for True Tick.
//!
//! Creates a Windows named pipe server at \\.\pipe\TrueTick-Ipc-v1
//! with an SDDL DACL granting access to SYSTEM, administrators, and the
//! interactive desktop user. Accepts TTIP frames, validates them via
//! tick-ipc, and dispatches to command handlers.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tick_ipc::{
    encode_response, write_token_file, CommandVerb, IpcError, IpcFrame, IpcResponse,
    PidRateLimiter, SessionToken, MAX_PAYLOAD_LEN, TOKEN_FILE_NAME,
};

#[cfg(test)]
use tick_ipc::MAX_REQUESTS_PER_SECOND;

use super::*;

// Named pipe constants
const PIPE_NAME: &str = r"\\.\pipe\TrueTick-Ipc-v1";
const PIPE_BUFFER_SIZE: u32 = 4096;
const PIPE_TIMEOUT_MS: u32 = 5000;

// SDDL DACL restricted to SYSTEM, administrators, and the pipe
// owner. The earlier AU and RC grants were removed so only the owning
// desktop session and elevated service accounts can open the pipe. CLI
// clients authenticate with the session token file published in the
// application state directory.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";

// Windows error codes
#[allow(dead_code)]
const ERROR_PIPE_CONNECTED: u32 = 535;
#[allow(dead_code)]
const ERROR_PIPE_BUSY: u32 = 231;
#[allow(dead_code)]
const ERROR_MORE_DATA: u32 = 234;
#[allow(dead_code)]
const ERROR_BROKEN_PIPE: u32 = 109;
#[allow(dead_code)]
const ERROR_INVALID_HANDLE: u32 = 6;
#[allow(dead_code)]
const ERROR_ACCESS_DENIED: u32 = 5;

// Pipe open modes
#[allow(dead_code)]
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
#[allow(dead_code)]
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;

// Pipe modes
#[allow(dead_code)]
const PIPE_TYPE_MESSAGE: u32 = 0x0000_0004;
#[allow(dead_code)]
const PIPE_READMODE_MESSAGE: u32 = 0x0000_0002;
#[allow(dead_code)]
const PIPE_WAIT: u32 = 0x0000_0000;

// Pipe limits
#[allow(dead_code)]
const PIPE_UNLIMITED_INSTANCES: u32 = 255;

// Read deadline for each client request in milliseconds
const PIPE_READ_DEADLINE_MS: u32 = 500;

// Ceiling in milliseconds for a mutating command to wait on the UI
// thread response before the pipe handler reports a transport timeout.
const IPC_COMMAND_WAIT_MS: u64 = 4_000 + 1_000;

// Wait constants
#[allow(dead_code)]
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x0000_0102;
#[allow(dead_code)]
const INFINITE: u32 = 0xFFFF_FFFF;

// CancelIoEx pending result marker
const ERROR_IO_PENDING: u32 = 997;

// Security descriptor constants
const SDDL_REVISION_1: u32 = 1;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: i32,
}

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut c_void,
}

impl Default for Overlapped {
    fn default() -> Self {
        Self {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: ptr::null_mut(),
        }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer_size: u32,
        in_buffer_size: u32,
        default_timeout: u32,
        security_attributes: *const SecurityAttributes,
    ) -> *mut c_void;

    fn ConnectNamedPipe(pipe: *mut c_void, overlapped: *mut Overlapped) -> i32;
    fn DisconnectNamedPipe(pipe: *mut c_void) -> i32;
    fn CancelIoEx(handle: *mut c_void, overlapped: *const Overlapped) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CreateEventW(
        attributes: *const SecurityAttributes,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    fn GetOverlappedResult(
        handle: *mut c_void,
        overlapped: *const Overlapped,
        bytes_transferred: *mut u32,
        wait: i32,
    ) -> i32;
    fn GetNamedPipeClientProcessId(pipe: *mut c_void, process_id: *mut u32) -> i32;
    fn ReadFile(
        file: *mut c_void,
        buffer: *mut u8,
        bytes_to_read: u32,
        bytes_read: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn WriteFile(
        file: *mut c_void,
        buffer: *const u8,
        bytes_to_write: u32,
        bytes_written: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn FlushFileBuffers(handle: *mut c_void) -> i32;
    #[allow(dead_code)]
    fn SetNamedPipeHandleState(
        pipe: *mut c_void,
        mode: *const u32,
        max_collection_count: *const u32,
        collect_data_timeout: *const u32,
    ) -> i32;
    #[allow(dead_code)]
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string: *const u16,
        revision: u32,
        security_descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[link(name = "user32")]
extern "system" {
    fn PostMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> i32;
}

/// Thread-safe snapshot of the application status surfaced over IPC.
///
/// The tray UI thread refreshes this snapshot after every publish so the
/// IPC server thread never touches `App` state directly.
#[derive(Debug, Default)]
pub struct IpcStatusSnapshot {
    inner: Mutex<IpcStatusInner>,
}

#[derive(Debug)]
struct IpcStatusInner {
    status_label: String,
    ownership_label: String,
    effective_hns: Option<u64>,
    requested_hns: Option<u64>,
}

impl Default for IpcStatusInner {
    fn default() -> Self {
        Self {
            status_label: "starting".to_owned(),
            ownership_label: "released".to_owned(),
            effective_hns: None,
            requested_hns: None,
        }
    }
}

impl IpcStatusSnapshot {
    /// Creates an empty snapshot for the IPC server.
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes a fresh status view from the UI thread.
    pub fn update(
        &self,
        status_label: impl Into<String>,
        ownership_label: impl Into<String>,
        effective_hns: Option<u64>,
        requested_hns: Option<u64>,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.status_label = status_label.into();
            inner.ownership_label = ownership_label.into();
            inner.effective_hns = effective_hns;
            inner.requested_hns = requested_hns;
        }
    }

    /// Serializes the snapshot into the QueryStatus wire payload.
    pub(crate) fn status_payload(&self) -> Vec<u8> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        format!(
            "status={} ownership={} effective_hns={} requested_hns={}",
            inner.status_label,
            inner.ownership_label,
            inner
                .effective_hns
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
            inner
                .requested_hns
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
        )
        .into_bytes()
    }
}

/// Named pipe IPC server handle.
pub struct IpcServer {
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    status: Arc<IpcStatusSnapshot>,
    token_file_path: PathBuf,
}

/// Invocation surface passed into the server loop. Mutating commands are
/// pushed into the queue and flushed by a `WM_APP_IPC` post to the tray
/// window so `App` state is only ever touched on the UI thread.
pub(crate) struct IpcCommandBridge {
    pub(crate) tray_hwnd: *mut c_void,
    pub(crate) queue: Arc<crate::tray::IpcCommandQueue>,
}

// The window handle is an opaque identifier only used for `PostMessageW`,
// never dereferenced, so it is safe to carry across the thread boundary.
unsafe impl Send for IpcCommandBridge {}

impl IpcServer {
    /// Creates and starts the IPC server bound to the shared status snapshot.
    ///
    /// Validates that the first pipe instance can be created before spawning
    /// the accept thread so a persistent permission or name collision failure
    /// is surfaced to the caller instead of looping silently forever. The
    /// accept thread publishes its session token to TOKEN_FILE_NAME inside
    /// the supplied state directory for CLI clients to read.
    pub fn new(
        status: Arc<IpcStatusSnapshot>,
        state_directory: PathBuf,
        bridge: IpcCommandBridge,
    ) -> Result<Self, u32> {
        let first_pipe = create_pipe_instance()?;
        unsafe { CloseHandle(first_pipe) };

        let token_file_path = state_directory.join(TOKEN_FILE_NAME);
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let status_clone = status.clone();
        let thread_token_path = token_file_path.clone();

        let thread = thread::spawn(move || {
            server_loop(running_clone, status_clone, thread_token_path, bridge);
        });

        Ok(Self {
            running,
            thread: Some(thread),
            status,
            token_file_path,
        })
    }

    /// Accesses the shared status snapshot for refresh from the UI thread.
    #[allow(dead_code)]
    pub fn status(&self) -> Arc<IpcStatusSnapshot> {
        self.status.clone()
    }

    /// Signals the server to stop and waits for the thread to finish.
    ///
    /// Returns the join error when the accept thread panicked so the caller
    /// can record the stop failure without crashing the shutdown path.
    pub fn shutdown(&mut self) -> Result<(), Box<dyn std::any::Any + Send>> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            return thread.join();
        }
        Ok(())
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
        // Best effort token file cleanup so a stopped server does not leave
        // a stale credential that CLI clients could present to a new owner.
        let _ = std::fs::remove_file(&self.token_file_path);
    }
}

/// Converts a Rust string to a UTF-16 wide string for Windows APIs.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Creates a security descriptor from SDDL string.
fn create_security_descriptor() -> Result<*mut c_void, u32> {
    let sddl = wide(PIPE_SDDL);
    let mut descriptor: *mut c_void = ptr::null_mut();
    let mut size: u32 = 0;

    let result = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            &mut size,
        )
    };

    if result == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(descriptor)
    }
}

/// Main server loop that accepts and handles pipe connections.
fn server_loop(
    running: Arc<AtomicBool>,
    status: Arc<IpcStatusSnapshot>,
    token_path: PathBuf,
    bridge: IpcCommandBridge,
) {
    let mut rate_limiter = PidRateLimiter::new();
    let session_token = SessionToken::generate_ephemeral();
    publish_session_token(&token_path, &session_token);
    let connection_count = AtomicU64::new(0);

    while running.load(Ordering::SeqCst) {
        match create_pipe_instance() {
            Ok(pipe) => {
                if !running.load(Ordering::SeqCst) {
                    unsafe { CloseHandle(pipe) };
                    break;
                }
                handle_pipe_connection(pipe, &session_token, &mut rate_limiter, &status, &bridge);
                unsafe { CloseHandle(pipe) };
                // Prune stale rate limiter entries periodically so repeated
                // short-lived clients cannot grow the records table unbounded.
                if connection_count
                    .fetch_add(1, Ordering::Relaxed)
                    .is_multiple_of(8)
                {
                    rate_limiter.prune_idle_callers(Instant::now());
                }
            }
            Err(_) => {
                // Pipe creation failed, wait before retrying
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    // The accept loop exited, retract the token file so a stopped server
    // does not leave a stale credential behind. IpcServer Drop also removes
    // the path so this is a belt and suspenders cleanup for early exits.
    let _ = std::fs::remove_file(&token_path);
}

/// Publishes the session token to TOKEN_FILE_NAME inside the state
/// directory so CLI clients can attach. The server thread has no App
/// access so failures are recorded to stderr and the server keeps serving,
/// which leaves IPC unreachable but keeps the diagnostics channel honest.
fn publish_session_token(token_path: &Path, session_token: &SessionToken) {
    if let Err(error) = write_token_file(token_path, session_token) {
        eprintln!(
            "tray.ipc.token_file.write_failed path={} error={error}",
            token_path.display()
        );
    }
}

/// Creates a new named pipe instance with security descriptor.
fn create_pipe_instance() -> Result<*mut c_void, u32> {
    let security_descriptor = create_security_descriptor()?;
    let security_attributes = SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        security_descriptor,
        inherit_handle: 0,
    };

    let pipe_name = wide(PIPE_NAME);
    let pipe = unsafe {
        CreateNamedPipeW(
            pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            PIPE_TIMEOUT_MS,
            &security_attributes,
        )
    };

    // Free the security descriptor
    unsafe { LocalFree(security_descriptor) };

    if pipe.is_null() || pipe as isize == -1 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(pipe)
    }
}

/// Handles a single pipe client connection.
fn handle_pipe_connection(
    pipe: *mut c_void,
    session_token: &SessionToken,
    rate_limiter: &mut PidRateLimiter,
    status: &IpcStatusSnapshot,
    bridge: &IpcCommandBridge,
) {
    // Wait for client to connect
    if !wait_for_client(pipe) {
        return;
    }

    // Get client process ID for rate limiting
    let client_pid = get_client_pid(pipe).unwrap_or(0);

    // Rate limit check
    let now = Instant::now();
    if let Err(error) = rate_limiter.check_and_record(client_pid, now) {
        send_error_response(pipe, error);
        disconnect_pipe(pipe);
        return;
    }

    // Read frame from client
    match read_frame(pipe) {
        Ok(frame) => {
            // Validate token
            if !frame.token.matches(session_token) {
                send_error_response(pipe, IpcError::UnauthorizedToken);
                disconnect_pipe(pipe);
                return;
            }

            // Dispatch command
            let result = dispatch_command(&frame, status, bridge);
            send_response(pipe, result);
        }
        Err(error) => {
            send_error_response(pipe, error);
        }
    }

    disconnect_pipe(pipe);
}

/// Waits for a client to connect to the pipe.
fn wait_for_client(pipe: *mut c_void) -> bool {
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return false;
    }

    let mut overlapped = Overlapped {
        event,
        ..Default::default()
    };

    let connected = unsafe { ConnectNamedPipe(pipe, &mut overlapped) };
    let last_error = unsafe { GetLastError() };

    let result = if connected != 0 || last_error == ERROR_PIPE_CONNECTED {
        true
    } else if last_error == ERROR_IO_PENDING || last_error == ERROR_PIPE_BUSY {
        // Overlapped connect is in flight. Bound the wait so shutdown or a
        // stalled connect cannot hang the server thread, then cancel the
        // pending operation if it did not complete in time.
        let wait = unsafe { WaitForSingleObject(event, PIPE_TIMEOUT_MS) };
        if wait == WAIT_OBJECT_0 {
            true
        } else {
            unsafe { CancelIoEx(pipe, &overlapped) };
            false
        }
    } else {
        false
    };

    unsafe { CloseHandle(event) };
    result
}

/// Gets the process ID of the connected client.
fn get_client_pid(pipe: *mut c_void) -> Option<u32> {
    let mut pid: u32 = 0;
    let result = unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) };
    if result != 0 {
        Some(pid)
    } else {
        None
    }
}

/// Reads a complete IPC frame from the pipe using overlapped IO with a
/// fixed deadline so a stalled client cannot lock the server thread.
fn read_frame(pipe: *mut c_void) -> Result<IpcFrame, IpcError> {
    let mut buffer = vec![0u8; MAX_PAYLOAD_LEN + 64];
    let mut bytes_read: u32 = 0;

    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err(IpcError::PayloadTruncated);
    }

    let mut overlapped = Overlapped {
        event,
        ..Default::default()
    };

    let result = unsafe {
        ReadFile(
            pipe,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut bytes_read,
            &mut overlapped,
        )
    };

    if result == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            unsafe { CloseHandle(event) };
            return Err(match error {
                ERROR_MORE_DATA => IpcError::PayloadTruncated,
                ERROR_BROKEN_PIPE => IpcError::PayloadTruncated,
                _ => IpcError::InvalidMagic,
            });
        }

        // Overlapped read is in flight. Bound the wait so a client that
        // sends a partial frame then stalls cannot hang the server thread.
        let wait = unsafe { WaitForSingleObject(event, PIPE_READ_DEADLINE_MS) };
        if wait == WAIT_TIMEOUT {
            unsafe { CancelIoEx(pipe, &overlapped) };
            unsafe { CloseHandle(event) };
            return Err(IpcError::PayloadTruncated);
        }
        if wait != WAIT_OBJECT_0 {
            unsafe { CancelIoEx(pipe, &overlapped) };
            unsafe { CloseHandle(event) };
            return Err(IpcError::PayloadTruncated);
        }

        let mut transferred: u32 = 0;
        let completed = unsafe { GetOverlappedResult(pipe, &overlapped, &mut transferred, 0) };
        unsafe { CloseHandle(event) };
        if completed == 0 {
            return Err(IpcError::PayloadTruncated);
        }
        bytes_read = transferred;
    } else {
        unsafe { CloseHandle(event) };
    }

    IpcFrame::deserialize(&buffer[..bytes_read as usize])
}

/// Dispatches a validated IPC command to the appropriate handler.
///
/// QueryStatus is served directly from the shared snapshot. Mutating
/// commands get an early payload validation pass for fast wire errors,
/// then are relayed to the UI thread through the command queue and a
/// `WM_APP_IPC` post, and the caller blocks on the per request reply
/// channel up to `IPC_COMMAND_WAIT_MS`.
fn dispatch_command(
    frame: &IpcFrame,
    status: &IpcStatusSnapshot,
    bridge: &IpcCommandBridge,
) -> IpcResponse {
    match frame.verb {
        CommandVerb::QueryStatus => handle_query_status(status),
        CommandVerb::RequestAcquire => match handle_request_acquire(&frame.payload) {
            IpcResponse::Error(error) => IpcResponse::Error(error),
            _ => forward_to_ui_thread(frame.verb, frame.payload.clone(), bridge),
        },
        CommandVerb::RequestRelease => {
            forward_to_ui_thread(frame.verb, frame.payload.clone(), bridge)
        }
        CommandVerb::ScheduleAction => match handle_schedule_action(&frame.payload) {
            IpcResponse::Error(error) => IpcResponse::Error(error),
            _ => forward_to_ui_thread(frame.verb, frame.payload.clone(), bridge),
        },
        CommandVerb::CancelSchedule => {
            forward_to_ui_thread(frame.verb, frame.payload.clone(), bridge)
        }
    }
}

/// Relays one mutating command to the tray message loop and waits for
/// the engine result. The responder channel is one shot so the UI thread
/// reply wakes exactly the pipe handler that enqueued the request.
fn forward_to_ui_thread(
    verb: CommandVerb,
    payload: Vec<u8>,
    bridge: &IpcCommandBridge,
) -> IpcResponse {
    let (responder, reply) = mpsc::channel();
    bridge.queue.push(crate::tray::IpcCommandRequest {
        verb,
        payload,
        responder,
    });
    // The post is best effort: when the tray window is already gone the
    // request stays queued until shutdown and the bounded wait below
    // surfaces the failure as a transport timeout.
    unsafe {
        PostMessageW(bridge.tray_hwnd, WM_APP_IPC, 0, 0);
    }
    match reply.recv_timeout(Duration::from_millis(IPC_COMMAND_WAIT_MS)) {
        Ok(response) => response,
        Err(mpsc::RecvTimeoutError::Timeout) => IpcResponse::Error(IpcError::TransportIo {
            raw_os_error: WAIT_TIMEOUT as i32,
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            IpcResponse::Error(IpcError::TransportIo { raw_os_error: -1 })
        }
    }
}

/// Handles QueryStatus command.
fn handle_query_status(status: &IpcStatusSnapshot) -> IpcResponse {
    IpcResponse::Status(String::from_utf8_lossy(&status.status_payload()).into_owned())
}

/// Handles RequestAcquire command.
///
/// Payload convention: an empty payload asks for automatic interval
/// selection, so the server replies with the automatic sentinel of zero
/// HNS for the ownership layer to resolve. A non empty payload must be
/// the 8 byte little endian interval produced by encode_interval_payload,
/// which is validated with the same bounds helper that encoder enforces.
fn handle_request_acquire(payload: &[u8]) -> IpcResponse {
    if payload.is_empty() {
        return IpcResponse::Acquired { effective_hns: 0 };
    }
    if payload.len() != 8 {
        return IpcResponse::Error(IpcError::PayloadTruncated);
    }
    let interval_hns = u64::from_le_bytes([
        payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
        payload[7],
    ]);
    match tick_ipc::validate_interval_hns(interval_hns) {
        Ok(hns) => IpcResponse::Acquired {
            effective_hns: hns.value(),
        },
        Err(e) => IpcResponse::Error(e),
    }
}

/// Handles RequestRelease command.
#[allow(dead_code)]
fn handle_request_release() -> IpcResponse {
    IpcResponse::Released
}

/// Handles ScheduleAction command.
///
/// Payload convention: encode_schedule_payload produces a 1 byte action
/// tag from ScheduleActionKind followed by a 4 byte little endian delay
/// in seconds. The scheduled action identifier reported back to the
/// client is the delay field.
fn handle_schedule_action(payload: &[u8]) -> IpcResponse {
    if payload.len() != 5 {
        return IpcResponse::Error(IpcError::PayloadTruncated);
    }
    let action_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
    IpcResponse::Scheduled { action_id }
}

/// Handles CancelSchedule command.
#[allow(dead_code)]
fn handle_cancel_schedule(_payload: &[u8]) -> IpcResponse {
    IpcResponse::Cancelled
}

/// Sends a response back to the client in the shared wire format.
fn send_response(pipe: *mut c_void, result: IpcResponse) {
    let payload = encode_response(&result);
    write_pipe(pipe, &payload);
}

/// Sends an error response back to the client in the shared wire format.
fn send_error_response(pipe: *mut c_void, error: IpcError) {
    let payload = encode_response(&IpcResponse::Error(error));
    write_pipe(pipe, &payload);
}

/// Writes data to the pipe.
fn write_pipe(pipe: *mut c_void, data: &[u8]) {
    let mut bytes_written: u32 = 0;
    unsafe {
        WriteFile(
            pipe,
            data.as_ptr(),
            data.len() as u32,
            &mut bytes_written,
            ptr::null_mut(),
        )
    };
    unsafe { FlushFileBuffers(pipe) };
}

/// Disconnects the client from the pipe.
fn disconnect_pipe(pipe: *mut c_void) {
    unsafe { DisconnectNamedPipe(pipe) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_correct() {
        assert_eq!(PIPE_NAME, r"\\.\pipe\TrueTick-Ipc-v1");
    }

    #[test]
    fn sddl_dacl_grants_owner_system_and_administrators_only() {
        assert!(PIPE_SDDL.contains("SY"));
        assert!(PIPE_SDDL.contains("BA"));
        assert!(PIPE_SDDL.contains("OW"));
        assert!(!PIPE_SDDL.contains("AU"));
        assert!(!PIPE_SDDL.contains("RC"));
    }

    #[test]
    fn rate_limiter_allows_max_requests() {
        let mut limiter = PidRateLimiter::new();
        let now = Instant::now();
        for _ in 0..MAX_REQUESTS_PER_SECOND {
            assert!(limiter.check_and_record(1234, now).is_ok());
        }
    }

    #[test]
    fn rate_limiter_rejects_over_limit() {
        let mut limiter = PidRateLimiter::new();
        let now = Instant::now();
        for _ in 0..MAX_REQUESTS_PER_SECOND {
            limiter.check_and_record(1234, now).unwrap();
        }
        assert!(matches!(
            limiter.check_and_record(1234, now),
            Err(IpcError::RateLimitExceeded { pid: 1234 })
        ));
    }

    #[test]
    fn rate_limiter_isolates_pids() {
        let mut limiter = PidRateLimiter::new();
        let now = Instant::now();
        for _ in 0..MAX_REQUESTS_PER_SECOND {
            limiter.check_and_record(1234, now).unwrap();
        }
        // Different PID should still work
        assert!(limiter.check_and_record(5678, now).is_ok());
    }

    #[test]
    fn handle_request_acquire_validates_interval() {
        let payload = 5000u64.to_le_bytes().to_vec();
        match handle_request_acquire(&payload) {
            IpcResponse::Acquired { effective_hns } => {
                assert_eq!(effective_hns, 5000);
            }
            _ => panic!("expected Acquired"),
        }
    }

    #[test]
    fn handle_request_acquire_empty_payload_selects_automatic() {
        match handle_request_acquire(&[]) {
            IpcResponse::Acquired { effective_hns } => {
                assert_eq!(effective_hns, 0);
            }
            _ => panic!("expected Acquired"),
        }
    }

    #[test]
    fn handle_request_acquire_rejects_invalid_interval() {
        let payload = 999999u64.to_le_bytes().to_vec();
        match handle_request_acquire(&payload) {
            IpcResponse::Error(IpcError::IntervalOutOfBounds { requested }) => {
                assert_eq!(requested, 999999);
            }
            _ => panic!("expected IntervalOutOfBounds"),
        }
    }

    #[test]
    fn handle_schedule_action_reads_action_id() {
        let payload = tick_ipc::encode_schedule_payload(tick_ipc::ScheduleActionKind::Start, 42);
        match handle_schedule_action(&payload) {
            IpcResponse::Scheduled { action_id } => {
                assert_eq!(action_id, 42);
            }
            _ => panic!("expected Scheduled"),
        }
    }

    #[test]
    fn dispatch_command_handles_all_verbs() {
        let token = SessionToken::from_bytes([0u8; 32]);
        let status = IpcStatusSnapshot::new();
        let queue = Arc::new(crate::tray::IpcCommandQueue::new());
        let bridge = IpcCommandBridge {
            tray_hwnd: ptr::null_mut(),
            queue,
        };

        // QueryStatus
        let frame = IpcFrame::new(token, CommandVerb::QueryStatus, vec![]).unwrap();
        match dispatch_command(&frame, &status, &bridge) {
            IpcResponse::Status(_) => {}
            _ => panic!("expected Status"),
        }

        // RequestAcquire with an invalid payload is rejected before the
        // UI relay so no window is required.
        let payload = 999999u64.to_le_bytes().to_vec();
        let frame = IpcFrame::new(token, CommandVerb::RequestAcquire, payload).unwrap();
        match dispatch_command(&frame, &status, &bridge) {
            IpcResponse::Error(IpcError::IntervalOutOfBounds { .. }) => {}
            _ => panic!("expected IntervalOutOfBounds"),
        }

        // ScheduleAction with a malformed payload is rejected locally.
        let frame = IpcFrame::new(token, CommandVerb::ScheduleAction, vec![1, 2]).unwrap();
        match dispatch_command(&frame, &status, &bridge) {
            IpcResponse::Error(IpcError::PayloadTruncated) => {}
            _ => panic!("expected PayloadTruncated"),
        }
    }

    #[test]
    fn session_token_validation_works() {
        let token = SessionToken::from_bytes([1u8; 32]);
        let wrong_token = SessionToken::from_bytes([2u8; 32]);

        assert!(token.matches(&token));
        assert!(!token.matches(&wrong_token));
    }

    #[test]
    fn ipc_status_snapshot_serializes_status_payload() {
        let snapshot = IpcStatusSnapshot::new();
        snapshot.update("running", "owned", Some(156250), Some(5000));
        let payload = snapshot.status_payload();
        let text = String::from_utf8(payload).unwrap();
        assert!(text.contains("status=running"));
        assert!(text.contains("ownership=owned"));
        assert!(text.contains("effective_hns=156250"));
        assert!(text.contains("requested_hns=5000"));
    }

    #[test]
    fn ipc_status_snapshot_defaults_to_starting() {
        let snapshot = IpcStatusSnapshot::new();
        let payload = snapshot.status_payload();
        let text = String::from_utf8(payload).unwrap();
        assert!(text.contains("status=starting"));
        assert!(text.contains("ownership=released"));
        assert!(text.contains("effective_hns=unknown"));
        assert!(text.contains("requested_hns=unknown"));
    }

    #[test]
    fn ipc_response_variants_round_trip() {
        for response in [
            IpcResponse::Status("text".to_owned()),
            IpcResponse::Acquired { effective_hns: 0 },
            IpcResponse::Released,
            IpcResponse::Scheduled { action_id: 0 },
            IpcResponse::Cancelled,
            IpcResponse::Error(IpcError::InvalidMagic),
        ] {
            let decoded = tick_ipc::decode_response(&encode_response(&response)).unwrap();
            assert_eq!(decoded, response);
        }
    }
}
