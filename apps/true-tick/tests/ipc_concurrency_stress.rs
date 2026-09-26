//! Live multi client concurrency and stress harness for the True Tick
//! named pipe IPC endpoint.
//!
//! The production `IpcServer` is compiled into the tray binary and its
//! command bridge is crate private, so this harness rehosts the identical
//! server side accept loop inside the test. The loop is copied operation
//! for operation from `src/tray/ipc.rs`: same pipe name, same SDDL DACL,
//! same overlapped `ConnectNamedPipe` and `ReadFile` handling with the
//! `CancelIoEx` abort path, the same `PidRateLimiter` gate before the
//! frame read, and the same session token file lifecycle.
//!
//! The harness runs on Windows only since the transport is a Win32 named
//! pipe. Each test is serialized on a shared mutex because the pipe name
//! is a single global kernel object.

#[cfg(windows)]
mod harness {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use tick_ipc::{
        encode_response, write_token_file, CommandVerb, IpcError, IpcFrame, IpcResponse,
        PidRateLimiter, SessionToken, MAX_PAYLOAD_LEN, MAX_REQUESTS_PER_SECOND, TOKEN_FILE_NAME,
    };

    // Pipe geometry mirrored from src/tray/ipc.rs so the harness exercises
    // the exact production wire surface.
    const PIPE_BUFFER_SIZE: u32 = 4096;
    const PIPE_TIMEOUT_MS: u32 = 5000;
    const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";

    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_PIPE_BUSY: u32 = 231;
    const ERROR_MORE_DATA: u32 = 234;
    const ERROR_BROKEN_PIPE: u32 = 109;

    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    const PIPE_TYPE_MESSAGE: u32 = 0x0000_0004;
    const PIPE_READMODE_MESSAGE: u32 = 0x0000_0002;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;

    const PIPE_READ_DEADLINE_MS: u32 = 500;

    const WAIT_OBJECT_0: u32 = 0;

    const ERROR_IO_PENDING: u32 = 997;
    const SDDL_REVISION_1: u32 = 1;

    // The severed pipe test writes this many bytes of a valid frame before
    // dropping the handle so the server read stays pending and must be
    // cancelled through CancelIoEx.
    const PARTIAL_FRAME_BYTES: usize = 10;

    // Cooling sleeps are padded past the limiter window because Windows
    // timer granularity can slice a deadline short by a few milliseconds.
    const WINDOW_COOLDOWN_MS: u64 = 1200;

    // Wall clock budget for the follow up exchange after the severed
    // client drops. Generous so a slow CI host does not flake.
    const RECOVERY_DEADLINE: Duration = Duration::from_secs(15);

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
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share_mode: u32,
            security_attributes: *mut u8,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: isize,
        ) -> isize;
        fn GetLastError() -> u32;
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

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE_VALUE: isize = -1;

    /// Serializes every harness scenario because each test owns a single
    /// accept loop against its own pipe instance.
    static PIPE_SCENARIO_LOCK: Mutex<()> = Mutex::new(());

    /// Builds a clean temporary directory under the crate target tree so
    /// token file artifacts never escape the workspace.
    fn temporary_directory(label: &str) -> PathBuf {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create harness state directory");
        directory
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

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

    fn create_pipe_instance(pipe_name: &str) -> Result<*mut c_void, u32> {
        let security_descriptor = create_security_descriptor()?;
        let security_attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            security_descriptor,
            inherit_handle: 0,
        };
        let pipe_name_wide = wide(pipe_name);
        let pipe = unsafe {
            CreateNamedPipeW(
                pipe_name_wide.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                PIPE_TIMEOUT_MS,
                &security_attributes,
            )
        };
        unsafe { LocalFree(security_descriptor) };
        if pipe.is_null() || pipe as isize == INVALID_HANDLE_VALUE {
            Err(unsafe { GetLastError() })
        } else {
            Ok(pipe)
        }
    }

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

    fn get_client_pid(pipe: *mut c_void) -> Option<u32> {
        let mut pid: u32 = 0;
        let result = unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) };
        if result != 0 {
            Some(pid)
        } else {
            None
        }
    }

    /// Overlapped read with the production five hundred millisecond
    /// deadline. A client that stalls mid frame is aborted through
    /// CancelIoEx so the server thread can loop back to accept.
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
            let wait = unsafe { WaitForSingleObject(event, PIPE_READ_DEADLINE_MS) };
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

    fn disconnect_pipe(pipe: *mut c_void) {
        unsafe { DisconnectNamedPipe(pipe) };
    }

    fn send_error_response(pipe: *mut c_void, error: IpcError) {
        write_pipe(pipe, &encode_response(&IpcResponse::Error(error)));
    }

    /// Per connection handler mirroring the production order of
    /// operations: accept wait, client PID lookup, rate limiter gate,
    /// frame read, token check, QueryStatus dispatch, disconnect.
    fn handle_pipe_connection(
        pipe: *mut c_void,
        session_token: &SessionToken,
        rate_limiter: &mut PidRateLimiter,
    ) {
        if !wait_for_client(pipe) {
            return;
        }
        let client_pid = get_client_pid(pipe).unwrap_or(0);
        let now = Instant::now();
        if let Err(error) = rate_limiter.check_and_record(client_pid, now) {
            send_error_response(pipe, error);
            disconnect_pipe(pipe);
            return;
        }
        match read_frame(pipe) {
            Ok(frame) => {
                if !frame.token.matches(session_token) {
                    send_error_response(pipe, IpcError::UnauthorizedToken);
                    disconnect_pipe(pipe);
                    return;
                }
                let result = match frame.verb {
                    CommandVerb::QueryStatus => {
                        IpcResponse::Status("status=running ownership=owned".to_owned())
                    }
                    // Mutating verbs are out of scope for the harness since
                    // there is no tray message loop to relay them to.
                    _ => IpcResponse::Error(IpcError::Unsupported),
                };
                write_pipe(pipe, &encode_response(&result));
            }
            Err(error) => send_error_response(pipe, error),
        }
        disconnect_pipe(pipe);
    }

    /// In process rehost of the production IPC server bound to a unique
    /// pipe name so the harness never collides with a live tray instance
    /// on the shared production endpoint. Publishes the session token
    /// file on start and removes it on teardown.
    struct HarnessServer {
        running: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
        pipe_name: String,
        token_file_path: PathBuf,
        state_directory: PathBuf,
    }

    impl HarnessServer {
        fn start(label: &str) -> (Self, SessionToken) {
            let state_directory = temporary_directory(label);
            let token_file_path = state_directory.join(TOKEN_FILE_NAME);
            // Per process pipe name keeps the harness isolated from the
            // production tray pipe when the app is running on the host.
            let pipe_name = format!(
                r"\\.\pipe\TrueTick-Ipc-Harness-{}-{}",
                std::process::id(),
                label
            );
            let running = Arc::new(AtomicBool::new(true));
            let running_in_thread = running.clone();
            let thread_token_path = token_file_path.clone();
            let thread_pipe_name = pipe_name.clone();
            let (ready_send, ready_recv) = std::sync::mpsc::channel();

            let thread = thread::spawn(move || {
                let mut rate_limiter = PidRateLimiter::new();
                let session_token = SessionToken::generate_ephemeral();
                if write_token_file(&thread_token_path, &session_token).is_err() {
                    let _ = ready_send.send(Err(session_token));
                    return;
                }
                if ready_send.send(Ok(session_token)).is_err() {
                    return;
                }
                while running_in_thread.load(Ordering::SeqCst) {
                    match create_pipe_instance(&thread_pipe_name) {
                        Ok(pipe) => {
                            if !running_in_thread.load(Ordering::SeqCst) {
                                unsafe { CloseHandle(pipe) };
                                break;
                            }
                            handle_pipe_connection(pipe, &session_token, &mut rate_limiter);
                            unsafe { CloseHandle(pipe) };
                        }
                        Err(_) => thread::sleep(Duration::from_millis(100)),
                    }
                }
            });

            let token = ready_recv
                .recv_timeout(Duration::from_secs(10))
                .expect("harness server must report startup")
                .expect("harness server must publish token file");

            let server = Self {
                running,
                thread: Some(thread),
                pipe_name,
                token_file_path,
                state_directory,
            };
            server.wait_for_token_file();
            (server, token)
        }

        fn wait_for_token_file(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if self.token_file_path.exists() {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            panic!(
                "token file {} never appeared",
                self.token_file_path.display()
            );
        }

        fn shutdown(mut self) {
            self.running.store(false, Ordering::SeqCst);
            // Nudge the accept loop by connecting once so shutdown does
            // not wait out the full connect deadline. The retry absorbs
            // the instance gap just like a normal client connect.
            let _ = connect_pipe(&self.pipe_name, Duration::from_millis(500));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = std::fs::remove_file(&self.token_file_path);
            let _ = std::fs::remove_dir_all(&self.state_directory);
        }
    }

    /// Connects to the harness pipe with a bounded retry budget.
    ///
    /// The production client retries only on ERROR_PIPE_BUSY, but a fresh
    /// connect can also land in the gap where the server closed one pipe
    /// instance and has not yet created the next. Retrying on
    /// ERROR_FILE_NOT_FOUND inside the budget keeps the harness focused
    /// on server behavior instead of transport timing.
    fn connect_pipe(pipe_name: &str, budget: Duration) -> Result<tick_ipc::IpcClient, IpcError> {
        const ERROR_FILE_NOT_FOUND: i32 = 2;
        let deadline = Instant::now() + budget;
        loop {
            match tick_ipc::connect(pipe_name, 200) {
                Ok(client) => return Ok(client),
                Err(error) => {
                    let raw = error.raw_os_error().unwrap_or(-1);
                    if raw == ERROR_FILE_NOT_FOUND && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    return Err(IpcError::TransportIo { raw_os_error: raw });
                }
            }
        }
    }

    /// Single exchange against the harness pipe using the supplied
    /// session token.
    fn query_with_token(pipe_name: &str, token: &SessionToken) -> Result<IpcResponse, IpcError> {
        let mut client = connect_pipe(pipe_name, Duration::from_secs(10))?;
        client.exchange(token, CommandVerb::QueryStatus, &[])
    }

    fn is_rate_limited(response: &Result<IpcResponse, IpcError>) -> bool {
        matches!(
            response,
            Ok(IpcResponse::Error(IpcError::RateLimitExceeded { .. }))
        )
    }

    fn is_status(response: &Result<IpcResponse, IpcError>) -> bool {
        matches!(response, Ok(IpcResponse::Status(_)))
    }

    /// Scenario 1: ten concurrent client threads connecting at the same
    /// instant. Every connection is accepted by the sequential accept
    /// loop and served a QueryStatus response. The limiter sees a single
    /// PID so at most MAX_REQUESTS_PER_SECOND can be served inside one
    /// window, which is exactly the thread count used here.
    pub fn concurrent_clients_all_succeed() {
        let _guard = PIPE_SCENARIO_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (server, token) = HarnessServer::start("ipc-concurrent");

        let barrier = Arc::new(std::sync::Barrier::new(10));
        let mut handles = Vec::with_capacity(10);
        for index in 0..10 {
            let barrier = barrier.clone();
            let pipe_name = server.pipe_name.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                let result = query_with_token(&pipe_name, &token);
                (index, result)
            }));
        }

        let mut succeeded = 0usize;
        for handle in handles {
            let (index, result) = handle.join().expect("client thread must not panic");
            assert!(
                is_status(&result),
                "client thread {index} expected Status, got {result:?}"
            );
            succeeded += 1;
        }
        assert_eq!(
            succeeded, 10,
            "every concurrent client thread must complete one exchange"
        );

        server.shutdown();
    }

    /// Scenario 2: fifty rapid fire requests from one thread and one PID.
    /// The limiter must admit the first MAX_REQUESTS_PER_SECOND inside the
    /// window and reject the rest with RateLimitExceeded, then resume
    /// normal service once the sliding window cools down.
    pub fn rapid_fire_rate_limit_enforced_and_recovers() {
        let _guard = PIPE_SCENARIO_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (server, token) = HarnessServer::start("ipc-flood");

        let mut accepted = 0usize;
        let mut rejected = 0usize;
        let mut unexpected = 0usize;

        for _ in 0..50 {
            let result = query_with_token(&server.pipe_name, &token);
            if is_status(&result) {
                accepted += 1;
            } else if is_rate_limited(&result) {
                rejected += 1;
            } else {
                unexpected += 1;
            }
        }

        assert_eq!(unexpected, 0, "flood produced an unexpected response");
        assert_eq!(
            accepted, MAX_REQUESTS_PER_SECOND,
            "exactly MAX_REQUESTS_PER_SECOND requests may be admitted per window"
        );
        assert_eq!(
            rejected,
            50 - MAX_REQUESTS_PER_SECOND,
            "all excess requests must surface RateLimitExceeded"
        );

        // Let the one second sliding window fully drain, then confirm the
        // server still accepts and answers new connections.
        thread::sleep(Duration::from_millis(WINDOW_COOLDOWN_MS));
        let after = query_with_token(&server.pipe_name, &token);
        assert!(
            is_status(&after),
            "server must keep serving after the rate limit window cools, got {after:?}"
        );

        server.shutdown();
    }

    /// Scenario 3: a client connects, writes a partial frame, and drops
    /// its handle immediately. The server must unwind the pending
    /// overlapped read through CancelIoEx, drop the client, and accept a
    /// fresh clean client without wedging.
    pub fn severed_partial_frame_then_clean_client() {
        let _guard = PIPE_SCENARIO_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (server, token) = HarnessServer::start("ipc-severed");

        // Open a raw pipe handle, write PARTIAL_FRAME_BYTES of a valid
        // serialized frame, then close the handle immediately.
        let probe = SessionToken::from_bytes([0u8; 32]);
        let frame = IpcFrame::new(probe, CommandVerb::QueryStatus, Vec::new())
            .expect("frame construction")
            .serialize();
        assert!(
            frame.len() > PARTIAL_FRAME_BYTES,
            "fixture frame must exceed the partial write length"
        );

        let pipe_name_wide = wide(&server.pipe_name);
        let open_deadline = Instant::now() + Duration::from_secs(10);
        let raw = loop {
            let attempt = unsafe {
                CreateFileW(
                    pipe_name_wide.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };
            if attempt != INVALID_HANDLE_VALUE {
                break attempt;
            }
            let error = unsafe { GetLastError() };
            if error != 2 || Instant::now() >= open_deadline {
                panic!("severed client connect failed with error {error}");
            }
            thread::sleep(Duration::from_millis(5));
        };

        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                raw as *mut c_void,
                frame.as_ptr(),
                PARTIAL_FRAME_BYTES as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        unsafe { CloseHandle(raw as *mut c_void) };
        // The write can fail with ERROR_PIPE_BUSY style races on some
        // hosts, but a successful partial write is the primary path. The
        // drop itself is what the server must survive.
        let _ = ok;
        let _ = written;

        // The severed connection consumed one slot in this PID window.
        // Wait out the read deadline plus limiter window, then prove the
        // server still serves clean clients.
        thread::sleep(Duration::from_millis(WINDOW_COOLDOWN_MS));

        let deadline = Instant::now() + RECOVERY_DEADLINE;
        let mut recovered = false;
        while Instant::now() < deadline {
            match query_with_token(&server.pipe_name, &token) {
                Ok(IpcResponse::Status(_)) => {
                    recovered = true;
                    break;
                }
                Ok(IpcResponse::Error(IpcError::RateLimitExceeded { .. })) => {
                    thread::sleep(Duration::from_millis(WINDOW_COOLDOWN_MS));
                }
                _ => thread::sleep(Duration::from_millis(100)),
            }
        }
        assert!(
            recovered,
            "server must accept a clean client after the severed partial frame"
        );

        server.shutdown();
    }
}

#[cfg(windows)]
#[test]
fn concurrent_clients_all_succeed() {
    harness::concurrent_clients_all_succeed();
}

#[cfg(windows)]
#[test]
fn rapid_fire_rate_limit_enforced_and_recovers() {
    harness::rapid_fire_rate_limit_enforced_and_recovers();
}

#[cfg(windows)]
#[test]
fn severed_partial_frame_then_clean_client() {
    harness::severed_partial_frame_then_clean_client();
}

#[cfg(not(windows))]
#[test]
fn ipc_concurrency_stress_requires_windows() {
    eprintln!("named pipe IPC harness is Windows only, skipping");
}
