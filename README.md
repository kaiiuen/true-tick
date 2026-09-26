# True Tick

A portable Windows notification area application and command line interface for managing user mode timer resolution.

True Tick acquires, verifies, and releases exclusively its own tracked timer resolution request through native NT system interfaces without injecting drivers, modifying global system defaults, or interfering with external processes.

## Binaries

- `true-tick.exe`: Windows notification area application managing timer resolution lifecycle and presenting diagnostic state.
- `true-tick-cli.exe`: Command line control client communicating over the local named pipe IPC interface.
- `Launcher.exe`: Standalone portable loader managing A/B slots and self healing recovery.
- `manifest-tool.exe`: Cryptographic release manifest signing and verification utility.

## System Requirements

- Windows 10 (version 1709 or newer) or Windows 11
- 64-bit x86 architecture (x86_64)
- No external runtime dependencies (statically linked C runtime)

## Building from Source

Prerequisites:
- Stable Rust toolchain with the `x86_64-pc-windows-msvc` target
- Microsoft Visual C++ Build Tools

Build all binaries in release mode:

```cmd
cargo build --release
```

Run automated workspace test suites:

```cmd
cargo test --workspace
```

## Command Line Interface

When `true-tick.exe` is running, `true-tick-cli.exe` can query state and submit commands over the local named pipe interface:

```cmd
# Query current status
true-tick-cli status
true-tick-cli status --json

# Request high resolution timer acquisition
true-tick-cli start
true-tick-cli start --interval 5000

# Release requested timer resolution
true-tick-cli stop

# Schedule timed actions
true-tick-cli schedule start-in 60
true-tick-cli schedule pause 300
true-tick-cli cancel

# View diagnostic logs directly from disk
true-tick-cli logs --tail 20
```

## License

Copyright (c) 2026 kaiiuen.

True Tick is licensed under the PolyForm Noncommercial License 1.0.0. See [LICENSE](LICENSE) for terms.
