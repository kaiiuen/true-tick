#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use tick_crypto::{
    check_release_counter, parse_signed_manifest, verify_entry_digest, verify_manifest,
    CryptoError, SignedManifest, TrustAnchor,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Slot {
    A,
    B,
}

impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }

    fn alternate(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

#[derive(Debug)]
enum SelectionError {
    MetadataRead(std::io::Error),
    InvalidMetadata(String),
    ExecutableMissing(PathBuf),
    ExecutableNotFile(PathBuf),
    ManifestMissing(PathBuf),
    ManifestRead(std::io::Error),
    ChecksumMismatch {
        target: String,
        expected: String,
        actual: String,
    },
    CrashLoopDetected {
        slot: Slot,
        consecutive_failures: u32,
    },
    BothSlotsCorrupted {
        active: String,
        standby: String,
    },
    HealthTrackingFailure,
    SignedManifestRead(std::io::Error),
    TrustAnchorMissing(PathBuf),
    TrustAnchorRead(std::io::Error),
    TrustAnchorInvalid(CryptoError),
    SignedManifestMalformed(CryptoError),
    SignatureVerificationFailed(CryptoError),
    ReleaseCounterStateFailure,
    ReleaseCounterRejected(CryptoError),
    SignedDigestMismatch {
        target: String,
        cause: CryptoError,
    },
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetadataRead(error) => {
                write!(formatter, "active slot metadata cannot be read: {error}")
            }
            Self::InvalidMetadata(value) => {
                write!(formatter, "active slot metadata is invalid: {value}")
            }
            Self::ExecutableMissing(path) => write!(
                formatter,
                "selected slot executable is missing: {}",
                path.display()
            ),
            Self::ExecutableNotFile(path) => write!(
                formatter,
                "selected slot executable is not a file: {}",
                path.display()
            ),
            Self::ManifestMissing(path) => write!(
                formatter,
                "package manifest SHA256SUMS.txt is missing: {}",
                path.display()
            ),
            Self::ManifestRead(error) => {
                write!(formatter, "package manifest SHA256SUMS.txt cannot be read: {error}")
            }
            Self::ChecksumMismatch { target, expected, actual } => write!(
                formatter,
                "integrity verification failed for {target}: expected {expected} but computed {actual}"
            ),
            Self::CrashLoopDetected { slot, consecutive_failures } => write!(
                formatter,
                "crash loop detected for slot {}: {consecutive_failures} consecutive failures",
                slot.name()
            ),
            Self::BothSlotsCorrupted { active, standby } => write!(
                formatter,
                "both slots failed verification and require package repair: active slot rejected with [{active}] and standby slot rejected with [{standby}]"
            ),
            Self::HealthTrackingFailure => {
                write!(formatter, "failed to read or update slot health tracking state")
            }
            Self::SignedManifestRead(error) => {
                write!(formatter, "signed manifest manifest.sig cannot be read: {error}")
            }
            Self::TrustAnchorMissing(path) => write!(
                formatter,
                "signed manifest is present but trust anchor is missing: {}",
                path.display()
            ),
            Self::TrustAnchorRead(error) => {
                write!(formatter, "trust anchor trust-anchor.pub cannot be read: {error}")
            }
            Self::TrustAnchorInvalid(cause) => {
                write!(formatter, "trust anchor trust-anchor.pub is invalid: {cause}")
            }
            Self::SignedManifestMalformed(cause) => {
                write!(formatter, "signed manifest is malformed: {cause}")
            }
            Self::SignatureVerificationFailed(cause) => write!(
                formatter,
                "signed manifest signature verification failed: {cause}"
            ),
            Self::ReleaseCounterStateFailure => write!(
                formatter,
                "failed to read or persist the release counter state"
            ),
            Self::ReleaseCounterRejected(cause) => write!(
                formatter,
                "signed manifest release counter rejected as replay or downgrade: {cause}"
            ),
            Self::SignedDigestMismatch { target, cause } => write!(
                formatter,
                "signed manifest digest verification failed for {target}: {cause}"
            ),
        }
    }
}

fn compute_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let hash = hasher.finalize();
    Ok(format!("{hash:x}"))
}

fn verify_slot_executable(root: &Path, slot: Slot) -> Result<PathBuf, SelectionError> {
    let executable = root.join("Slots").join(slot.name()).join("true-tick.exe");
    if !executable.exists() {
        return Err(SelectionError::ExecutableMissing(executable));
    }
    if !executable.is_file() {
        return Err(SelectionError::ExecutableNotFile(executable));
    }
    verify_slot_integrity(root, slot)?;
    Ok(executable)
}

fn verify_slot_integrity(root: &Path, slot: Slot) -> Result<(), SelectionError> {
    if prepare_signed_manifest(root, slot)?.is_some() {
        // prepare_signed_manifest already verified the slot and recovery
        // digests before it committed the release counter.
        return Ok(());
    }
    verify_slot_integrity_checksum(root, slot)
}

fn load_signed_manifest(root: &Path) -> Result<Option<SignedManifest>, SelectionError> {
    let manifest_path = root.join("manifest.sig");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&manifest_path).map_err(SelectionError::SignedManifestRead)?;
    parse_signed_manifest(&text)
        .map(Some)
        .map_err(SelectionError::SignedManifestMalformed)
}

fn load_trust_anchor(root: &Path) -> Result<TrustAnchor, SelectionError> {
    let anchor_path = root.join("trust-anchor.pub");
    if !anchor_path.exists() {
        return Err(SelectionError::TrustAnchorMissing(anchor_path));
    }
    let bytes = fs::read(&anchor_path).map_err(SelectionError::TrustAnchorRead)?;
    TrustAnchor::from_bytes(&bytes).map_err(SelectionError::TrustAnchorInvalid)
}

fn release_counter_state_path(root: &Path) -> PathBuf {
    root.join("Data")
        .join("state")
        .join("release-counter.state")
}

fn read_release_counter(root: &Path) -> Result<u64, SelectionError> {
    let state_path = release_counter_state_path(root);
    if !state_path.exists() {
        return Ok(0);
    }
    let content = fs::read_to_string(&state_path).map_err(|err| {
        eprintln!(
            "failed to read release counter at {}: {err}",
            state_path.display()
        );
        SelectionError::ReleaseCounterStateFailure
    })?;
    content.trim().parse::<u64>().map_err(|err| {
        eprintln!(
            "failed to parse release counter at {}: {err}",
            state_path.display()
        );
        SelectionError::ReleaseCounterStateFailure
    })
}

fn persist_release_counter(root: &Path, counter: u64) -> Result<(), SelectionError> {
    let state_dir = root.join("Data").join("state");
    fs::create_dir_all(&state_dir).map_err(|err| {
        eprintln!(
            "failed to create release counter directory {}: {err}",
            state_dir.display()
        );
        SelectionError::ReleaseCounterStateFailure
    })?;
    let state_path = release_counter_state_path(root);
    let pid = std::process::id();
    let staging = state_dir.join(format!("release-counter.state.staging.{pid}.tmp"));
    fs::write(&staging, format!("{counter}\n")).map_err(|err| {
        eprintln!(
            "failed to stage release counter at {}: {err}",
            staging.display()
        );
        SelectionError::ReleaseCounterStateFailure
    })?;
    atomic_replace(&staging, &state_path)?;
    let _ = fs::remove_file(&staging);
    Ok(())
}

/// Loads, signature-verifies, and counter-checks the signed manifest when one
/// is present. The selected slot executable and the recovery executable
/// digests are verified before the new release counter is committed, so a
/// failed digest check leaves the previous counter on disk and preserves
/// rollback capability.
///
/// Returns `None` when no `manifest.sig` is present so the caller can retain
/// the existing SHA256SUMS.txt behaviour. Signature verification is not
/// performed in that case because no signing tool exists yet to produce the
/// artifact, so absence is a diagnostic rather than a failure.
fn prepare_signed_manifest(
    root: &Path,
    slot: Slot,
) -> Result<Option<SignedManifest>, SelectionError> {
    let Some(manifest) = load_signed_manifest(root)? else {
        eprintln!(
            "diagnostic: manifest.sig absent under {}, signature verification not performed, \
             falling back to SHA256SUMS.txt",
            root.display()
        );
        return Ok(None);
    };
    let anchor = load_trust_anchor(root)?;
    verify_manifest(&manifest, &anchor).map_err(SelectionError::SignatureVerificationFailed)?;
    let previously_seen = read_release_counter(root)?;
    check_release_counter(&manifest, previously_seen)
        .map_err(SelectionError::ReleaseCounterRejected)?;
    // Commit ordering matters. Verifying the payload before the counter is
    // written keeps the on disk counter at the last release that proved valid,
    // which is what makes an autonomous rollback possible.
    verify_slot_integrity_signed(root, slot, &manifest)?;
    verify_recovery_integrity_signed(root, &manifest)?;
    persist_release_counter(root, manifest.release_counter)?;
    Ok(Some(manifest))
}

const RECOVERY_EXECUTABLE_REL: &str = "Recovery/true-tick.exe";

/// Verifies the recovery golden master digest when the signed manifest lists
/// it. A manifest without a recovery entry is tolerated so packages that only
/// authorize slot executables keep working.
fn verify_recovery_integrity_signed(
    root: &Path,
    manifest: &SignedManifest,
) -> Result<(), SelectionError> {
    let authorized = manifest
        .entries
        .iter()
        .any(|entry| entry.path == RECOVERY_EXECUTABLE_REL);
    if !authorized {
        return Ok(());
    }
    let recovery_path = root.join("Recovery").join("true-tick.exe");
    let bytes = fs::read(&recovery_path).map_err(SelectionError::MetadataRead)?;
    verify_entry_digest(manifest, RECOVERY_EXECUTABLE_REL, &bytes).map_err(|cause| {
        SelectionError::SignedDigestMismatch {
            target: RECOVERY_EXECUTABLE_REL.to_owned(),
            cause,
        }
    })
}

fn verify_slot_integrity_signed(
    root: &Path,
    slot: Slot,
    manifest: &SignedManifest,
) -> Result<(), SelectionError> {
    let executable_path = root.join("Slots").join(slot.name()).join("true-tick.exe");
    let bytes = fs::read(&executable_path).map_err(SelectionError::MetadataRead)?;
    let target_rel_path = format!("Slots/{}/true-tick.exe", slot.name());
    verify_entry_digest(manifest, &target_rel_path, &bytes).map_err(|cause| {
        SelectionError::SignedDigestMismatch {
            target: target_rel_path,
            cause,
        }
    })
}

fn verify_slot_integrity_checksum(root: &Path, slot: Slot) -> Result<(), SelectionError> {
    let manifest_path = root.join("SHA256SUMS.txt");
    if !manifest_path.exists() {
        return Err(SelectionError::ManifestMissing(manifest_path));
    }
    let manifest_content =
        fs::read_to_string(&manifest_path).map_err(SelectionError::ManifestRead)?;
    let target_rel_path = format!("Slots/{}/true-tick.exe", slot.name());
    let normalized_rel_path = target_rel_path.replace('/', "\\");

    let mut expected_hash = None;
    for line in manifest_content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(hash), Some(filename)) = (parts.next(), parts.next()) {
            if filename == target_rel_path || filename == normalized_rel_path {
                expected_hash = Some(hash.to_lowercase());
                break;
            }
        }
    }

    if let Some(expected) = expected_hash {
        let executable_path = root.join("Slots").join(slot.name()).join("true-tick.exe");
        let actual = compute_sha256(&executable_path).map_err(SelectionError::MetadataRead)?;
        if actual.to_lowercase() != expected {
            return Err(SelectionError::ChecksumMismatch {
                target: target_rel_path,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

fn read_slot_health(root: &Path, slot: Slot) -> Result<u32, SelectionError> {
    let health_path = root
        .join("Data")
        .join("state")
        .join(format!("slot-{}-health.state", slot.name()));
    if !health_path.exists() {
        return Ok(0);
    }
    let content = fs::read_to_string(&health_path).map_err(|err| {
        eprintln!(
            "failed to read slot health at {}: {err}",
            health_path.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    content.trim().parse::<u32>().map_err(|err| {
        eprintln!(
            "failed to parse slot health content at {}: {err}",
            health_path.display()
        );
        SelectionError::HealthTrackingFailure
    })
}

fn increment_slot_health(root: &Path, slot: Slot) -> Result<u32, SelectionError> {
    let state_dir = root.join("Data").join("state");
    fs::create_dir_all(&state_dir).map_err(|err| {
        eprintln!(
            "failed to create health state directory {}: {err}",
            state_dir.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    let count = read_slot_health(root, slot)? + 1;
    let health_path = state_dir.join(format!("slot-{}-health.state", slot.name()));
    fs::write(&health_path, format!("{count}\n")).map_err(|err| {
        eprintln!(
            "failed to write slot health at {}: {err}",
            health_path.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    Ok(count)
}

fn clear_slot_health(root: &Path, slot: Slot) -> Result<(), SelectionError> {
    let health_path = root
        .join("Data")
        .join("state")
        .join(format!("slot-{}-health.state", slot.name()));
    if health_path.exists() {
        fs::remove_file(&health_path).map_err(|err| {
            eprintln!(
                "failed to clear slot health at {}: {err}",
                health_path.display()
            );
            SelectionError::HealthTrackingFailure
        })?;
    }
    Ok(())
}

const BOOT_FAILURE_THRESHOLD: u32 = 3;

fn verify_golden_master(root: &Path) -> Result<PathBuf, SelectionError> {
    let executable = root.join("Recovery").join("true-tick.exe");
    if !executable.exists() {
        return Err(SelectionError::ExecutableMissing(executable));
    }
    if !executable.is_file() {
        return Err(SelectionError::ExecutableNotFile(executable));
    }
    let manifest_path = root.join("SHA256SUMS.txt");
    if !manifest_path.exists() {
        return Err(SelectionError::ManifestMissing(manifest_path));
    }
    let manifest_content =
        fs::read_to_string(&manifest_path).map_err(SelectionError::ManifestRead)?;
    let target_rel_path = "Recovery/true-tick.exe";
    let normalized_rel_path = target_rel_path.replace('/', "\\");
    let mut expected_hash = None;
    for line in manifest_content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(hash), Some(filename)) = (parts.next(), parts.next()) {
            if filename == target_rel_path || filename == normalized_rel_path {
                expected_hash = Some(hash.to_lowercase());
                break;
            }
        }
    }
    if let Some(expected) = expected_hash {
        let actual = compute_sha256(&executable).map_err(SelectionError::MetadataRead)?;
        if actual.to_lowercase() != expected {
            return Err(SelectionError::ChecksumMismatch {
                target: target_rel_path.to_owned(),
                expected,
                actual,
            });
        }
    }
    Ok(executable)
}

fn record_golden_master_restoration(root: &Path, target: Slot) -> Result<(), SelectionError> {
    let logs = root.join("Data").join("logs");
    fs::create_dir_all(&logs).map_err(|err| {
        eprintln!(
            "failed to create rollback log directory {}: {err}",
            logs.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    let entry = format!(
        "event=autonomous_golden_master_restoration target=Slot::{} status=restored\n",
        target.name()
    );
    let path = logs.join("launcher-rollback.log");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| {
            eprintln!(
                "failed to open launcher rollback log at {}: {err}",
                path.display()
            );
            SelectionError::HealthTrackingFailure
        })?;
    std::io::Write::write_all(&mut file, entry.as_bytes()).map_err(|err| {
        eprintln!(
            "failed to append to launcher rollback log at {}: {err}",
            path.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    Ok(())
}

fn restore_from_golden_master(root: &Path) -> Result<PathBuf, SelectionError> {
    let recovery = root.join("Recovery");
    let golden_executable = verify_golden_master(root)?;
    let golden_config = recovery.join("true-tick.toml");
    if !golden_config.is_file() {
        return Err(SelectionError::ExecutableMissing(golden_config));
    }
    let slot_dir = root.join("Slots").join(Slot::A.name());
    fs::create_dir_all(&slot_dir).map_err(SelectionError::MetadataRead)?;
    let executable = slot_dir.join("true-tick.exe");
    let pid = std::process::id();
    let staging = slot_dir.join(format!("true-tick.exe.staging.{pid}.tmp"));
    fs::copy(&golden_executable, &staging).map_err(SelectionError::MetadataRead)?;
    let staging_hash = compute_sha256(&staging).map_err(SelectionError::MetadataRead)?;
    let golden_source_hash =
        compute_sha256(&golden_executable).map_err(SelectionError::MetadataRead)?;
    if staging_hash != golden_source_hash {
        let _ = fs::remove_file(&staging);
        return Err(SelectionError::ChecksumMismatch {
            target: "Recovery/true-tick.exe".to_owned(),
            expected: golden_source_hash,
            actual: staging_hash,
        });
    }
    atomic_replace(&staging, &executable)?;
    let _ = fs::remove_file(&staging);
    fs::copy(&golden_config, slot_dir.join("true-tick.toml"))
        .map_err(SelectionError::MetadataRead)?;
    verify_slot_executable(root, Slot::A)?;
    record_golden_master_restoration(root, Slot::A)?;
    write_active_slot(root, Slot::A)?;
    Ok(executable)
}

#[cfg(windows)]
fn atomic_replace(staging: &Path, target: &Path) -> Result<(), SelectionError> {
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = OsStr::new(staging)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to: Vec<u16> = OsStr::new(target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        let _ = fs::remove_file(staging);
        return Err(SelectionError::MetadataRead(err));
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(staging: &Path, target: &Path) -> Result<(), SelectionError> {
    fs::rename(staging, target).map_err(SelectionError::MetadataRead)
}

fn write_active_slot(root: &Path, slot: Slot) -> Result<(), SelectionError> {
    let metadata = root.join("active-slot.txt");
    let pid = std::process::id();
    let staging_meta = root.join(format!("active-slot.txt.staging.{pid}.tmp"));
    fs::write(&staging_meta, format!("{}\n", slot.name())).map_err(SelectionError::MetadataRead)?;
    atomic_replace(&staging_meta, &metadata)?;
    let _ = fs::remove_file(&staging_meta);
    Ok(())
}

fn record_rollback_decision(
    root: &Path,
    from: Slot,
    to: Slot,
    cause: &SelectionError,
) -> Result<(), SelectionError> {
    let logs = root.join("Data").join("logs");
    fs::create_dir_all(&logs).map_err(|err| {
        eprintln!(
            "failed to create rollback log directory {}: {err}",
            logs.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    let entry = format!(
        "event=autonomous_slot_rollback from={} to={} cause={}\n",
        from.name(),
        to.name(),
        cause
    );
    let path = logs.join("launcher-rollback.log");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| {
            eprintln!(
                "failed to open launcher rollback log at {}: {err}",
                path.display()
            );
            SelectionError::HealthTrackingFailure
        })?;
    std::io::Write::write_all(&mut file, entry.as_bytes()).map_err(|err| {
        eprintln!(
            "failed to append to launcher rollback log at {}: {err}",
            path.display()
        );
        SelectionError::HealthTrackingFailure
    })?;
    Ok(())
}

/// Normalizes raw `active-slot.txt` bytes by dropping a UTF-8 BOM, CRLF
/// terminators, surrounding whitespace, and null bytes before the value is
/// interpreted.
fn normalize_active_slot(raw: &str) -> String {
    raw.trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .trim_matches('\u{feff}')
        .trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_owned()
}

fn parse_active_slot(raw: &str) -> Option<Slot> {
    match normalize_active_slot(raw).as_str() {
        "A" => Some(Slot::A),
        "B" => Some(Slot::B),
        _ => None,
    }
}

/// Last resort selector used when `active-slot.txt` cannot be interpreted.
/// Slot A wins when both slot executables are present so startup is
/// deterministic.
fn fallback_slot_from_installed_executables(root: &Path) -> Option<Slot> {
    [Slot::A, Slot::B].into_iter().find(|slot| {
        root.join("Slots")
            .join(slot.name())
            .join("true-tick.exe")
            .exists()
    })
}

fn select(root: &Path) -> Result<(Slot, PathBuf), SelectionError> {
    let metadata = root.join("active-slot.txt");
    let raw = fs::read_to_string(&metadata).map_err(SelectionError::MetadataRead)?;
    let slot = match parse_active_slot(&raw) {
        Some(slot) => slot,
        None => {
            let invalid = normalize_active_slot(&raw);
            match fallback_slot_from_installed_executables(root) {
                Some(slot) => {
                    eprintln!(
                        "warning: active-slot.txt value {invalid:?} is invalid, falling back to slot {}",
                        slot.name()
                    );
                    slot
                }
                None => return Err(SelectionError::InvalidMetadata(invalid)),
            }
        }
    };

    let verify_and_check_slot = |target_slot: Slot| -> Result<PathBuf, SelectionError> {
        let failures = read_slot_health(root, target_slot)?;
        if failures >= BOOT_FAILURE_THRESHOLD {
            return Err(SelectionError::CrashLoopDetected {
                slot: target_slot,
                consecutive_failures: failures,
            });
        }
        verify_slot_executable(root, target_slot)
    };

    match verify_and_check_slot(slot) {
        Ok(executable) => {
            clear_slot_health(root, slot)?;
            Ok((slot, executable))
        }
        Err(active_error) => {
            if matches!(active_error, SelectionError::HealthTrackingFailure) {
                return Err(active_error);
            }
            increment_slot_health(root, slot)?;
            let standby = slot.alternate();
            match verify_and_check_slot(standby) {
                Ok(executable) => {
                    record_rollback_decision(root, slot, standby, &active_error)?;
                    write_active_slot(root, standby)?;
                    clear_slot_health(root, standby)?;
                    Ok((standby, executable))
                }
                Err(standby_error) => {
                    if matches!(standby_error, SelectionError::HealthTrackingFailure) {
                        return Err(standby_error);
                    }
                    increment_slot_health(root, standby)?;
                    match restore_from_golden_master(root) {
                        Ok(executable) => {
                            clear_slot_health(root, Slot::A)?;
                            clear_slot_health(root, Slot::B)?;
                            Ok((Slot::A, executable))
                        }
                        Err(_) => Err(SelectionError::BothSlotsCorrupted {
                            active: active_error.to_string(),
                            standby: standby_error.to_string(),
                        }),
                    }
                }
            }
        }
    }
}

fn workspace_root(launcher: &Path) -> Option<&Path> {
    launcher.parent()
}

fn repair_reason(root: &Path, error: &str) -> String {
    format!(
        "True\u{2122} Tick launcher repair required: {error}\n\nPackage root: {}",
        root.display()
    )
}

fn run() -> Result<i32, String> {
    let launcher = std::env::current_exe().map_err(|error| error.to_string())?;
    let root = workspace_root(&launcher)
        .ok_or_else(|| "Launcher.exe has no workspace root".to_owned())?
        .to_owned();
    let (slot, executable) =
        select(&root).map_err(|error| repair_reason(&root, &error.to_string()))?;
    Command::new(&executable)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|error| {
            repair_reason(
                &root,
                &format!("selected slot {slot:?} could not be launched: {error}"),
            )
        })?;
    Ok(0)
}

#[cfg(windows)]
mod dialog {
    use std::ffi::c_void;

    const MB_OK: u32 = 0x0000_0000;
    const MB_ICONERROR: u32 = 0x0000_0010;
    const MB_SETFOREGROUND: u32 = 0x0001_0000;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, title: *const u16, kind: u32) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(crate) fn show_error(reason: &str) {
        let text = wide(reason);
        let title = wide("True Tick launcher");
        // SAFETY: MessageBoxW accepts a null owner and valid null terminated
        // UTF-16 buffers that outlive the call
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("{error}");
            #[cfg(windows)]
            dialog::show_error(&error);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("true-tick-launcher-{name}-{}", std::process::id()))
    }

    #[test]
    fn repair_reason_includes_the_package_root_and_cause() {
        let path = Path::new("C:\\packages\\true-tick");
        let reason = repair_reason(path, "active slot metadata is invalid: C");
        assert!(reason.contains("repair required: active slot metadata is invalid: C"));
        assert!(reason.contains("Package root: C:\\packages\\true-tick"));
    }

    #[test]
    fn missing_metadata_is_an_error() {
        let path = root("missing-metadata");
        fs::create_dir_all(&path).unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::MetadataRead(_))
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn invalid_metadata_is_an_error() {
        let path = root("invalid-metadata");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("active-slot.txt"), "C\n").unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::InvalidMetadata(_))
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn selected_slot_must_contain_the_expected_executable() {
        let path = root("missing-executable");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::BothSlotsCorrupted { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn valid_metadata_selects_only_the_active_slot() {
        let path = root("valid");
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::write(path.join("active-slot.txt"), "B\n").unwrap();
        let executable = path.join("Slots").join("B").join("true-tick.exe");
        fs::write(&executable, b"fixture").unwrap();
        let hash = compute_sha256(&executable).unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            format!("{hash}  Slots/B/true-tick.exe\n"),
        )
        .unwrap();
        assert_eq!(select(&path).unwrap().0, Slot::B);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_manifest_is_a_hard_failure() {
        let path = root("missing-manifest");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(
            path.join("Slots").join("A").join("true-tick.exe"),
            b"fixture",
        )
        .unwrap();
        assert!(matches!(
            verify_slot_integrity(&path, Slot::A),
            Err(SelectionError::ManifestMissing(_))
        ));
        assert!(matches!(
            select(&path),
            Err(SelectionError::BothSlotsCorrupted { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn manifest_checksum_mismatch_fails_selection() {
        let path = root("checksum-mismatch");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(
            path.join("Slots").join("A").join("true-tick.exe"),
            b"tampered",
        )
        .unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  Slots/A/true-tick.exe\n",
        )
        .unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::BothSlotsCorrupted { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_autonomous_rollback_when_active_slot_corrupted() {
        let path = root("rollback");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(
            path.join("Slots").join("A").join("true-tick.exe"),
            b"tampered",
        )
        .unwrap();
        fs::write(
            path.join("Slots").join("B").join("true-tick.exe"),
            b"fixture",
        )
        .unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  Slots/A/true-tick.exe\n",
        )
        .unwrap();
        let (slot, _) = select(&path).unwrap();
        assert_eq!(slot, Slot::B);
        assert_eq!(
            fs::read_to_string(path.join("active-slot.txt")).unwrap(),
            "B\n"
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_both_slots_corrupted_fails_with_explicit_error() {
        let path = root("both-corrupted");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::BothSlotsCorrupted { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_golden_master_autonomous_reconstruction() {
        let path = root("golden-master");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::create_dir_all(path.join("Recovery")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(path.join("Recovery").join("true-tick.exe"), b"golden").unwrap();
        fs::write(
            path.join("Recovery").join("true-tick.toml"),
            "automatic = false\n",
        )
        .unwrap();
        let golden_hash = compute_sha256(&path.join("Recovery").join("true-tick.exe")).unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            format!(
                "{golden_hash}  Recovery/true-tick.exe\n{golden_hash}  Slots/A/true-tick.exe\n"
            ),
        )
        .unwrap();
        let (slot, executable) = select(&path).unwrap();
        assert_eq!(slot, Slot::A);
        assert_eq!(
            executable,
            path.join("Slots").join("A").join("true-tick.exe")
        );
        assert_eq!(
            fs::read_to_string(path.join("Slots").join("A").join("true-tick.exe")).unwrap(),
            "golden"
        );
        assert_eq!(
            fs::read_to_string(path.join("Slots").join("A").join("true-tick.toml")).unwrap(),
            "automatic = false\n"
        );
        assert_eq!(
            fs::read_to_string(path.join("active-slot.txt")).unwrap(),
            "A\n"
        );
        let log = fs::read_to_string(path.join("Data").join("logs").join("launcher-rollback.log"))
            .unwrap();
        assert!(log
            .contains("event=autonomous_golden_master_restoration target=Slot::A status=restored"));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_crash_loop_triggers_autonomous_rollback() {
        let path = root("crash-loop-rollback");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::create_dir_all(path.join("Data").join("state")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(
            path.join("Slots").join("A").join("true-tick.exe"),
            b"fixture_a",
        )
        .unwrap();
        let b_exe = path.join("Slots").join("B").join("true-tick.exe");
        fs::write(&b_exe, b"fixture_b").unwrap();
        fs::write(
            path.join("Data").join("state").join("slot-A-health.state"),
            "3\n",
        )
        .unwrap();
        let b_hash = compute_sha256(&b_exe).unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            format!("{b_hash}  Slots/B/true-tick.exe\n"),
        )
        .unwrap();

        let (slot, _) = select(&path).unwrap();
        assert_eq!(slot, Slot::B);
        assert_eq!(
            fs::read_to_string(path.join("active-slot.txt")).unwrap(),
            "B\n"
        );
        assert!(!path
            .join("Data")
            .join("state")
            .join("slot-B-health.state")
            .exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_dual_slot_crash_loop_triggers_golden_master() {
        let path = root("crash-loop-golden");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::create_dir_all(path.join("Recovery")).unwrap();
        fs::create_dir_all(path.join("Data").join("state")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(
            path.join("Slots").join("A").join("true-tick.exe"),
            b"fixture_a",
        )
        .unwrap();
        fs::write(
            path.join("Slots").join("B").join("true-tick.exe"),
            b"fixture_b",
        )
        .unwrap();
        fs::write(
            path.join("Data").join("state").join("slot-A-health.state"),
            "3\n",
        )
        .unwrap();
        fs::write(
            path.join("Data").join("state").join("slot-B-health.state"),
            "3\n",
        )
        .unwrap();
        fs::write(path.join("Recovery").join("true-tick.exe"), b"golden").unwrap();
        fs::write(
            path.join("Recovery").join("true-tick.toml"),
            "automatic = false\n",
        )
        .unwrap();
        let golden_hash = compute_sha256(&path.join("Recovery").join("true-tick.exe")).unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            format!(
                "{golden_hash}  Recovery/true-tick.exe\n{golden_hash}  Slots/A/true-tick.exe\n"
            ),
        )
        .unwrap();

        let (slot, executable) = select(&path).unwrap();
        assert_eq!(slot, Slot::A);
        assert_eq!(
            executable,
            path.join("Slots").join("A").join("true-tick.exe")
        );
        assert_eq!(
            fs::read_to_string(path.join("Slots").join("A").join("true-tick.exe")).unwrap(),
            "golden"
        );
        assert_eq!(
            fs::read_to_string(path.join("active-slot.txt")).unwrap(),
            "A\n"
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_dual_slot_failure_without_recovery_fails_with_explicit_error() {
        let path = root("no-recovery");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Slots").join("B")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::BothSlotsCorrupted { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_golden_master_missing_manifest_fails_with_manifest_missing() {
        let path = root("golden-no-manifest");
        fs::create_dir_all(path.join("Recovery")).unwrap();
        fs::write(path.join("Recovery").join("true-tick.exe"), b"golden").unwrap();
        assert!(matches!(
            verify_golden_master(&path),
            Err(SelectionError::ManifestMissing(_))
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_successful_slot_selection_clears_health_state() {
        let path = root("clear-health-on-success");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Data").join("state")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        let exe = path.join("Slots").join("A").join("true-tick.exe");
        fs::write(&exe, b"valid_exe").unwrap();
        let hash = compute_sha256(&exe).unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            format!("{hash}  Slots/A/true-tick.exe\n"),
        )
        .unwrap();
        let health_file = path.join("Data").join("state").join("slot-A-health.state");
        fs::write(&health_file, "2\n").unwrap();
        assert!(health_file.exists());
        let (slot, _) = select(&path).unwrap();
        assert_eq!(slot, Slot::A);
        assert!(!health_file.exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn test_health_tracking_corrupted_counter_returns_health_tracking_failure() {
        let path = root("health-corrupted-counter");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::create_dir_all(path.join("Data").join("state")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        fs::write(
            path.join("Data").join("state").join("slot-A-health.state"),
            "not-a-number\n",
        )
        .unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::HealthTrackingFailure)
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(test)]
    fn write_signed_package(
        path: &Path,
        slot: Slot,
        payload: &[u8],
        counter: u64,
    ) -> (ed25519_dalek::SigningKey, PathBuf) {
        use ed25519_dalek::Signer;
        fs::create_dir_all(path.join("Slots").join(slot.name())).unwrap();
        fs::write(path.join("active-slot.txt"), format!("{}\n", slot.name())).unwrap();
        let executable = path.join("Slots").join(slot.name()).join("true-tick.exe");
        fs::write(&executable, payload).unwrap();
        let digest = Sha256::digest(payload);
        let mut hex = String::with_capacity(64);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        let unsigned = format!(
            "release_counter {counter}\nentry Slots/{}/true-tick.exe {hex}\n",
            slot.name()
        );
        let signing = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
        let signature = signing.sign(unsigned.as_bytes());
        let mut sig_hex = String::with_capacity(128);
        for byte in signature.to_bytes() {
            sig_hex.push_str(&format!("{byte:02x}"));
        }
        fs::write(
            path.join("manifest.sig"),
            format!("{unsigned}signature {sig_hex}\n"),
        )
        .unwrap();
        fs::write(
            path.join("trust-anchor.pub"),
            signing.verifying_key().to_bytes(),
        )
        .unwrap();
        (signing, executable)
    }

    #[test]
    fn signed_manifest_accepts_valid_package() {
        let path = root("signed-valid");
        write_signed_package(&path, Slot::A, b"signed_fixture", 1);
        let (slot, executable) = select(&path).unwrap();
        assert_eq!(slot, Slot::A);
        assert_eq!(
            executable,
            path.join("Slots").join("A").join("true-tick.exe")
        );
        assert_eq!(
            fs::read_to_string(
                path.join("Data")
                    .join("state")
                    .join("release-counter.state")
            )
            .unwrap(),
            "1\n"
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn signed_manifest_rejects_tampered_binary() {
        let path = root("signed-tampered");
        write_signed_package(&path, Slot::A, b"signed_fixture", 1);
        fs::write(
            path.join("Slots").join("A").join("true-tick.exe"),
            b"tampered_payload",
        )
        .unwrap();
        assert!(matches!(
            select(&path),
            Err(SelectionError::BothSlotsCorrupted { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_signed_manifest_falls_back_to_checksum_manifest() {
        let path = root("signed-fallback");
        fs::create_dir_all(path.join("Slots").join("A")).unwrap();
        fs::write(path.join("active-slot.txt"), "A\n").unwrap();
        let executable = path.join("Slots").join("A").join("true-tick.exe");
        fs::write(&executable, b"fixture").unwrap();
        let hash = compute_sha256(&executable).unwrap();
        fs::write(
            path.join("SHA256SUMS.txt"),
            format!("{hash}  Slots/A/true-tick.exe\n"),
        )
        .unwrap();
        let (slot, _) = select(&path).unwrap();
        assert_eq!(slot, Slot::A);
        assert!(!path.join("manifest.sig").exists());
        assert!(!path
            .join("Data")
            .join("state")
            .join("release-counter.state")
            .exists());
        fs::remove_dir_all(path).unwrap();
    }
}
