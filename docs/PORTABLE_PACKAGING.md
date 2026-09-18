# Portable Packaging and A/B Slot Architecture

## 1. Zero-Footprint Self-Containment
True Tick adheres to a strict self-containment contract:
- The entire application runtime resides within a single portable directory tree.
- Zero writes to `%APPDATA%`, `%LOCALAPPDATA%`, or `ProgramData`.
- Moving or renaming the containing directory leaves the application fully functional.
- The single machine-level touchpoint is the optional Windows Run key (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`), which points strictly to `Launcher.exe` inside the portable root.

## 2. Directory Layout
The official portable release package is structured as follows:

```
True-Tick-internal-v0.1.0/
├── Launcher.exe               Primary user-facing startup executable
├── active-slot.txt            Single ASCII character identifying active partition (A or B)
├── SHA256SUMS.txt             Cryptographic manifest of package payload
├── INTERNAL_TEST_README.txt   Verification instructions and test notices
├── Data/                      Mutable runtime directory (created upon first launch)
│   └── logs/
│       ├── session.state      Unclean shutdown marker (running vs clean)
│       └── true-tick-*.csv    Rolling daily forensic audit logs with SHA-256 chains
└── Slots/                     Immutable payload partitions
    ├── A/
    │   ├── true-tick.exe      Primary compiled application binary
    │   └── true-tick.toml     Slot-specific configuration file
    └── B/
        ├── true-tick.exe      Staged update or standby binary
        └── true-tick.toml     Standby configuration file
```

## 3. Launcher Execution Semantics
`Launcher.exe` acts as an isolated process supervisor:
1. Locates its own executable path via `GetModuleFileNameW`.
2. Verifies the existence of `active-slot.txt` within the adjacent directory.
3. Reads the active slot identifier (`A` or `B`).
4. Validates that `Slots/<slot>/true-tick.exe` exists, is a file, and passes the `SHA256SUMS.txt` manifest checksum.
5. If the active slot fails verification, the launcher autonomously checks the alternate standby slot, and when the standby verifies cleanly it appends a rollback record to `Data/logs/launcher-rollback.log`, rewrites `active-slot.txt` to the standby slot, and boots the standby.
6. If both slots fail verification, the launcher reports a `BothSlotsCorrupted` repair condition and refuses to boot.
7. Spawns the verified target binary as an independent child process via `Command::spawn()`.
8. Exits immediately with code 0, freeing all launcher resources while the payload runs.

If `active-slot.txt` is missing or corrupt, `Launcher.exe` presents a native error dialog informing the user of the damaged package rather than guessing a default.

## 4. Manifest Integrity and Verification
Every packaged file is fingerprinted in `SHA256SUMS.txt`. The packaging tool (`rebuild_package.py`) performs pre-flight SHA-256 calculation and post-assembly verification to guarantee that no payload binary is corrupted during staging.
