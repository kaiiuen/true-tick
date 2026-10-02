# Portable Packaging Model

This document explains the portable packaging architecture and runtime lifecycle of True Tick. The portable deployment model allows True Tick to execute from a self-contained directory tree without system-wide installation, registry modifications, or external dependencies.

For a control flow diagram of the launcher selection and rollback sequence, refer to `docs/diagrams/apps/launcher/src/main.md`.

## Directory Layout

The portable package root contains a structured set of binaries, slot directories, manifests, mutable data stores, and recovery artifacts:

```text
<package-root>/
|-- Launcher.exe
|-- active-slot.txt
|-- SHA256SUMS.txt
|-- manifest.sig           (optional signed manifest)
|-- trust-anchor.pub       (32-byte Ed25519 public key anchor)
|-- Slots/
|   |-- A/
|   |   |-- true-tick.exe
|   |   `-- true-tick.toml
|   `-- B/
|       |-- true-tick.exe
|       `-- true-tick.toml
|-- Recovery/
|   |-- true-tick.exe
|   `-- true-tick.toml
`-- Data/
    |-- logs/
    |   `-- launcher-rollback.log
    `-- state/
        |-- release-counter.state
        |-- slot-A-health.state
        `-- slot-B-health.state
```

The components serve designated roles in the deployment lifecycle:

- `Launcher.exe`: The root entry point. It inspects metadata, selects the active slot, validates payload integrity, orchestrates rollback when needed, and spawns the payload executable.
- `active-slot.txt`: A plain text file defining the currently designated slot. The valid payload must normalize to either `A` or `B`.
- `SHA256SUMS.txt`: The checksum manifest containing SHA-256 digest entries for portable package executables.
- `manifest.sig`: An optional cryptographically signed manifest containing entries, SHA-256 digests, a monotonic release counter, and an Ed25519 signature.
- `trust-anchor.pub`: A raw 32-byte Ed25519 public key file used to verify `manifest.sig`.
- `Slots/A/` and `Slots/B/`: Dual partition payload directories housing the application binaries (`true-tick.exe`) and configurations (`true-tick.toml`).
- `Recovery/`: A golden master partition storing a known valid executable and configuration for autonomous emergency restoration.
- `Data/`: The partition designated for runtime mutable state, including crash-loop health counters, persisted release counter state, and rollback audit logs.

The application executable itself inspects its own location via `apps/true-tick/src/portable.rs`. It resolves the root directory by walking up from the slot path (`Slots/<slot>/true-tick.exe`) and locates `Launcher.exe` at the package root.

## Slot Selection and Normalization

When `Launcher.exe` starts, it derives the workspace root from its own executable path and reads `active-slot.txt`.

### Metadata Normalization

Raw bytes in `active-slot.txt` may contain formatting noise introduced by editors or transfer tools. The launcher normalizes the input across up to 8 passes by stripping noise characters from both edges:

- UTF-8 byte order marks (U+FEFF)
- Carriage returns (`\r`)
- Line feeds (`\n`)
- Null bytes (`\0`)
- Standard Unicode whitespace

If the normalized value equals `A`, slot A is preferred. If it equals `B`, slot B is preferred.

### Fallback Behavior

If `active-slot.txt` contains invalid content or cannot be resolved to `A` or `B`, the launcher attempts deterministic fallback via `fallback_slot_from_installed_executables`. It checks whether `Slots/A/true-tick.exe` exists, and if absent, checks `Slots/B/true-tick.exe`. Slot A takes precedence when both binaries exist. If neither executable exists on disk, selection terminates with an `InvalidMetadata` error.

In `apps/true-tick/src/portable.rs`, standalone slot selection directly inspects `active-slot.txt`. If the file is missing or invalid, it returns a `RepairRequired` descriptor rather than attempting autonomous slot repair.

## Integrity Verification

Before launching any executable, the launcher verifies binary integrity against manifest entries.

### Signed Manifest Verification

When `manifest.sig` exists in the package root, the launcher prioritizes signed verification:

1. `trust-anchor.pub` is loaded from the package root. The anchor must contain exactly 32 bytes representing an Ed25519 public key. Missing anchor files produce `TrustAnchorMissing`, while improper byte lengths or invalid public keys produce `TrustAnchorInvalid`.
2. The manifest signature is verified against the canonical manifest byte representation using the raw 32-byte Ed25519 anchor. Any signature failure returns `SignatureVerificationFailed`.
3. The release counter within `manifest.sig` is compared against the persisted counter in `Data/state/release-counter.state`.
4. The target slot executable digest is matched against the corresponding `Slots/<slot>/true-tick.exe` entry in `manifest.sig`. Digest mismatches return `SignedDigestMismatch`.
5. If the signed manifest contains an entry for `Recovery/true-tick.exe`, the recovery binary digest is also verified.

### Checksum Verification Fallback

If `manifest.sig` is not present, the launcher logs a diagnostic message and falls back to `SHA256SUMS.txt`:

1. The launcher checks for `SHA256SUMS.txt`. If absent, it yields `ManifestMissing`.
2. The manifest is parsed line by line for an entry matching the relative path of the target executable (`Slots/<slot>/true-tick.exe`, supporting both forward and backward slashes).
3. If no matching entry exists in the manifest, the launcher returns a `ManifestHashMissing` error.
4. The SHA-256 digest of the slot executable is computed and compared case-insensitively with the expected hash. A mismatch produces `ChecksumMismatch`.

## Crash-Loop Health Tracking and Rollback

To prevent persistent startup failure loops caused by runtime crashes or faulty updates, the launcher tracks slot health in `Data/state/slot-<slot>-health.state`.

### Health Tracking and Atomic Counter Updates

The health file contains a plain text integer representing consecutive startup failures for that slot:

- When verifying a slot, the launcher reads the slot health counter. Missing files default to 0.
- If the failure count reaches or exceeds `BOOT_FAILURE_THRESHOLD` (constant value of 3), the launcher rejects the slot with `CrashLoopDetected`.
- When a slot fails verification or crash-loop checks, its failure counter is incremented using `saturating_add(1)`.
- Updates to health counters are written via a temporary staging file (`slot-<slot>-health.state.staging.<pid>.tmp`) and committed using `atomic_replace`.
- On Windows, `atomic_replace` calls `MoveFileExW` with flags `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`, ensuring atomic durability without corrupting state during power loss.
- When a slot verification succeeds and the process is committed for launch, the slot health state file is deleted via `clear_slot_health`.

### Tier 2 Autonomous Rollback

When the active slot fails verification or exceeds the crash threshold:

1. The launcher increments the health failure counter of the active slot.
2. The launcher evaluates the standby slot (`slot.alternate()`).
3. If the standby slot passes health check and integrity verification, Tier 2 rollback executes:
   - A rollback decision event is appended to `Data/logs/launcher-rollback.log`.
   - `active-slot.txt` is updated atomically to point to the standby slot.
   - The health counter of the standby slot is cleared.
   - Any pending release counter is committed.
   - The launcher spawns the standby executable.

## Golden Master Recovery

When both slot A and slot B fail integrity checks or hit crash-loop thresholds, the launcher falls back to Tier 3 autonomous reconstruction using the golden master partition in `Recovery/`:

1. Both active and standby failure counters are incremented.
2. The launcher checks `Recovery/true-tick.exe` and `Recovery/true-tick.toml`.
3. The golden master executable is verified against `SHA256SUMS.txt`. It must have a valid hash entry and match its recorded checksum.
4. The golden binary is copied into a temporary staging file in `Slots/A/` (`true-tick.exe.staging.<pid>.tmp`).
5. The launcher computes the hash of the staged file, compares it against the golden master binary hash, and commits it into `Slots/A/true-tick.exe` using `atomic_replace`.
6. `Recovery/true-tick.toml` is copied into `Slots/A/true-tick.toml`.
7. `verify_slot_executable` validates slot A once more.
8. An event record is appended to `Data/logs/launcher-rollback.log`:
   `event=autonomous_golden_master_restoration target=Slot::A status=restored`
9. `active-slot.txt` is atomically rewritten to `A`.
10. Health tracking files for both slot A and slot B are cleared.
11. Slot A is launched.

If golden master restoration fails or `Recovery/` is unverified, the launcher returns `BothSlotsCorrupted` and displays a graphical modal error dialog on Windows before terminating.

## Release Counter Ordering Rule

Downgrade protection and rollback safety are governed by the release counter in `manifest.sig` and `Data/state/release-counter.state`.

The release counter rule operates under a three-way comparison:

1. Counter less than persisted state: The manifest release counter is lower than the counter previously recorded on disk. The package is rejected immediately as a replay or downgrade attack (`ReleaseCounterRejected`).
2. Counter equal to persisted state: The launcher is relaunching the currently installed release. The launch proceeds without modifying the counter file on disk.
3. Counter greater than persisted state: A verified upgrade is detected. The new counter is accepted and marked as pending.

### Deferred Counter Persistence

Persistence of an upgraded release counter is strictly deferred until after slot verification succeeds and the target slot is committed for launch. Staging and atomic replacement ensure that if manifest reading, signature verification, executable hashing, or slot launching fails, the on-disk counter in `release-counter.state` remains at the previous value. This guarantees that failed upgrade attempts do not prevent rollback to an earlier bootable release.

## Source of Truth Diagram

For the complete visual flowchart depicting slot parsing, signature verification, integrity checksums, crash thresholds, and multi-tier recovery pathways, see `docs/diagrams/apps/launcher/src/main.md`.
