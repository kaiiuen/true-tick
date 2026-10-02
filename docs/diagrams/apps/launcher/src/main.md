# True Tick Launcher main Control Flow Diagram

Source path: `true-tick/apps/launcher/src/main.rs`

```mermaid
flowchart TD
    Start["main runs launcher"] --> Run["run resolves workspace root"]
    Run --> Select["select reads active slot file"]
    Select --> ParseMeta{"parse slot value"}
    ParseMeta -- "valid" --> HealthCheck
    ParseMeta -- "invalid" --> Fallback{"installed executable present?"}
    Fallback -- "yes" --> HealthCheck
    Fallback -- "no" --> ErrInvalid["InvalidMetadata error"]

    HealthCheck{"slot health at or above 3?"}
    HealthCheck -- "yes" --> CrashLoop["CrashLoopDetected"]
    CrashLoop --> IncActive["increment active slot health via atomic replace"]
    HealthCheck -- "no" --> VerifyExe["verify slot executable"]

    VerifyExe --> HasSig{"manifest sig present?"}
    HasSig -- "yes" --> SignedLoad["load manifest and trust anchor, verify signature"]
    HasSig -- "no" --> ChecksumRead["read checksum manifest, find hash entry"]

    Counter -- "below seen" --> Downgrade["ReleaseCounterRejected"]
    Downgrade --> ActiveErr
    Counter -- "equal seen" --> SignedDigest
    Counter -- "above seen" --> SignedDigest
    SignedDigest["verify slot and recovery digests"] -- "fail" --> ActiveErr
    SignedDigest -- "ok, pending counter" --> ActiveOk

    ChecksumRead --> HashFound{"entry present?"}
    HashFound -- "no" --> ManifestHashMissing["ManifestHashMissing"]
    ManifestHashMissing --> ActiveErr
    HashFound -- "yes" --> HashCompare{"computed sha256 matches?"}
    HashCompare -- "no" --> ChecksumMismatch["ChecksumMismatch"]
    ChecksumMismatch --> ActiveErr
    HashCompare -- "yes" --> ActiveOk

    ActiveOk["active slot verified"] --> ClearActive["clear active slot health"]
    ClearActive --> CommitActive["commit release counter"]
    CommitActive --> ReturnActive["launch active slot"]

    ActiveErr --> IncActive
    IncActive --> CheckStandby{"standby health ok and verifies?"}
    CheckStandby -- "ok" --> Rollback["record rollback, write standby as active"]
    Rollback --> ClearStandby["clear standby health"]
    ClearStandby --> CommitStandby["commit release counter"]
    CommitStandby --> ReturnStandby["launch standby slot"]
    CheckStandby -- "fail" --> IncStandby["increment standby health via atomic replace"]
    IncStandby --> Golden{"restore from golden master?"}

    Golden -- "ok" --> GoldenOk["verify recovery exe, copy to slot A, write active A"]
    GoldenOk --> ClearBoth["clear both health states"]
    ClearBoth --> CommitGolden["commit release counter"]
    CommitGolden --> ReturnGolden["launch restored slot A"]
    Golden -- "fail" --> BothCorrupted["BothSlotsCorrupted error"]

    ReturnActive --> Spawn["spawn true tick exe with forwarded args"]
    ReturnStandby --> Spawn
    ReturnGolden --> Spawn
    Spawn --> OkZero["return 0 without waiting for child"]
    OkZero --> End["launcher exits"]
    ErrInvalid --> Repair["show repair dialog and exit failure"]
    BothCorrupted --> Repair
```

## Notes

- Slot selection reads `active-slot.txt`, normalizes a UTF-8 BOM, CRLF, whitespace and null bytes, then parses `A` or `B`, with a fallback to the first installed executable when the value is invalid.
- Verification uses the signed path when `manifest.sig` is present, otherwise it falls back to the `SHA256SUMS.txt` checksum path.
- The checksum path requires the manifest to exist and to carry a hash entry for the target executable, otherwise `ManifestMissing` or `ManifestHashMissing` is returned.
- The signed path loads the trust anchor, verifies the manifest signature, and verifies both the slot and recovery entry digests.
- Release counter ordering is three way, a manifest counter below the persisted value is rejected as a downgrade, an equal counter is accepted without rewriting the state file, and a greater counter yields a pending value.
- The pending release counter is committed only after a verified slot is committed for launch, so a failed selection or commit cannot advance the counter past the last bootable release.
- Crash-loop health tracking stores a per slot counter under `Data/state/slot-N-health.state`, with a threshold of three before `CrashLoopDetected` is raised. The counter increments with `saturating_add` and is persisted through `atomic_replace` staging plus replace rather than a plain write.
- A successful active slot launch clears its health state, while any failure increments the active slot and moves to the standby slot, which is the Tier 2 rollback.
- Tier 2 rollback records the decision in `Data/logs/launcher-rollback.log`, writes the standby as the active slot, clears standby health, and commits the counter.
- Tier 3 runs when both slots fail, restoring slot A from the golden master under `Recovery`, which must itself pass the `SHA256SUMS.txt` hash requirement, then clears both health states.
- After selection the launcher spawns `true-tick.exe` with the original arguments minus the launcher program name and returns 0 without waiting for the child to exit.
- `main` maps selection or spawn errors to a Windows message box plus a failure exit code, while a successful spawn exits with code 0.