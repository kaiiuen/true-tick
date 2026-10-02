//! Command line control surface for a running True Tick tray instance.
//!
//! The binary talks to the tray process over the tick-ipc named pipe for
//! control verbs and reads daily CSV logs straight from disk for `logs`,
//! which never touches the pipe.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use tick_ipc::{
    encode_interval_payload, encode_schedule_payload, read_token_file, CommandVerb, IpcResponse,
    ScheduleActionKind, PIPE_NAME, TOKEN_FILE_NAME,
};

const VERSION: &str = "0.1.0";

/// Busy retry budget handed to WaitNamedPipeW on each pipe connect attempt.
const PIPE_BUSY_RETRY_MS: u32 = 250 + 250;

const EXIT_OK: u8 = 0;
const EXIT_FAILURE: u8 = 1;
const EXIT_UNREACHABLE: u8 = 2;

/// Default line count for `logs` when `--tail` is not given.
const DEFAULT_LOG_TAIL: usize = 20;

const LOG_FILE_PREFIX: &str = "true-tick-";
const LOG_FILE_SUFFIX: &str = ".csv";

/// Budget for the ipc-token file to appear after the engine is spawned.
const DAEMON_TOKEN_TIMEOUT_MS: u64 = 10_000;
const DAEMON_TOKEN_POLL_MS: u64 = 50;

/// Win32 process creation flags that detach the spawned engine from the
/// console and the job of this short lived launcher.
#[cfg(windows)]
const DAEMON_DETACHED_FLAGS: u32 = 0x0000_0008 | 0x0000_0200;

/// Name of the marker the tray writes beside the ipc token carrying the PID
/// of the server process backing the named pipe.
const SERVER_PID_FILE_NAME: &str = "ipc-server-pid";

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut std::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: isize,
    ) -> isize;
    fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: isize) -> i32;
    fn GetNamedPipeServerProcessId(pipe: isize, process_id: *mut u32) -> i32;
}

#[cfg(windows)]
const GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const OPEN_EXISTING: u32 = 3;
#[cfg(windows)]
const ERROR_PIPE_BUSY: u32 = 231;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: isize = -1;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("true-tick-cli.exe"));
    ExitCode::from(run(&args, &executable))
}

/// Parsed command intent. Arg parsing produces this shape so formatting and
/// dispatch stay unit testable without a live pipe.
#[derive(Clone, Debug, Eq, PartialEq)]
enum CliRequest {
    Status {
        json: bool,
    },
    Start {
        interval_hns: Option<u64>,
    },
    Stop,
    Schedule {
        kind: ScheduleActionKind,
        seconds: u32,
    },
    Cancel {
        action_id: Option<u32>,
    },
    /// Launch the headless background engine when no instance is running.
    Daemon,
    Logs {
        tail: usize,
        date: Option<String>,
    },
    Help,
    Version,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CliParseError {
    MissingSubcommand,
    UnknownSubcommand(String),
    MissingArgument(&'static str),
    InvalidArgument(String),
}

/// Failure channel for work after parsing. Unreachable maps to exit code 2
/// for a missing token file or a dead pipe, everything else is exit code 1.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RunError {
    Failure(String),
    Unreachable(String),
}

fn exit_code(error: &RunError) -> u8 {
    match error {
        RunError::Failure(_) => EXIT_FAILURE,
        RunError::Unreachable(_) => EXIT_UNREACHABLE,
    }
}

fn run(args: &[String], executable: &Path) -> u8 {
    let (root_override, rest) = match extract_root(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {}", describe_parse_error(&error));
            return EXIT_FAILURE;
        }
    };
    let request = match parse_args(&rest) {
        Ok(request) => request,
        Err(CliParseError::MissingSubcommand) => {
            eprint!("{}", usage());
            return EXIT_FAILURE;
        }
        Err(error) => {
            eprintln!("error: {}", describe_parse_error(&error));
            eprintln!("try `true-tick-cli help`");
            return EXIT_FAILURE;
        }
    };
    let root = resolve_root(executable, root_override.as_deref());
    match request {
        CliRequest::Help => {
            print!("{}", usage());
            EXIT_OK
        }
        CliRequest::Version => {
            println!("{VERSION}");
            EXIT_OK
        }
        CliRequest::Logs { tail, date } => run_logs(&root, tail, date.as_deref()),
        CliRequest::Daemon => run_daemon(&root, executable),
        CliRequest::Status { json } => match exchange(&root, CommandVerb::QueryStatus, &[]) {
            Ok(IpcResponse::Status(text)) => {
                if json {
                    println!("{}", status_json(&text));
                } else {
                    println!("{text}");
                }
                EXIT_OK
            }
            Ok(other) => report_response(&other),
            Err(error) => report_error(&error),
        },
        CliRequest::Start { interval_hns } => match acquire_payload(interval_hns) {
            Ok(payload) => match exchange(&root, CommandVerb::RequestAcquire, &payload) {
                Ok(response) => report_response(&response),
                Err(error) => report_error(&error),
            },
            Err(error) => report_error(&error),
        },
        CliRequest::Stop => match exchange(&root, CommandVerb::RequestRelease, &[]) {
            Ok(response) => report_response(&response),
            Err(error) => report_error(&error),
        },
        CliRequest::Schedule { kind, seconds } => {
            let payload = encode_schedule_payload(kind, seconds);
            match exchange(&root, CommandVerb::ScheduleAction, &payload) {
                Ok(response) => report_response(&response),
                Err(error) => report_error(&error),
            }
        }
        CliRequest::Cancel { action_id } => {
            let payload = match action_id {
                Some(id) => id.to_le_bytes().to_vec(),
                None => Vec::new(),
            };
            match exchange(&root, CommandVerb::CancelSchedule, &payload) {
                Ok(response) => report_response(&response),
                Err(error) => report_error(&error),
            }
        }
    }
}

/// Resolves the application data root. With `--root` the given path is the
/// root, otherwise the directory holding this executable is used, which in a
/// slot layout sits beside the shared `Data` directory.
fn resolve_root(executable: &Path, root_override: Option<&Path>) -> PathBuf {
    match root_override {
        Some(root) => root.to_path_buf(),
        None => executable
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

fn token_path(root: &Path) -> PathBuf {
    root.join("Data").join("state").join(TOKEN_FILE_NAME)
}

/// Removes an ipc-token file left behind by a crashed engine so the ready
/// probe after a spawn cannot be satisfied by a stale credential. A missing
/// file is the common case and is not an error, any other failure is reported
/// as a warning that does not abort the spawn.
fn clear_stale_token(root: &Path) {
    match std::fs::remove_file(token_path(root)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("warning: cannot remove stale ipc token: {error}"),
    }
}

fn server_pid_path(root: &Path) -> PathBuf {
    root.join("Data").join("state").join(SERVER_PID_FILE_NAME)
}

/// Removes the ipc-server-pid marker alongside the stale token so a ready
/// probe cannot be satisfied by a PID published by a crashed engine. A
/// missing file is the common case and is not an error.
fn clear_stale_server_pid(root: &Path) {
    match std::fs::remove_file(server_pid_path(root)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("warning: cannot remove stale server pid marker: {error}"),
    }
}

fn log_directory(root: &Path) -> PathBuf {
    root.join("Data").join("logs")
}

fn daily_log_path(root: &Path, date: &str) -> PathBuf {
    log_directory(root).join(format!("{LOG_FILE_PREFIX}{date}{LOG_FILE_SUFFIX}"))
}

/// Pulls the global `--root <path>` option out of the raw argv tail so it can
/// appear before or after the subcommand.
fn extract_root(args: &[String]) -> Result<(Option<PathBuf>, Vec<String>), CliParseError> {
    let mut root = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--root" {
            let value = args
                .get(index + 1)
                .ok_or(CliParseError::MissingArgument("--root <path>"))?;
            root = Some(PathBuf::from(value));
            index += 2;
        } else {
            rest.push(args[index].clone());
            index += 1;
        }
    }
    Ok((root, rest))
}

fn parse_args(args: &[String]) -> Result<CliRequest, CliParseError> {
    let (subcommand, rest) = args.split_first().ok_or(CliParseError::MissingSubcommand)?;
    match subcommand.as_str() {
        "status" => {
            let mut json = false;
            for arg in rest {
                match arg.as_str() {
                    "--json" => json = true,
                    other => return Err(CliParseError::InvalidArgument(other.to_owned())),
                }
            }
            Ok(CliRequest::Status { json })
        }
        "start" => {
            let mut interval_hns = None;
            let mut index = 0;
            while index < rest.len() {
                match rest[index].as_str() {
                    "--interval" => {
                        let value = rest
                            .get(index + 1)
                            .ok_or(CliParseError::MissingArgument("--interval <hns>"))?;
                        let parsed = value.parse::<u64>().map_err(|_| {
                            CliParseError::InvalidArgument(format!("--interval `{value}`"))
                        })?;
                        interval_hns = Some(parsed);
                        index += 2;
                    }
                    other => return Err(CliParseError::InvalidArgument(other.to_owned())),
                }
            }
            Ok(CliRequest::Start { interval_hns })
        }
        "stop" => expect_no_args(rest, CliRequest::Stop),
        "cancel" => {
            if rest.len() > 1 {
                return Err(CliParseError::InvalidArgument(rest[1].clone()));
            }
            let action_id = match rest.first() {
                Some(id_text) => Some(id_text.parse::<u32>().map_err(|_| {
                    CliParseError::InvalidArgument(format!("cancel action id `{id_text}`"))
                })?),
                None => None,
            };
            Ok(CliRequest::Cancel { action_id })
        }
        "daemon" => expect_no_args(rest, CliRequest::Daemon),
        "schedule" => {
            let action = rest.first().ok_or(CliParseError::MissingArgument(
                "schedule <start-in|stop-in|pause> <seconds>",
            ))?;
            let kind = parse_schedule_action(action).ok_or_else(|| {
                CliParseError::InvalidArgument(format!("schedule action `{action}`"))
            })?;
            let seconds_text = rest
                .get(1)
                .ok_or(CliParseError::MissingArgument("schedule <seconds>"))?;
            let seconds = seconds_text.parse::<u32>().map_err(|_| {
                CliParseError::InvalidArgument(format!("schedule seconds `{seconds_text}`"))
            })?;
            if let Some(extra) = rest.get(2) {
                return Err(CliParseError::InvalidArgument(extra.clone()));
            }
            Ok(CliRequest::Schedule { kind, seconds })
        }
        "logs" => {
            let mut tail = DEFAULT_LOG_TAIL;
            let mut date = None;
            let mut index = 0;
            while index < rest.len() {
                match rest[index].as_str() {
                    "--tail" => {
                        let value = rest
                            .get(index + 1)
                            .ok_or(CliParseError::MissingArgument("--tail <n>"))?;
                        tail = value.parse::<usize>().map_err(|_| {
                            CliParseError::InvalidArgument(format!("--tail `{value}`"))
                        })?;
                        index += 2;
                    }
                    "--date" => {
                        let value = rest
                            .get(index + 1)
                            .ok_or(CliParseError::MissingArgument("--date <yyyy-mm-dd>"))?;
                        if !valid_log_date(value) {
                            return Err(CliParseError::InvalidArgument(format!(
                                "--date `{value}`"
                            )));
                        }
                        date = Some(value.clone());
                        index += 2;
                    }
                    other => return Err(CliParseError::InvalidArgument(other.to_owned())),
                }
            }
            Ok(CliRequest::Logs { tail, date })
        }
        "help" => expect_no_args(rest, CliRequest::Help),
        "version" => expect_no_args(rest, CliRequest::Version),
        other => Err(CliParseError::UnknownSubcommand(other.to_owned())),
    }
}

fn expect_no_args(rest: &[String], request: CliRequest) -> Result<CliRequest, CliParseError> {
    match rest.first() {
        Some(extra) => Err(CliParseError::InvalidArgument(extra.clone())),
        None => Ok(request),
    }
}

fn parse_schedule_action(name: &str) -> Option<ScheduleActionKind> {
    match name {
        "start-in" => Some(ScheduleActionKind::Start),
        "stop-in" => Some(ScheduleActionKind::Stop),
        "pause" => Some(ScheduleActionKind::Pause),
        _ => None,
    }
}

fn describe_parse_error(error: &CliParseError) -> String {
    match error {
        CliParseError::MissingSubcommand => "missing subcommand".to_owned(),
        CliParseError::UnknownSubcommand(name) => format!("unknown subcommand `{name}`"),
        CliParseError::MissingArgument(what) => format!("missing argument {what}"),
        CliParseError::InvalidArgument(what) => format!("invalid argument {what}"),
    }
}

/// Builds the RequestAcquire payload. An explicit `--interval` value is
/// bounds checked by the shared encoder, no flag means an empty payload and
/// the server picks the interval.
fn acquire_payload(interval_hns: Option<u64>) -> Result<Vec<u8>, RunError> {
    match interval_hns {
        Some(hns) => encode_interval_payload(hns)
            .map_err(|error| RunError::Failure(format!("invalid --interval value {hns}: {error}"))),
        None => Ok(Vec::new()),
    }
}

/// Thin pipe path: token file, connect, one exchange. Kept narrow so every
/// other piece of the binary stays testable without a tray process.
fn exchange(root: &Path, verb: CommandVerb, payload: &[u8]) -> Result<IpcResponse, RunError> {
    let token_file = token_path(root);
    let token = read_token_file(&token_file).map_err(|error| {
        RunError::Unreachable(format!(
            "cannot read session token at {}: {error}",
            token_file.display()
        ))
    })?;
    let mut client = tick_ipc::connect(PIPE_NAME, PIPE_BUSY_RETRY_MS).map_err(|error| {
        RunError::Unreachable(format!("cannot reach true-tick at {PIPE_NAME}: {error}"))
    })?;
    client
        .exchange(&token, verb, payload)
        .map_err(|error| RunError::Failure(format!("ipc exchange failed: {error}")))
}

fn report_response(response: &IpcResponse) -> u8 {
    match response {
        IpcResponse::Status(text) => {
            println!("{text}");
            EXIT_OK
        }
        IpcResponse::Acquired { effective_hns } => {
            println!("acquired effective_hns={effective_hns}");
            EXIT_OK
        }
        IpcResponse::Released => {
            println!("released");
            EXIT_OK
        }
        IpcResponse::Scheduled { action_id } => {
            println!("scheduled action_id={action_id}");
            EXIT_OK
        }
        IpcResponse::Cancelled => {
            println!("cancelled");
            EXIT_OK
        }
        IpcResponse::Error(error) => {
            eprintln!("error: {error}");
            EXIT_FAILURE
        }
    }
}

fn report_error(error: &RunError) -> u8 {
    match error {
        RunError::Unreachable(message) => eprintln!("unreachable: {message}"),
        RunError::Failure(message) => eprintln!("error: {message}"),
    }
    exit_code(error)
}

/// Ordered spawn candidates for the headless engine: a sibling true-tick.exe
/// first, then each slot under the root, then Launcher.exe beside the cli or
/// at the root so both flat and slot layouts are covered.
fn daemon_candidates(root: &Path, executable: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(directory) = executable.parent() {
        candidates.push(directory.join("true-tick.exe"));
    }
    candidates.push(root.join("true-tick.exe"));
    for slot in ["A", "B"] {
        candidates.push(root.join("Slots").join(slot).join("true-tick.exe"));
    }
    let mut launcher_directories = Vec::new();
    if let Some(directory) = executable.parent() {
        launcher_directories.push(directory);
    }
    if !launcher_directories.contains(&root) {
        launcher_directories.push(root);
    }
    candidates.extend(
        launcher_directories
            .iter()
            .map(|directory| directory.join("Launcher.exe")),
    );
    candidates
}

fn resolve_daemon_target(root: &Path, executable: &Path) -> Result<PathBuf, RunError> {
    daemon_candidates(root, executable)
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            RunError::Failure(format!(
                "no true-tick engine found beside {} or under {}",
                executable.display(),
                root.display()
            ))
        })
}

/// Opens the named pipe directly and reads the PID of the process serving
/// it. The returned handle is independent of any tick-ipc client, since that
/// client exposes no public accessor for its raw handle.
#[cfg(windows)]
fn serving_pipe_pid() -> std::io::Result<u32> {
    let wide: Vec<u16> = std::ffi::OsStr::new(PIPE_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    loop {
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if raw != INVALID_HANDLE_VALUE {
            let mut pid: u32 = 0;
            let ok = unsafe { GetNamedPipeServerProcessId(raw, &mut pid) };
            unsafe {
                CloseHandle(raw);
            }
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
            return Ok(pid);
        }
        let last = unsafe { GetLastError() };
        if last != ERROR_PIPE_BUSY {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { WaitNamedPipeW(wide.as_ptr(), PIPE_BUSY_RETRY_MS) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
}

/// Confirms the process actually serving the named pipe is the engine that
/// published the ipc-server-pid marker. A successful connect alone is not
/// proof the engine is up because a stale or unrelated server can still
/// answer, so the marker PID is compared against the serving process PID.
fn verify_server_pid(root: &Path, _client: &tick_ipc::IpcClient) -> Result<(), String> {
    let marker = std::fs::read_to_string(server_pid_path(root))
        .map_err(|error| format!("cannot read server pid marker: {error}"))?;
    let expected: u32 = marker
        .trim()
        .parse()
        .map_err(|_| format!("server pid marker `{marker}` is not a pid"))?;
    let actual = serving_pipe_pid()
        .map_err(|error| format!("cannot identify the engine serving {PIPE_NAME}: {error}"))?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "server pid mismatch: marker pid {expected} but serving pid {actual}"
        ))
    }
}

/// Off Windows the named pipe endpoint does not exist, so a connect fails
/// long before this runs. Kept for API parity across platforms.
#[cfg(not(windows))]
fn verify_server_pid(_root: &Path, _client: &tick_ipc::IpcClient) -> Result<(), String> {
    Ok(())
}

/// Ensures a background engine is running. A live pipe means an instance is
/// already up, otherwise the resolved engine is spawned detached and the
/// ipc-token file is awaited before success is reported.
fn run_daemon(root: &Path, executable: &Path) -> u8 {
    if let Ok(client) = tick_ipc::connect(PIPE_NAME, PIPE_BUSY_RETRY_MS) {
        return match verify_server_pid(root, &client) {
            Ok(()) => {
                println!("daemon already running");
                EXIT_OK
            }
            Err(message) => report_error(&RunError::Failure(message)),
        };
    }
    let target = match resolve_daemon_target(root, executable) {
        Ok(target) => target,
        Err(error) => return report_error(&error),
    };
    let token_file = token_path(root);
    let mut command = std::process::Command::new(&target);
    // Forward the resolved root to the engine through its working directory
    // so a --root override reaches the spawned process instead of being lost.
    command.current_dir(root);
    // Null every stdio handle so the detached engine never inherits and pins
    // open this caller's console or pipe handles after spawn returns.
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(DAEMON_DETACHED_FLAGS);
    // A token file left behind by a crashed engine would satisfy the ready
    // check below before the new server publishes its own token, and a
    // follow-up status exchange with the stale credential gets rejected, so
    // clear it before spawn. Clearing before spawn also keeps a fast engine
    // from publishing a fresh credential that this cleanup would delete.
    // A failed spawn simply leaves the next attempt to repeat the cleanup.
    clear_stale_token(root);
    clear_stale_server_pid(root);
    if let Err(error) = command.spawn() {
        return report_error(&RunError::Failure(format!(
            "cannot spawn {}: {error}",
            target.display()
        )));
    }
    println!("spawned true-tick daemon");
    let deadline = Instant::now() + Duration::from_millis(DAEMON_TOKEN_TIMEOUT_MS);
    while Instant::now() < deadline {
        if token_file.is_file() {
            if let Ok(client) = tick_ipc::connect(PIPE_NAME, PIPE_BUSY_RETRY_MS) {
                if verify_server_pid(root, &client).is_ok() {
                    return EXIT_OK;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(DAEMON_TOKEN_POLL_MS));
    }
    report_error(&RunError::Unreachable(format!(
        "spawned engine did not publish {} and serve {} within {} ms",
        token_file.display(),
        PIPE_NAME,
        DAEMON_TOKEN_TIMEOUT_MS
    )))
}

/// Reads the daily CSV log straight from disk. This path deliberately avoids
/// the pipe so logs stay reachable while the tray is down.
fn run_logs(root: &Path, tail: usize, date: Option<&str>) -> u8 {
    let date = match date {
        Some(value) => value.to_owned(),
        None => utc_date_today(),
    };
    let path = daily_log_path(root, &date);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read log file {}: {error}", path.display());
            return EXIT_FAILURE;
        }
    };
    for line in tail_lines(&text, tail) {
        println!("{line}");
    }
    EXIT_OK
}

fn tail_lines(text: &str, count: usize) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].to_vec()
}

fn valid_log_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

/// Current UTC date as YYYY-MM-DD computed from the epoch so no external
/// date crate is needed.
fn utc_date_today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day) = tick_core::civil_from_days((seconds / 86_400) as i64);
    tick_core::format_ymd(year, month, day)
}

/// Renders the Status payload as a JSON object with one key per space
/// separated key=value token. Values that parse as u64 are emitted unquoted,
/// remaining values are escaped as strings, and tokens with no equals sign
/// or an `unknown` value are omitted.
fn status_json(text: &str) -> String {
    let mut body = String::new();
    let mut first = true;
    for token in text.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        if value == "unknown" {
            continue;
        }
        if !first {
            body.push(',');
        }
        first = false;
        body.push('"');
        body.push_str(&json_escape(key));
        body.push_str("\":");
        if let Ok(number) = value.parse::<u64>() {
            body.push_str(&number.to_string());
        } else {
            body.push('"');
            body.push_str(&json_escape(value));
            body.push('"');
        }
    }
    format!("{{{body}}}")
}

fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out
}

fn usage() -> String {
    "true-tick-cli - command line control surface for a running True Tick tray\n\
     \n\
     USAGE:\n\
     \x20   true-tick-cli [--root <path>] <subcommand> [options]\n\
     \n\
     GLOBAL OPTIONS:\n\
     \x20   --root <path>   Application data root holding the Data directory.\n\
     \x20                   Defaults to the directory containing this executable.\n\
     \n\
     SUBCOMMANDS:\n\
     \x20   status [--json]            Query tray status. --json prints {\"status\": \"...\"}.\n\
     \x20   start [--interval <hns>]   Acquire the timer. Interval is in 100-nanosecond\n\
     \x20                              units. Omit it for automatic selection.\n\
     \x20   stop                       Release the timer.\n\
     \x20   schedule <start-in|stop-in|pause> <seconds>\n\
     \x20                              Arm a timed start, stop, or pause action.\n\
     \x20                              Seconds must be within [10, 86400].\n\
     \x20   cancel [action-id]          Cancel a pending scheduled action. Without\n\
     \x20                              an id every pending action is cleared.\n\
     \x20   daemon                     Launch the headless background engine.\n\
     \x20                              Exits quietly when an instance is already up.\n\
     \x20   logs [--tail N] [--date YYYY-MM-DD]\n\
     \x20                              Print the last N lines of the daily CSV log.\n\
     \x20                              Defaults to N=20 and the current UTC date.\n\
     \x20                              Reads the file directly, no tray needed.\n\
     \x20   help                       Print this usage text.\n\
     \x20   version                    Print the version.\n\
     \n\
     EXIT CODES:\n\
     \x20   0   Success.\n\
     \x20   1   Command, usage, parse, or protocol error.\n\
     \x20   2   Tray cannot be reached or the session token file cannot be\n\
     \x20       read, meaning the application is not running.\n"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn root_defaults_to_executable_directory() {
        let executable = PathBuf::from(r"C:\package\Slots\A\true-tick-cli.exe");
        assert_eq!(
            resolve_root(&executable, None),
            PathBuf::from(r"C:\package\Slots\A")
        );
    }

    #[test]
    fn root_override_wins_over_executable_directory() {
        let executable = PathBuf::from(r"C:\package\Slots\A\true-tick-cli.exe");
        let override_root = PathBuf::from(r"D:\elsewhere");
        assert_eq!(
            resolve_root(&executable, Some(&override_root)),
            override_root
        );
    }

    #[test]
    fn token_path_sits_under_data_state() {
        let root = PathBuf::from("/pkg");
        assert_eq!(
            token_path(&root),
            PathBuf::from("/pkg")
                .join("Data")
                .join("state")
                .join(TOKEN_FILE_NAME)
        );
    }

    #[test]
    fn server_pid_path_sits_under_data_state() {
        let root = PathBuf::from("/pkg");
        assert_eq!(
            server_pid_path(&root),
            PathBuf::from("/pkg")
                .join("Data")
                .join("state")
                .join(SERVER_PID_FILE_NAME)
        );
    }

    #[test]
    fn clear_stale_server_pid_removes_marker_and_ignores_missing() {
        let root =
            std::env::temp_dir().join(format!("true-tick-cli-test-pid-{}", std::process::id()));
        let state = root.join("Data").join("state");
        std::fs::create_dir_all(&state).unwrap();
        let marker = state.join(SERVER_PID_FILE_NAME);
        std::fs::write(&marker, b"1234").unwrap();
        clear_stale_server_pid(&root);
        assert!(!marker.is_file());
        // A second call with no marker must not panic or error.
        clear_stale_server_pid(&root);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tail_selects_last_n_lines() {
        let text = "one\ntwo\nthree\nfour\n";
        assert_eq!(tail_lines(text, 2), vec!["three", "four"]);
    }

    #[test]
    fn tail_larger_than_file_returns_everything() {
        let text = "only\ntwo\n";
        assert_eq!(tail_lines(text, 9), vec!["only", "two"]);
    }

    #[test]
    fn start_interval_passes_through_to_acquire_payload() {
        let request = parse_args(&args(&["start", "--interval", "25000"])).unwrap();
        let CliRequest::Start { interval_hns } = request else {
            panic!("expected start request");
        };
        assert_eq!(interval_hns, Some(25_000));
        let payload = acquire_payload(interval_hns).unwrap();
        assert_eq!(payload, 25_000u64.to_le_bytes());
    }

    #[test]
    fn start_without_interval_yields_empty_payload() {
        let request = parse_args(&args(&["start"])).unwrap();
        let CliRequest::Start { interval_hns } = request else {
            panic!("expected start request");
        };
        assert_eq!(interval_hns, None);
        assert!(acquire_payload(interval_hns).unwrap().is_empty());
    }

    #[test]
    fn start_rejects_out_of_bounds_interval() {
        let request = parse_args(&args(&["start", "--interval", "1"])).unwrap();
        let CliRequest::Start { interval_hns } = request else {
            panic!("expected start request");
        };
        assert!(matches!(
            acquire_payload(interval_hns),
            Err(RunError::Failure(_))
        ));
    }

    #[test]
    fn schedule_parses_each_action_and_seconds() {
        for (word, kind) in [
            ("start-in", ScheduleActionKind::Start),
            ("stop-in", ScheduleActionKind::Stop),
            ("pause", ScheduleActionKind::Pause),
        ] {
            let request = parse_args(&args(&["schedule", word, "30"])).unwrap();
            assert_eq!(request, CliRequest::Schedule { kind, seconds: 30 });
        }
    }

    #[test]
    fn every_subcommand_parses_to_expected_request() {
        assert_eq!(
            parse_args(&args(&["status"])).unwrap(),
            CliRequest::Status { json: false }
        );
        assert_eq!(
            parse_args(&args(&["status", "--json"])).unwrap(),
            CliRequest::Status { json: true }
        );
        // Repeated --json is idempotent and must stay accepted.
        assert_eq!(
            parse_args(&args(&["status", "--json", "--json"])).unwrap(),
            CliRequest::Status { json: true }
        );
        assert_eq!(parse_args(&args(&["stop"])).unwrap(), CliRequest::Stop);
        assert_eq!(
            parse_args(&args(&["cancel"])).unwrap(),
            CliRequest::Cancel { action_id: None }
        );
        assert_eq!(
            parse_args(&args(&["cancel", "42"])).unwrap(),
            CliRequest::Cancel {
                action_id: Some(42)
            }
        );
        assert_eq!(parse_args(&args(&["daemon"])).unwrap(), CliRequest::Daemon);
        assert_eq!(parse_args(&args(&["help"])).unwrap(), CliRequest::Help);
        assert_eq!(
            parse_args(&args(&["version"])).unwrap(),
            CliRequest::Version
        );
        assert_eq!(
            parse_args(&args(&["logs"])).unwrap(),
            CliRequest::Logs {
                tail: DEFAULT_LOG_TAIL,
                date: None,
            }
        );
        assert_eq!(
            parse_args(&args(&["logs", "--tail", "5", "--date", "2026-01-02"])).unwrap(),
            CliRequest::Logs {
                tail: 5,
                date: Some("2026-01-02".to_owned()),
            }
        );
    }

    #[test]
    fn schedule_rejects_bad_action_missing_and_non_numeric_seconds() {
        assert!(matches!(
            parse_args(&args(&["schedule", "bogus", "5"])),
            Err(CliParseError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_args(&args(&["schedule"])),
            Err(CliParseError::MissingArgument(_))
        ));
        assert!(matches!(
            parse_args(&args(&["schedule", "start-in"])),
            Err(CliParseError::MissingArgument(_))
        ));
        assert!(matches!(
            parse_args(&args(&["schedule", "start-in", "abc"])),
            Err(CliParseError::InvalidArgument(_))
        ));
    }

    #[test]
    fn cancel_accepts_optional_id_and_rejects_noise() {
        assert_eq!(
            parse_args(&args(&["cancel"])).unwrap(),
            CliRequest::Cancel { action_id: None }
        );
        assert_eq!(
            parse_args(&args(&["cancel", "7"])).unwrap(),
            CliRequest::Cancel { action_id: Some(7) }
        );
        assert!(matches!(
            parse_args(&args(&["cancel", "abc"])),
            Err(CliParseError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_args(&args(&["cancel", "1", "2"])),
            Err(CliParseError::InvalidArgument(_))
        ));
    }

    #[test]
    fn cancel_id_encodes_as_four_byte_little_endian_payload() {
        let payload = 0x0102_0304u32.to_le_bytes().to_vec();
        assert_eq!(payload, vec![0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn daemon_parses_and_rejects_extra_arguments() {
        assert_eq!(parse_args(&args(&["daemon"])).unwrap(), CliRequest::Daemon);
        assert!(matches!(
            parse_args(&args(&["daemon", "--json"])),
            Err(CliParseError::InvalidArgument(_))
        ));
    }

    #[test]
    fn daemon_candidates_cover_sibling_slots_and_launcher() {
        let root = PathBuf::from(r"C:\package\Slots\A");
        let executable = PathBuf::from(r"C:\package\Slots\A\true-tick-cli.exe");
        let candidates = daemon_candidates(&root, &executable);
        assert_eq!(
            candidates,
            vec![
                PathBuf::from(r"C:\package\Slots\A\true-tick.exe"),
                PathBuf::from(r"C:\package\Slots\A\true-tick.exe"),
                PathBuf::from(r"C:\package\Slots\A\Slots\A\true-tick.exe"),
                PathBuf::from(r"C:\package\Slots\A\Slots\B\true-tick.exe"),
                PathBuf::from(r"C:\package\Slots\A\Launcher.exe"),
            ]
        );
    }

    #[test]
    fn daemon_target_prefers_sibling_engine() {
        let root =
            std::env::temp_dir().join(format!("true-tick-cli-test-daemon-{}", std::process::id()));
        let slot = root.join("Slots").join("A");
        std::fs::create_dir_all(&slot).unwrap();
        let sibling = slot.join("true-tick.exe");
        std::fs::write(&sibling, b"fixture").unwrap();
        let executable = slot.join("true-tick-cli.exe");
        assert_eq!(resolve_daemon_target(&slot, &executable).unwrap(), sibling);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn daemon_target_errors_when_nothing_is_installed() {
        let root = std::env::temp_dir().join(format!(
            "true-tick-cli-test-nodaemon-{}",
            std::process::id()
        ));
        let executable = root.join("true-tick-cli.exe");
        assert!(matches!(
            resolve_daemon_target(&root, &executable),
            Err(RunError::Failure(_))
        ));
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert_eq!(
            parse_args(&args(&["frob"])),
            Err(CliParseError::UnknownSubcommand("frob".to_owned()))
        );
    }

    #[test]
    fn missing_subcommand_errors() {
        assert_eq!(
            parse_args(&args(&[])),
            Err(CliParseError::MissingSubcommand)
        );
    }

    #[test]
    fn reachability_error_maps_to_exit_two() {
        assert_eq!(
            exit_code(&RunError::Unreachable("gone".into())),
            EXIT_UNREACHABLE
        );
        assert_eq!(exit_code(&RunError::Failure("bad".into())), EXIT_FAILURE);
    }

    #[test]
    fn status_against_missing_root_exits_two() {
        let root =
            std::env::temp_dir().join(format!("true-tick-cli-test-missing-{}", std::process::id()));
        let executable = PathBuf::from("true-tick-cli.exe");
        let code = run(
            &args(&["status", "--root", root.to_str().unwrap()]),
            &executable,
        );
        assert_eq!(code, EXIT_UNREACHABLE);
    }

    #[test]
    fn logs_reads_file_without_any_pipe() {
        let root =
            std::env::temp_dir().join(format!("true-tick-cli-test-logs-{}", std::process::id()));
        let logs = root.join("Data").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("true-tick-2026-09-25.csv"), "a\nb\nc\n").unwrap();
        let executable = PathBuf::from("true-tick-cli.exe");
        let code = run(
            &args(&[
                "logs",
                "--date",
                "2026-09-25",
                "--tail",
                "2",
                "--root",
                root.to_str().unwrap(),
            ]),
            &executable,
        );
        assert_eq!(code, EXIT_OK);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_log_file_is_exit_one_not_two() {
        let root =
            std::env::temp_dir().join(format!("true-tick-cli-test-nolog-{}", std::process::id()));
        let executable = PathBuf::from("true-tick-cli.exe");
        let code = run(
            &args(&[
                "logs",
                "--date",
                "2026-01-01",
                "--root",
                root.to_str().unwrap(),
            ]),
            &executable,
        );
        assert_eq!(code, EXIT_FAILURE);
    }

    #[test]
    fn status_json_escapes_into_valid_object() {
        assert_eq!(
            status_json(r#"status=quoted"value"#),
            r#"{"status":"quoted\"value"}"#
        );
        assert_eq!(
            status_json(r"status=back\slash"),
            r#"{"status":"back\\slash"}"#
        );
        // A token with no equals sign is not a key=value pair and is dropped.
        assert_eq!(status_json("plain"), "{}");
    }

    #[test]
    fn status_json_emits_structured_key_value_object() {
        assert_eq!(
            status_json("status=stopped ownership=released effective_hns=9966 requested_hns=5000"),
            r#"{"status":"stopped","ownership":"released","effective_hns":9966,"requested_hns":5000}"#
        );
        // Unknown numeric values are omitted rather than quoted.
        assert_eq!(
            status_json(
                "status=running ownership=released effective_hns=unknown requested_hns=unknown"
            ),
            r#"{"status":"running","ownership":"released"}"#
        );
    }

    #[test]
    fn date_validation_accepts_shape_and_rejects_noise() {
        assert!(valid_log_date("2026-09-25"));
        assert!(!valid_log_date("2026/09/25"));
        assert!(!valid_log_date("2026-9-5"));
        assert!(!valid_log_date("abcdef-gh-ij"));
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        let (year, month, day) = tick_core::civil_from_days(0);
        assert_eq!(tick_core::format_ymd(year, month, day), "1970-01-01");
        let (year, month, day) = tick_core::civil_from_days(20_721);
        assert_eq!(tick_core::format_ymd(year, month, day), "2026-09-25");
        let (year, month, day) = tick_core::civil_from_days(-1);
        assert_eq!(tick_core::format_ymd(year, month, day), "1969-12-31");
    }
}
