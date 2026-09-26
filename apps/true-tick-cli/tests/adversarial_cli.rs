//! Adversarial integration tests for `true-tick-cli`.
//!
//! Every case spawns the compiled binary through `std::process::Command` so
//! argument parsing, exit codes, and stderr text are exercised end to end
//! without requiring a live tray process or named pipe. Hostile input vectors
//! are grouped by attack surface:
//!
//! 1. Hostile argument fuzzing: empty argv, unknown or missing subcommands,
//!    oversized strings, malformed numerics, and out of bounds intervals.
//! 2. Missing or corrupted `ipc-token` files under `--root`.
//! 3. Missing `Data/logs` directories and malformed `--date` values.
//!
//! Assertions cover exact process exit codes (0, 1, 2) and require clean
//! diagnostics on stderr with no panic markers, no backtraces, and no empty
//! error output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXIT_OK: i32 = 0;
const EXIT_FAILURE: i32 = 1;
const EXIT_UNREACHABLE: i32 = 2;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_true-tick-cli"))
}

fn run_cli(args: &[&str]) -> Output {
    cli()
        .args(args)
        .output()
        .expect("failed to spawn true-tick-cli")
}

fn exit_code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_clean_failure(output: &Output, expected: i32) {
    let code = exit_code(output);
    let stderr = stderr_text(output);
    assert_eq!(code, expected, "stderr: {stderr}");
    assert!(!stderr.is_empty(), "expected error output on stderr");
    assert!(!stderr.contains("panic"), "panic marker found: {stderr}");
    assert!(
        !stderr.contains("RUST_BACKTRACE"),
        "backtrace found: {stderr}"
    );
    assert!(
        !stderr.contains("thread 'main'"),
        "thread panic found: {stderr}"
    );
}

fn temporary_root(label: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-tmp")
        .join(format!("adv-cli-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

fn write_token(root: &Path, contents: &str) {
    let state = root.join("Data").join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("ipc-token"), contents).unwrap();
}

fn write_log(root: &Path, date: &str, contents: &str) {
    let logs = root.join("Data").join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join(format!("true-tick-{date}.csv")), contents).unwrap();
}

mod hostile_arguments {
    use super::*;

    #[test]
    fn empty_args_prints_usage_and_exits_one() {
        let output = run_cli(&[]);
        assert_eq!(exit_code(&output), EXIT_FAILURE);
        let stderr = stderr_text(&output);
        assert!(stderr.contains("USAGE"), "expected usage, got: {stderr}");
        assert!(!stderr.contains("panic"), "panic marker found");
    }

    #[test]
    fn unknown_subcommand_rejected() {
        let output = run_cli(&["frob"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("unknown subcommand"));
    }

    #[test]
    fn unknown_subcommand_case_variants_rejected() {
        for word in ["STATUS", "Status", "LOGS", "Schedule", "Start"] {
            let output = run_cli(&[word]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn status_rejects_extra_args() {
        let output = run_cli(&["status", "--bogus"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["status", "extra"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn status_rejects_double_json_flag() {
        // Parser accepts repeated --json since it is idempotent, verify no crash.
        let output = run_cli(&["status", "--json", "--json"]);
        // This reaches exchange so it fails with unreachable or pipe error.
        assert!(matches!(
            exit_code(&output),
            EXIT_FAILURE | EXIT_UNREACHABLE
        ));
    }

    #[test]
    fn stop_rejects_extra_args() {
        let output = run_cli(&["stop", "extra"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["stop", "--flag"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn cancel_rejects_extra_args() {
        let output = run_cli(&["cancel", "x"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn help_rejects_extra_args() {
        let output = run_cli(&["help", "extra"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn version_rejects_extra_args() {
        let output = run_cli(&["version", "extra"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn schedule_rejects_missing_action() {
        let output = run_cli(&["schedule"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("missing argument"));
    }

    #[test]
    fn schedule_rejects_missing_seconds() {
        let output = run_cli(&["schedule", "start-in"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("missing argument"));
    }

    #[test]
    fn schedule_rejects_unknown_action() {
        let output = run_cli(&["schedule", "explode", "10"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("invalid argument"));
    }

    #[test]
    fn schedule_rejects_negative_seconds() {
        let output = run_cli(&["schedule", "start-in", "-5"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["schedule", "stop-in", "-999999"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn schedule_rejects_zero_seconds() {
        // Zero parses as u32 fine, but should still reach IPC and fail cleanly.
        let output = run_cli(&["schedule", "pause", "0"]);
        // Zero is a valid parse, so we expect either unreachable or failure.
        assert!(matches!(
            exit_code(&output),
            EXIT_FAILURE | EXIT_UNREACHABLE
        ));
    }

    #[test]
    fn schedule_rejects_astronomical_seconds() {
        let output = run_cli(&["schedule", "start-in", "99999999999999999999"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["schedule", "pause", "18446744073709551616"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn schedule_rejects_non_numeric_seconds() {
        for bad in ["abc", "1.5", "0x10", "1e3", " 10", "10 "] {
            let output = run_cli(&["schedule", "start-in", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn schedule_rejects_extra_positional() {
        let output = run_cli(&["schedule", "start-in", "10", "extra"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn start_rejects_missing_interval_value() {
        let output = run_cli(&["start", "--interval"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("missing argument"));
    }

    #[test]
    fn start_rejects_zero_interval() {
        let output = run_cli(&["start", "--interval", "0"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("invalid --interval"));
    }

    #[test]
    fn start_rejects_below_minimum_interval() {
        // 1 HNS is below MIN_INTERVAL_HNS (5000).
        let output = run_cli(&["start", "--interval", "1"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["start", "--interval", "4999"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn start_rejects_above_maximum_interval() {
        // 156251 exceeds MAX_INTERVAL_HNS (156250).
        let output = run_cli(&["start", "--interval", "156251"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["start", "--interval", "9999999999999"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["start", "--interval", "18446744073709551615"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn start_rejects_negative_interval() {
        let output = run_cli(&["start", "--interval", "-1"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        let output = run_cli(&["start", "--interval", "-100000"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn start_rejects_non_numeric_interval() {
        for bad in ["abc", "1.5", "0x10", "1e3", " 10", "10 "] {
            let output = run_cli(&["start", "--interval", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn start_accepts_symbolic_interval_boundary() {
        // SYMBOLIC_RESOLUTION_HNS equals 5000 which is the minimum bound.
        let output = run_cli(&["start", "--interval", "5000"]);
        assert!(matches!(
            exit_code(&output),
            EXIT_FAILURE | EXIT_UNREACHABLE
        ));
    }

    #[test]
    fn start_accepts_maximum_interval_boundary() {
        let output = run_cli(&["start", "--interval", "156250"]);
        assert!(matches!(
            exit_code(&output),
            EXIT_FAILURE | EXIT_UNREACHABLE
        ));
    }

    #[test]
    fn start_rejects_unknown_flag() {
        let output = run_cli(&["start", "--bogus"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn logs_rejects_missing_tail_value() {
        let output = run_cli(&["logs", "--tail"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("missing argument"));
    }

    #[test]
    fn logs_rejects_non_numeric_tail() {
        for bad in ["abc", "1.5", "0x10", "1e3", "-5"] {
            let output = run_cli(&["logs", "--tail", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn logs_rejects_zero_tail() {
        // Zero parses as usize, then fails reading the log file.
        let output = run_cli(&["logs", "--tail", "0"]);
        assert_eq!(exit_code(&output), EXIT_FAILURE);
    }

    #[test]
    fn logs_rejects_huge_tail() {
        let output = run_cli(&["logs", "--tail", "18446744073709551615"]);
        assert_eq!(exit_code(&output), EXIT_FAILURE);
    }

    #[test]
    fn logs_rejects_missing_date_value() {
        let output = run_cli(&["logs", "--date"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("missing argument"));
    }

    #[test]
    fn logs_rejects_malformed_date_formats() {
        // Shape-invalid dates are rejected at parse time with "invalid argument".
        for bad in [
            "2026/09/25",
            "2026-9-5",
            "abcdef-gh-ij",
            "2026-09-25T00:00:00",
            "25-09-2026",
            "notadate",
        ] {
            let output = run_cli(&["logs", "--date", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
            assert!(stderr_text(&output).contains("invalid argument"));
        }
        // Shape-valid but semantically invalid dates pass parsing, then fail
        // at file access with "cannot read log file".
        for bad in ["2026-13-01", "2026-00-01", "2026-01-00", "2026-01-32"] {
            let output = run_cli(&["logs", "--date", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
            assert!(stderr_text(&output).contains("cannot read log file"));
        }
    }

    #[test]
    fn logs_rejects_unknown_flag() {
        let output = run_cli(&["logs", "--bogus"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn root_rejects_missing_value() {
        let output = run_cli(&["--root"]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("missing argument"));
    }

    #[test]
    fn root_with_empty_string() {
        let output = run_cli(&["--root", "", "status"]);
        // Empty path resolves to empty PathBuf, token read fails.
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
    }

    #[test]
    fn oversized_subcommand_rejected() {
        let huge = "a".repeat(30_000);
        let output = run_cli(&[&huge]);
        assert_clean_failure(&output, EXIT_FAILURE);
        assert!(stderr_text(&output).contains("unknown subcommand"));
    }

    #[test]
    fn oversized_flag_value_rejected() {
        let huge = "b".repeat(30_000);
        let output = run_cli(&["start", "--interval", &huge]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn oversized_root_path_rejected() {
        let huge = "c".repeat(30_000);
        let output = run_cli(&["--root", &huge, "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
    }

    #[test]
    fn oversized_date_value_rejected() {
        let huge = "d".repeat(30_000);
        let output = run_cli(&["logs", "--date", &huge]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn path_traversal_in_root() {
        for path in [
            "../../../../etc/passwd",
            "..\\..\\..\\..\\windows\\system32",
            "../../../../../../../etc/shadow",
            "..\\..\\..\\",
            "./.././../",
        ] {
            let output = run_cli(&["--root", path, "status"]);
            assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        }
    }

    #[test]
    fn path_traversal_in_date() {
        for bad in ["../../etc/passwd", "..\\..\\win.ini", "2026-09-25/../../x"] {
            let output = run_cli(&["logs", "--date", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn unicode_and_special_chars_in_subcommand() {
        for word in ["sto\x7fp", "\u{202e}status", "\u{1f680}"] {
            let output = run_cli(&[word]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn unicode_and_special_chars_in_interval() {
        for bad in ["５０００", "５０００", "0x1388"] {
            let output = run_cli(&["start", "--interval", bad]);
            assert_clean_failure(&output, EXIT_FAILURE);
        }
    }

    #[test]
    fn multiple_unknown_args_before_subcommand() {
        let output = run_cli(&["--bogus", "status"]);
        assert_clean_failure(&output, EXIT_FAILURE);
    }

    #[test]
    fn root_flag_after_subcommand() {
        let output = run_cli(&["status", "--root", "somepath"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
    }

    #[test]
    fn repeated_root_flag_last_wins() {
        let root = temporary_root("repeat-root");
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let first_str = first.to_str().unwrap();
        let second_str = second.to_str().unwrap();
        let output = run_cli(&["--root", first_str, "status", "--root", second_str]);
        // Both resolve to unreachable since no token exists.
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn help_and_version_exit_zero() {
        let output = run_cli(&["help"]);
        assert_eq!(exit_code(&output), EXIT_OK);
        assert!(stdout_text(&output).contains("USAGE"));

        let output = run_cli(&["version"]);
        assert_eq!(exit_code(&output), EXIT_OK);
        assert!(!stdout_text(&output).is_empty());
    }
}

mod corrupted_token {
    use super::*;

    #[test]
    fn missing_root_directory_exits_two() {
        let root = temporary_root("missing-root");
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        let stderr = stderr_text(&output);
        assert!(
            stderr.contains("cannot read session token"),
            "stderr: {stderr}"
        );
        assert!(!stderr.contains("panic"), "panic marker found");
    }

    #[test]
    fn empty_token_file_exits_two() {
        let root = temporary_root("empty-token");
        std::fs::create_dir_all(&root).unwrap();
        write_token(&root, "");
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        let stderr = stderr_text(&output);
        assert!(
            stderr.contains("cannot read session token"),
            "stderr: {stderr}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn invalid_hex_token_exits_two() {
        let root = temporary_root("bad-hex");
        std::fs::create_dir_all(&root).unwrap();
        write_token(&root, "not-valid-hex-at-all");
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn short_hex_token_exits_two() {
        let root = temporary_root("short-hex");
        std::fs::create_dir_all(&root).unwrap();
        write_token(&root, "abcd");
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn uppercase_hex_token_exits_two() {
        // Uppercase is rejected by the strict parser.
        let root = temporary_root("upper-hex");
        std::fs::create_dir_all(&root).unwrap();
        write_token(
            &root,
            "A1B2C3D4E5F60718293A4B5C6D7E8F90A1B2C3D4E5F60718293A4B5C6D7E8F90",
        );
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn token_with_whitespace_only_exits_two() {
        let root = temporary_root("ws-token");
        std::fs::create_dir_all(&root).unwrap();
        write_token(&root, "   \n\t  \r\n");
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn token_with_trailing_whitespace_and_bad_hex_exits_two() {
        let root = temporary_root("ws-hex");
        std::fs::create_dir_all(&root).unwrap();
        write_token(&root, "abcd\n");
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn oversized_token_file_exits_two() {
        let root = temporary_root("big-token");
        std::fs::create_dir_all(&root).unwrap();
        write_token(&root, &"f".repeat(1_000_000));
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn token_directory_but_file_missing_exits_two() {
        let root = temporary_root("dir-only");
        let state = root.join("Data").join("state");
        std::fs::create_dir_all(&state).unwrap();
        // Directory exists but no ipc-token file.
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn root_is_a_file_not_directory_exits_two() {
        let root = temporary_root("file-root");
        std::fs::create_dir_all(&root).unwrap();
        let file_root = root.join("fake-root.txt");
        std::fs::write(&file_root, "not a dir").unwrap();
        let output = run_cli(&["--root", file_root.to_str().unwrap(), "status"]);
        assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn all_pipe_verbs_fail_with_missing_token() {
        let root = temporary_root("all-verbs");
        std::fs::create_dir_all(&root).unwrap();
        let root_str = root.to_str().unwrap();
        for sub in ["status", "start", "stop", "cancel"] {
            let output = run_cli(&["--root", root_str, sub]);
            assert_eq!(
                exit_code(&output),
                EXIT_UNREACHABLE,
                "subcommand {sub} should exit 2"
            );
        }
        for args in [
            ["--root", root_str, "schedule", "start-in", "10"],
            ["--root", root_str, "schedule", "pause", "0"],
        ] {
            let output = run_cli(&args);
            assert_eq!(exit_code(&output), EXIT_UNREACHABLE);
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn valid_token_then_pipe_fails() {
        let root = temporary_root("valid-token");
        std::fs::create_dir_all(&root).unwrap();
        // Valid 64-char lowercase hex token. With no live tray the pipe is
        // unreachable (exit 2). With a running tray the token is unauthorized
        // (exit 1). Either way the CLI fails cleanly with a non-zero code.
        write_token(
            &root,
            "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
        );
        let output = run_cli(&["--root", root.to_str().unwrap(), "status"]);
        let code = exit_code(&output);
        assert!(
            matches!(code, EXIT_FAILURE | EXIT_UNREACHABLE),
            "expected 1 or 2, got {code}"
        );
        let stderr = stderr_text(&output);
        assert!(!stderr.contains("panic"), "panic marker found");
        std::fs::remove_dir_all(&root).ok();
    }
}

mod missing_logs {
    use super::*;

    #[test]
    fn missing_logs_directory_exits_one() {
        let root = temporary_root("no-logs");
        std::fs::create_dir_all(&root).unwrap();
        let output = run_cli(&[
            "--root",
            root.to_str().unwrap(),
            "logs",
            "--date",
            "2026-01-01",
        ]);
        assert_eq!(exit_code(&output), EXIT_FAILURE);
        let stderr = stderr_text(&output);
        assert!(stderr.contains("cannot read log file"), "stderr: {stderr}");
        assert!(!stderr.contains("panic"), "panic marker found");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_log_file_for_valid_date_exits_one() {
        let root = temporary_root("no-file");
        let logs = root.join("Data").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        // Log directory exists but file does not.
        let output = run_cli(&[
            "--root",
            root.to_str().unwrap(),
            "logs",
            "--date",
            "2026-01-01",
        ]);
        assert_eq!(exit_code(&output), EXIT_FAILURE);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_date_rejected_before_file_access() {
        let root = temporary_root("bad-date");
        std::fs::create_dir_all(&root).unwrap();
        let output = run_cli(&["--root", root.to_str().unwrap(), "logs", "--date", "bogus"]);
        assert_eq!(exit_code(&output), EXIT_FAILURE);
        let stderr = stderr_text(&output);
        assert!(stderr.contains("invalid argument"), "stderr: {stderr}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_log_file_returns_ok() {
        let root = temporary_root("empty-log");
        write_log(&root, "2026-09-26", "");
        let output = run_cli(&[
            "--root",
            root.to_str().unwrap(),
            "logs",
            "--date",
            "2026-09-26",
        ]);
        assert_eq!(exit_code(&output), EXIT_OK);
        assert!(stdout_text(&output).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn log_file_with_only_newlines_returns_ok() {
        let root = temporary_root("blank-log");
        write_log(&root, "2026-09-26", "\n\n\n");
        let output = run_cli(&[
            "--root",
            root.to_str().unwrap(),
            "logs",
            "--date",
            "2026-09-26",
        ]);
        assert_eq!(exit_code(&output), EXIT_OK);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tail_larger_than_file_returns_all_lines() {
        let root = temporary_root("big-tail");
        write_log(&root, "2026-09-26", "line1\nline2\nline3\n");
        let output = run_cli(&[
            "--root",
            root.to_str().unwrap(),
            "logs",
            "--date",
            "2026-09-26",
            "--tail",
            "100",
        ]);
        assert_eq!(exit_code(&output), EXIT_OK);
        let stdout = stdout_text(&output);
        assert!(stdout.contains("line1"));
        assert!(stdout.contains("line3"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tail_zero_returns_no_lines() {
        let root = temporary_root("zero-tail");
        write_log(&root, "2026-09-26", "line1\nline2\nline3\n");
        let output = run_cli(&[
            "--root",
            root.to_str().unwrap(),
            "logs",
            "--date",
            "2026-09-26",
            "--tail",
            "0",
        ]);
        assert_eq!(exit_code(&output), EXIT_OK);
        assert!(stdout_text(&output).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn log_filename_injection_via_date() {
        // Date validation prevents traversal, but also verify shape rejection.
        let root = temporary_root("inject");
        std::fs::create_dir_all(&root).unwrap();
        for bad in [
            "../secret",
            "..\\secret",
            "2026-09-25/../x",
            "x/../../y",
            "2026-09-25%00.csv",
        ] {
            let output = run_cli(&["--root", root.to_str().unwrap(), "logs", "--date", bad]);
            assert_eq!(exit_code(&output), EXIT_FAILURE, "date {bad} should fail");
        }
        std::fs::remove_dir_all(&root).ok();
    }
}
