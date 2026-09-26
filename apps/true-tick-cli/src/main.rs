//! Command line control surface for a running True Tick tray instance.
//!
//! The binary talks to the tray process over the tick-ipc named pipe for
//! control verbs and reads daily CSV logs straight from disk for `logs`,
//! which never touches the pipe.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

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
    Cancel,
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
        CliRequest::Cancel => match exchange(&root, CommandVerb::CancelSchedule, &[]) {
            Ok(response) => report_response(&response),
            Err(error) => report_error(&error),
        },
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
        "cancel" => expect_no_args(rest, CliRequest::Cancel),
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

/// Wraps the Status payload text as a small valid JSON object.
fn status_json(text: &str) -> String {
    format!("{{\"status\":\"{}\"}}", json_escape(text))
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
     \x20   cancel                     Cancel a pending scheduled action.\n\
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
        assert_eq!(status_json("plain"), "{\"status\":\"plain\"}");
        assert_eq!(status_json("a\"b\nc"), "{\"status\":\"a\\\"b\\nc\"}");
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
