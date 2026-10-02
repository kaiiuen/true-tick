# True Tick Development Guide

This guide covers how the True Tick workspace is organized, how to build and test it, and the conventions to follow during normal development. It is written against the implemented code and describes what exists today, not what is planned.

## Repository layout

The workspace is a single Cargo workspace declared in the root `Cargo.toml` with `resolver = "2"`. It is split into two directories: `apps/` holds the four shipped binaries and `crates/` holds the eleven library and harness crates they compose. The workspace default member is `apps/true-tick`, so a bare `cargo build` compiles the tray application alone.

### Applications

`apps/true-tick` produces `true-tick.exe`, the Windows notification area application at the center of the project. Its `src/main.rs` compiles under the `windows` subsystem, links `ntdll` and `kernel32` directly for `NtSetTimerResolution` and console control handling, and is organized into focused modules: `config`, `environment`, `portable`, `session`, `shutdown`, `emergency`, `logging`, `pause`, `tray_surface`, a `tray/` subtree covering the icon, menu, controller, and IPC server, and a `ui/` subtree covering the diagnostic window, presets window, and marquee rendering.

`apps/true-tick-cli` produces `true-tick-cli.exe`, the command line control client. It is a thin IPC client: control verbs are sent to a running tray instance over the `tick-ipc` named pipe and authenticated with the ephemeral session token read from the `ipc-token` file, while the `logs` verb reads daily CSV log files straight from disk and never touches the pipe. When no tray instance is running it can spawn a detached engine process and poll briefly for the token file to appear.

`apps/launcher` produces `Launcher.exe`, the standalone portable loader. It manages the A/B slot layout of a portable package, verifies the signed manifest and per entry digests through `tick-crypto` before selecting a slot, tracks consecutive launch failures for crash loop detection, and falls back to the standby slot when the active slot is corrupted or its manifest fails verification.

`apps/manifest-tool` produces `manifest-tool.exe`, the release signing utility. It generates Ed25519 trust anchors and signing keys, renders canonical `manifest.sig` files for package directories using the producer helpers exported by `tick-crypto`, and verifies signed packages against a trust anchor. The crate forbids unsafe code.

### Crates

`tick-core` is the platform-neutral domain layer. It defines the `Hns` value type for durations in 100-nanosecond units, the `Lifecycle`, `Status`, `DesiredIntent`, and `Event` vocabulary shared across boundaries, `CoreError`, and the pure civil calendar conversion functions (`days_from_civil`, `civil_from_days`, `format_ymd`) used for date handling. It intentionally makes no operating system calls.

`tick-policy` holds the pure decision logic. The `decide` function maps a `PolicyInput` (enabled flag, profile eligibility, `PowerState`, battery lockout) to a `PolicyDecision`, the `OperatingTier` evaluation escalates through nominal, surface degraded, metrology degraded with a fallback floor, and quiescent states, and `ResponsivenessTracker` classifies observed latency as normal, elevated, or degraded from consecutive sample streaks.

`tick-ownership` models timer resolution ownership. The `Ownership` state machine tracks released, owned, and uncertain states, and `TimerController` wraps a `TimerPlatform` to sequence preflight queries, requests, releases, verification, and the startup kernel settle probe that recovers orphaned resolution requests left behind by a previous process lifetime.

`tick-platform-windows` is the Windows adapter behind the `TimerPlatform` trait. It calls `NtQueryTimerResolution` and `NtSetTimerResolution` through `ntdll`, normalizes reported bounds inside `TimerBounds`, validates requests against a 100 HNS hardware tolerance, takes a median of three verification samples after each request, and implements `attempt_kernel_settle_probe`.

`tick-observation-windows` implements event-driven power observation. `WindowsObservation` queries `GetSystemPowerStatus` at startup and again after power broadcast events, never polls, maps the AC line status and system status flag onto `PowerSnapshot`, and clears stale readings to `Unknown` whenever a query fails.

`tick-diagnostics` provides the bounded in-memory diagnostic store. `DiagnosticStore` is a per process event buffer capped at `HARD_MAX_EVENTS` (512) whose entries are SHA-256 hash chained from a genesis seed so tampering is detectable at an exact index, and the `privacy`, `recorder`, and `retention` modules handle field sanitization, CSV and TSV rendering, and retention bounds including the telemetry directory byte cap.

`tick-startup-windows` handles per user boot registration. `WindowsUserStartup` writes or removes the `TrueTick` value under the current user `Run` key only, validates that the registered path is a launcher or development executable, and creates no service, scheduled task, or machine-wide state.

`tick-multiclient` is a test harness crate with both a library and a helper binary. It exists to prove that a tick release drops only the application's own tracked contribution to the global timer resolution, classifying the post release observation as finer client retained, tick only released, or an unknown state within a 100 HNS tolerance.

`tick-watchdog` is the heartbeat monitor. A `Heartbeat` records monotonic kick timestamps and a `WatchdogHandle` runs a bounded monitor thread that flags a stall after `STALL_THRESHOLD_MS` (4000 ms) without a kick, invokes a stall callback once per episode with recovery resetting the episode state, and exits after `MAX_STALL_EPISODES` (8) so a broken process cannot spin callbacks indefinitely.

`tick-crypto` implements the signed manifest scheme. It renders a deterministic canonical manifest text, produces and verifies Ed25519 signatures over that byte sequence, checks per entry SHA-256 digests, and enforces a strictly increasing release counter to refuse downgrades. The crate exports both the verifier consumed by the launcher and the producer helpers reused by `manifest-tool`.

`tick-ipc` defines the IPC wire protocol. Frames are bounded to `MAX_PAYLOAD_LEN` (256 bytes) under a magic header and protocol version, carry a CRC-32 integrity check and a 32-byte ephemeral session token, restrict timer intervals to the 0.5 ms to 15.625 ms range, and enforce per PID rate limiting of `MAX_REQUESTS_PER_SECOND` (10) across at most `MAX_TRACKED_PIDS` (64) callers. The tray serves the `\\.\pipe\TrueTick-Ipc-v1` named pipe.

## Building

### Prerequisites

- A stable Rust toolchain with the `x86_64-pc-windows-msvc` target installed.
- Microsoft Visual C++ Build Tools, which supply the MSVC linker and import libraries.
- Windows 10 version 1709 or newer, or Windows 11, on 64-bit x86 hardware, for actually running the binaries.

The repository pins the toolchain in `rust-toolchain.toml` to the `stable` channel with the `rustfmt` and `clippy` components, so `rustup` selects the correct toolchain automatically inside the project directory. The checked in `.cargo/config.toml` adds `-C target-feature=+crt-static` for the MSVC target, so the binaries link the C runtime statically and run with no external runtime dependency.

### Build commands

Build the whole workspace, every app and every crate:

```cmd
cargo build --workspace
```

Produce optimized binaries:

```cmd
cargo build --release
```

Build a single application or crate by package name:

```cmd
cargo build -p true-tick
cargo build -p true-tick-cli
cargo build -p true-tick-launcher
cargo build -p manifest-tool
```

A bare `cargo build` compiles only `apps/true-tick` because it is the workspace default member. Binaries land under `target/debug/` or `target/release/` depending on profile. The launcher package is named `true-tick-launcher` even though its binary is `Launcher.exe`, and the multiclient harness binary is `tick-multiclient`.

## Testing

Run the full suite with:

```cmd
cargo test --workspace
```

Unit tests live inline in each crate's `src/` next to the code they cover, for example the policy decision tables in `tick-policy`, the ownership transition matrix in `tick-ownership`, and the calendar arithmetic in `tick-core`. Because `tick-core`, `tick-policy`, `tick-ownership`, `tick-multiclient`, and most of `tick-watchdog` are pure logic behind the `TimerPlatform` and `ObservationSource` traits, the bulk of the suite runs without native calls. Windows adapters gate their real system paths with `cfg(windows)` and return `Unsupported` elsewhere, so the portable logic still compiles and tests cleanly.

Integration suites under each crate's `tests/` directory cover the hostile and long running cases:

- `crates/tick-ipc/tests/adversarial_security.rs` fuzzes the wire protocol: malformed magic and version fields, token tampering and bit flipping, CRC-32 corruption detection, payload length hard limits, and token file parser hardening.
- `crates/tick-diagnostics/tests/tamper_injection.rs` mutates recorded events byte by byte and asserts the hash chain reports the exact index of each tampered entry.
- `apps/true-tick/tests/ipc_concurrency_stress.rs` rehosts the production named pipe accept loop inside the test and drives concurrent clients against it. It runs on Windows only and serializes on a shared mutex because the pipe name is a single global kernel object.
- `apps/true-tick/tests/soak_stress.rs` drives ten thousand cyclic policy, ownership, and scheduling transitions against a mock platform while continuously verifying the diagnostic hash chain and rate limiter bounds.
- `apps/true-tick/tests/long_run.rs` runs the shared workload mixer for `TRUE_TICK_LONG_RUN_SECONDS`, five seconds by default, and asserts zero retention bound violations. Setting the variable is the opt in for a longer pass.
- `apps/true-tick-cli/tests/adversarial_cli.rs` spawns the compiled CLI binary end to end with hostile arguments, missing or corrupted token files, and malformed dates, asserting exact process exit codes and clean stderr output.
- `crates/tick-ownership/tests/state_machine_permutations.rs` covers ownership transition permutations, and `crates/tick-platform-windows/tests/fault_injection.rs` covers platform fault injection.

The CLI tests exercise `true-tick-cli` as what it is, a thin client over the named pipe. Control verbs round trip through `tick-ipc` framing, session token authentication, and the per PID rate limiter, so exercising the binary exercises the same protocol surface a real caller uses.

## Development workflow

### Toolchain

Development targets stable Rust. The `rust-toolchain.toml` pin requests `channel = "stable"` plus the `rustfmt` and `clippy` components, so let `rustup` manage the toolchain rather than overriding it per machine. All crates share the workspace `edition = "2021"` and the license declared once in the root `Cargo.toml`.

### Formatting and linting

Formatting is enforced with rustfmt. To check without writing:

```cmd
cargo fmt --all -- --check
```

To apply formatting:

```cmd
cargo fmt --all
```

Linting is enforced with clippy at deny warnings level. The exact gate is:

```cmd
cargo clippy --workspace --all-targets -- -D warnings
```

Run both before finishing a change. The `--all-targets` flag matters because it pulls test code, including the integration suites under `tests/`, into the lint pass.

### Working across the layers

The crate graph flows in one direction: `tick-core` holds the shared vocabulary, `tick-policy` and `tick-platform-windows` build on it, `tick-ownership` composes the platform trait, and `apps/true-tick` wires everything together. When you change a type or constant in a lower crate, search its dependents for call sites and update the matching diagram under `docs/diagrams/` in the same change. The `TimerPlatform` and `ObservationSource` traits exist so ownership and policy logic stays testable without native calls, so prefer adding behavior behind those seams rather than reaching for `cfg(windows)` inside the pure crates.

## Architecture diagrams

This guide is the prose layer of the documentation set. For an alternate visual view, every production source file has a Mermaid diagram under `docs/diagrams/` that mirrors its path, so `crates/tick-ownership/src/lib.rs` maps to `docs/diagrams/crates/tick-ownership/src/lib.md`. The index at `docs/diagrams/README.md` lists all diagrams and links each one to its source file.
