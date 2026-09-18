# Development and Verification Workflow

## 1. Prerequisites and Toolchain
True Tick requires a standard 64-bit Rust toolchain on Windows:
- Rust toolchain: stable `x86_64-pc-windows-msvc` (edition 2021).
- Microsoft Visual C++ build tools (MSVC linker and Windows SDK).
- Python 3 for running workspace style checkers and packaging scripts.

## 2. Workspace Structure
The repository is organized as a Cargo workspace with dedicated crates enforcing strict domain separation:

```
true-tick/
├── apps/
│   ├── launcher/              Portable launcher binary (Launcher.exe)
│   └── true-tick/             Primary Windows notification area application
└── crates/
    ├── tick-calibration/      Platform timing capability boundaries
    ├── tick-core/             Mathematical primitives and 100-nanosecond units
    ├── tick-diagnostics/      In-memory session log and SHA-256 hash chaining
    ├── tick-multiclient/      Multi-client arbitration verification harness
    ├── tick-observation-windows/ Power and session query adapter
    ├── tick-ownership/        Deterministic request lifecycle and ownership tracking
    ├── tick-platform-windows/ Native ntdll FFI and kernel boundary normalization
    ├── tick-policy/           Formal evaluation rules across power states
    └── tick-startup-windows/  Windows Run key startup registration
```

## 3. Building the Application
To build the binaries under debug mode:
```sh
cargo build --workspace
```

To build optimized release binaries:
```sh
cargo build --release --workspace
```

The primary application binary (`true-tick.exe`) is generated with an embedded Common Controls version 6 application manifest via `build.rs` to ensure modern visual styling and listview support.

## 4. Automated Verification Suite
True Tick enforces exhaustive automated test verification across all subsystem layers. To execute all unit and integration tests:
```sh
cargo test --workspace
```

To run strict static analysis with all compiler and clippy warnings denied:
```sh
cargo clippy --workspace --all-targets -- -D warnings
```

## 5. Continuous Integration Style Guards
Two specialized style guards enforce architectural hygiene across the codebase:

### 5.1 Documentation Punctuation Checker
Enforces the strict punctuation invariant (zero em-dashes and zero semicolons in prose or documentation):
```sh
py tools/true-tick-dev/check_doc_punctuation.py docs README.md ARCHITECTURE.md STATUS.md
```

### 5.2 Live Timer Literal Checker
Ensures that no production code contains hardcoded or captured timer resolution constants, guaranteeing that all timer decisions derive strictly from live kernel queries:
```sh
py tools/true-tick-dev/check_live_timer_values.py
```

## 6. Internal Package Assembly
To assemble a fresh self-contained portable package with clean data roots and verified SHA-256 manifests:
```sh
py tools/true-tick-dev/rebuild_package.py
```
This script validates directory bounds, stages `Launcher.exe`, generates `active-slot.txt`, populates the `Slots/A` and `Slots/B` targets, and writes `SHA256SUMS.txt`.
