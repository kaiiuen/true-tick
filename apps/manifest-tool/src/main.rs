//! Producer tool for Ed25519 signed release manifests.
//!
//! Generates trust anchors and signing keys, signs package directories into
//! canonical `manifest.sig` files consumable by `tick-crypto`, and verifies
//! signed packages against a trust anchor.

#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tick_crypto::{
    generate_keypair, parse_signed_manifest, sign_manifest, verify_entry_digest, verify_manifest,
    CryptoError, ManifestEntry, TrustAnchor, MAX_MANIFEST_ENTRIES, MAX_MANIFEST_LINE_BYTES,
    PUBLIC_KEY_LEN, SECRET_KEY_LEN,
};

/// Maximum byte length of a single file accepted for hashing.
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

/// Byte overhead of one `entry <path> <hex>` line excluding the path.
const ENTRY_LINE_OVERHEAD: usize = "entry ".len() + 1 + 64;

/// Errors produced by the manifest tool.
#[derive(Debug)]
enum ToolError {
    Usage(String),
    Io {
        context: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Crypto {
        context: &'static str,
        source: CryptoError,
    },
    Exists(PathBuf),
    TooManyEntries(usize),
    LineTooLong(String),
    EntryTooLarge {
        path: String,
        len: u64,
    },
    InvalidPath(PathBuf),
    VerifyFailed(Vec<String>),
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "error: {message}"),
            Self::Io {
                context,
                path,
                source,
            } => write!(formatter, "error: {context} {}: {source}", path.display()),
            Self::Crypto { context, source } => {
                write!(formatter, "error: {context}: {source}")
            }
            Self::Exists(path) => write!(
                formatter,
                "error: refusing to overwrite existing file {} without --force",
                path.display()
            ),
            Self::TooManyEntries(count) => write!(
                formatter,
                "error: package contains {count} files which exceeds \
                 MAX_MANIFEST_ENTRIES of {MAX_MANIFEST_ENTRIES}"
            ),
            Self::LineTooLong(path) => write!(
                formatter,
                "error: manifest entry line for {path} exceeds \
                 MAX_MANIFEST_LINE_BYTES of {MAX_MANIFEST_LINE_BYTES}"
            ),
            Self::EntryTooLarge { path, len } => write!(
                formatter,
                "error: file {path} is {len} bytes which exceeds \
                 MAX_ENTRY_BYTES of {MAX_ENTRY_BYTES}"
            ),
            Self::InvalidPath(path) => write!(
                formatter,
                "error: path {} cannot be represented in the manifest",
                path.display()
            ),
            Self::VerifyFailed(failures) => {
                write!(formatter, "verification failed:\n{}", failures.join("\n"))
            }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, ToolError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(ToolError::Usage(
            "missing subcommand, expected gen-key, sign, or verify".to_owned(),
        ));
    };
    match command.as_str() {
        "gen-key" => cmd_gen_key(rest),
        "sign" => cmd_sign(rest),
        "verify" => cmd_verify(rest),
        other => Err(ToolError::Usage(format!("unknown subcommand {other}"))),
    }
}

fn take_value<'a>(args: &'a [String], flag: &str) -> Result<&'a str, ToolError> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| ToolError::Usage(format!("missing required argument {flag}")))?;
    args.get(index + 1)
        .filter(|value| !value.starts_with("--"))
        .map(String::as_str)
        .ok_or_else(|| ToolError::Usage(format!("missing value for {flag}")))
}

fn take_optional_value<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, ToolError> {
    match args.iter().position(|arg| arg == flag) {
        None => Ok(None),
        Some(index) => args
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .map(|value| Some(value.as_str()))
            .ok_or_else(|| ToolError::Usage(format!("missing value for {flag}"))),
    }
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_hex_lower<const N: usize>(text: &str) -> Result<[u8; N], CryptoError> {
    let trimmed = text.trim();
    if trimmed.len() != N * 2 {
        return Err(CryptoError::MalformedManifest);
    }
    let mut out = [0u8; N];
    for (index, pair) in trimmed.as_bytes().chunks_exact(2).enumerate() {
        let high = (pair[0] as char)
            .to_digit(16)
            .ok_or(CryptoError::MalformedManifest)?;
        let low = (pair[1] as char)
            .to_digit(16)
            .ok_or(CryptoError::MalformedManifest)?;
        out[index] = ((high << 4) | low) as u8;
    }
    Ok(out)
}

/// Loads a key file that stores either raw key bytes or lowercase hex text.
fn load_key_bytes<const N: usize>(path: &Path) -> Result<[u8; N], ToolError> {
    let bytes = fs::read(path).map_err(|source| ToolError::Io {
        context: "cannot read key file",
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() == N {
        let mut out = [0u8; N];
        out.copy_from_slice(&bytes);
        return Ok(out);
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CryptoError::WrongKeyLength)
        .map_err(|source| ToolError::Crypto {
            context: "key file is not valid hex text",
            source,
        })?;
    decode_hex_lower::<N>(text).map_err(|source| ToolError::Crypto {
        context: "key file is not valid hex text",
        source,
    })
}

fn load_secret_key(path: &Path) -> Result<[u8; SECRET_KEY_LEN], ToolError> {
    load_key_bytes::<SECRET_KEY_LEN>(path)
}

fn load_anchor(path: &Path) -> Result<TrustAnchor, ToolError> {
    let key = load_key_bytes::<PUBLIC_KEY_LEN>(path)?;
    TrustAnchor::from_bytes(&key).map_err(|source| ToolError::Crypto {
        context: "trust anchor is invalid",
        source,
    })
}

fn cmd_gen_key(args: &[String]) -> Result<String, ToolError> {
    let out_dir = PathBuf::from(take_value(args, "--out-dir")?);
    let force = has_flag(args, "--force");
    fs::create_dir_all(&out_dir).map_err(|source| ToolError::Io {
        context: "cannot create output directory",
        path: out_dir.clone(),
        source,
    })?;
    let anchor_path = out_dir.join("trust-anchor.pub");
    let secret_path = out_dir.join("signing-key.sec");
    if !force {
        for path in [&anchor_path, &secret_path] {
            if path.exists() {
                return Err(ToolError::Exists(path.clone()));
            }
        }
    }
    let (secret, public) = generate_keypair();
    write_key_file(&anchor_path, &public, false)?;
    write_key_file(&secret_path, &secret, true)?;
    Ok(format!(
        "generated Ed25519 keypair\n  trust anchor: {}\n  signing key: {}",
        anchor_path.display(),
        secret_path.display()
    ))
}

fn write_key_file(path: &Path, key_bytes: &[u8], secret: bool) -> Result<(), ToolError> {
    let mut text = hex_lower(key_bytes);
    text.push('\n');
    fs::write(path, text).map_err(|source| ToolError::Io {
        context: "cannot write key file",
        path: path.to_path_buf(),
        source,
    })?;
    if secret {
        restrict_secret_permissions(path);
    }
    Ok(())
}

#[cfg(windows)]
fn restrict_secret_permissions(path: &Path) {
    let outcome = std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg("*S-1-3-4:(R,W)")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match outcome {
        Ok(status) if status.success() => {}
        _ => eprintln!(
            "warning: could not restrict permissions on {}",
            path.display()
        ),
    }
}

#[cfg(all(unix, not(windows)))]
fn restrict_secret_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if fs::set_permissions(path, fs::Permissions::from_mode(0o600)).is_err() {
        eprintln!(
            "warning: could not restrict permissions on {}",
            path.display()
        );
    }
}

#[cfg(not(any(windows, unix)))]
fn restrict_secret_permissions(path: &Path) {
    eprintln!(
        "warning: could not restrict permissions on {}",
        path.display()
    );
}

fn collect_files(dir: &Path, root: &Path, files: &mut Vec<PathBuf>) -> Result<(), ToolError> {
    let mut children = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| ToolError::Io {
        context: "cannot read directory",
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ToolError::Io {
            context: "cannot read directory entry",
            path: dir.to_path_buf(),
            source,
        })?;
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        let file_type = fs::symlink_metadata(&child)
            .map_err(|source| ToolError::Io {
                context: "cannot stat path",
                path: child.clone(),
                source,
            })?
            .file_type();
        if file_type.is_dir() {
            collect_files(&child, root, files)?;
        } else if file_type.is_file() {
            files.push(child.strip_prefix(root).unwrap_or(&child).to_path_buf());
        }
    }
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn is_excluded(package_dir: &Path, relative: &Path, out_path: &Path) -> bool {
    if let Some(name) = relative.file_name().and_then(|name| name.to_str()) {
        if name == "manifest.sig" || name == "SHA256SUMS.txt" {
            return true;
        }
    }
    same_path(&package_dir.join(relative), out_path)
}

fn validate_manifest_path(path: &str) -> Result<(), ToolError> {
    if path.is_empty() || path.contains(char::is_whitespace) || path.contains(char::is_control) {
        return Err(ToolError::InvalidPath(PathBuf::from(path)));
    }
    Ok(())
}

fn hash_file(path: &Path, relative: &str) -> Result<[u8; 32], ToolError> {
    let metadata = fs::metadata(path).map_err(|source| ToolError::Io {
        context: "cannot stat file",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_ENTRY_BYTES {
        return Err(ToolError::EntryTooLarge {
            path: relative.to_owned(),
            len: metadata.len(),
        });
    }
    let mut reader = io::BufReader::new(fs::File::open(path).map_err(|source| ToolError::Io {
        context: "cannot open file",
        path: path.to_path_buf(),
        source,
    })?);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| ToolError::Io {
            context: "cannot read file",
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn read_bounded(path: &Path, relative: &str) -> Result<Vec<u8>, ToolError> {
    let metadata = fs::metadata(path).map_err(|source| ToolError::Io {
        context: "cannot stat file",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_ENTRY_BYTES {
        return Err(ToolError::EntryTooLarge {
            path: relative.to_owned(),
            len: metadata.len(),
        });
    }
    fs::read(path).map_err(|source| ToolError::Io {
        context: "cannot read file",
        path: path.to_path_buf(),
        source,
    })
}

fn build_entries(package_dir: &Path, out_path: &Path) -> Result<Vec<ManifestEntry>, ToolError> {
    let mut files = Vec::new();
    collect_files(package_dir, package_dir, &mut files)?;
    files.retain(|relative| !is_excluded(package_dir, relative, out_path));
    if files.len() > MAX_MANIFEST_ENTRIES {
        return Err(ToolError::TooManyEntries(files.len()));
    }
    let mut entries = Vec::with_capacity(files.len());
    for relative in files {
        let path_text = relative
            .to_str()
            .ok_or_else(|| ToolError::InvalidPath(relative.clone()))?
            .replace('\\', "/");
        validate_manifest_path(&path_text)?;
        if ENTRY_LINE_OVERHEAD + path_text.len() > MAX_MANIFEST_LINE_BYTES {
            return Err(ToolError::LineTooLong(path_text));
        }
        let sha256 = hash_file(&package_dir.join(&relative), &path_text)?;
        entries.push(ManifestEntry {
            path: path_text,
            sha256,
        });
    }
    Ok(entries)
}

fn render_sha256sums(entries: &[ManifestEntry]) -> String {
    let mut text = String::new();
    for entry in entries {
        text.push_str(&format!("{}  {}\n", hex_lower(&entry.sha256), entry.path));
    }
    text
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| ToolError::InvalidPath(path.to_path_buf()))?;
    let tmp = parent.join(format!(
        "{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    fs::write(&tmp, bytes).map_err(|source| ToolError::Io {
        context: "cannot write temporary file",
        path: tmp.clone(),
        source,
    })?;
    let renamed = fs::rename(&tmp, path)
        .or_else(|_| fs::remove_file(path).and_then(|_| fs::rename(&tmp, path)));
    renamed.map_err(|source| ToolError::Io {
        context: "cannot replace output file",
        path: path.to_path_buf(),
        source,
    })
}

fn cmd_sign(args: &[String]) -> Result<String, ToolError> {
    let package_dir = PathBuf::from(take_value(args, "--package-dir")?);
    let key_path = PathBuf::from(take_value(args, "--key")?);
    let counter_text = take_value(args, "--counter")?;
    let counter = counter_text
        .parse::<u64>()
        .map_err(|_| ToolError::Usage(format!("invalid counter value {counter_text}")))?;
    let out_path = take_optional_value(args, "--out")?
        .map(PathBuf::from)
        .unwrap_or_else(|| package_dir.join("manifest.sig"));

    let secret = load_secret_key(&key_path)?;
    let entries = build_entries(&package_dir, &out_path)?;
    let text = sign_manifest(&secret, counter, &entries).map_err(|source| ToolError::Crypto {
        context: "cannot sign manifest",
        source,
    })?;

    atomic_write(&out_path, text.as_bytes())?;
    let sums = render_sha256sums(&entries);
    atomic_write(&package_dir.join("SHA256SUMS.txt"), sums.as_bytes())?;

    Ok(format!(
        "signed {} entries in {} at release counter {counter}\n  manifest: {}",
        entries.len(),
        package_dir.display(),
        out_path.display()
    ))
}

fn cmd_verify(args: &[String]) -> Result<String, ToolError> {
    let package_dir = PathBuf::from(take_value(args, "--package-dir")?);
    let anchor_path = PathBuf::from(take_value(args, "--anchor")?);
    let anchor = load_anchor(&anchor_path)?;

    let manifest_path = package_dir.join("manifest.sig");
    let text = fs::read_to_string(&manifest_path).map_err(|source| ToolError::Io {
        context: "cannot read manifest",
        path: manifest_path.clone(),
        source,
    })?;
    let manifest = parse_signed_manifest(&text).map_err(|source| ToolError::Crypto {
        context: "manifest is malformed",
        source,
    })?;

    let mut failures = Vec::new();
    if let Err(cause) = verify_manifest(&manifest, &anchor) {
        failures.push(format!(
            "{}: signature verification failed: {cause}",
            manifest_path.display()
        ));
    }
    for entry in &manifest.entries {
        let file_path = package_dir.join(&entry.path);
        match read_bounded(&file_path, &entry.path) {
            Ok(bytes) => {
                if let Err(cause) = verify_entry_digest(&manifest, &entry.path, &bytes) {
                    failures.push(format!("{}: {cause}", entry.path));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", entry.path)),
        }
    }
    if failures.is_empty() {
        Ok(format!(
            "verified {} entries in {}",
            manifest.entries.len(),
            package_dir.display()
        ))
    } else {
        Err(ToolError::VerifyFailed(failures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(text: &str) -> String {
        text.to_owned()
    }

    fn path_text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("manifest-tool-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_package_file(root: &Path, rel: &str, bytes: &[u8]) {
        let mut path = root.to_path_buf();
        for part in rel.split('/') {
            path.push(part);
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn gen_key(dir: &Path) -> Result<String, ToolError> {
        run(&[owned("gen-key"), owned("--out-dir"), path_text(dir)])
    }

    fn sign_package(package: &Path, keys: &Path, counter: &str) -> Result<String, ToolError> {
        run(&[
            owned("sign"),
            owned("--package-dir"),
            path_text(package),
            owned("--key"),
            path_text(&keys.join("signing-key.sec")),
            owned("--counter"),
            owned(counter),
        ])
    }

    fn verify_package(package: &Path, anchor: &Path) -> Result<String, ToolError> {
        run(&[
            owned("verify"),
            owned("--package-dir"),
            path_text(package),
            owned("--anchor"),
            path_text(anchor),
        ])
    }

    fn make_package(dir: &Path) -> PathBuf {
        let package = dir.join("package");
        write_package_file(&package, "Slots/A/true-tick.exe", b"payload-a");
        write_package_file(&package, "Slots/B/true-tick.exe", b"payload-b");
        write_package_file(&package, "active-slot.txt", b"A\n");
        package
    }

    #[test]
    fn gen_key_writes_anchor_and_secret_files() {
        let dir = temp_dir("gen-key-basic");
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        let anchor = fs::read_to_string(keys.join("trust-anchor.pub")).unwrap();
        let secret = fs::read_to_string(keys.join("signing-key.sec")).unwrap();
        assert_eq!(anchor.len(), 65);
        assert_eq!(secret.len(), 65);
        assert!(anchor
            .trim_end()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(secret
            .trim_end()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gen_key_refuses_to_overwrite_without_force() {
        let dir = temp_dir("gen-key-overwrite");
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        let err = gen_key(&keys).unwrap_err();
        assert!(err.to_string().contains("--force"));
        run(&[
            owned("gen-key"),
            owned("--out-dir"),
            path_text(&keys),
            owned("--force"),
        ])
        .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sign_produces_manifest_that_verifies_against_the_anchor() {
        let dir = temp_dir("sign-verify");
        let package = make_package(&dir);
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        sign_package(&package, &keys, "7").unwrap();

        let text = fs::read_to_string(package.join("manifest.sig")).unwrap();
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(manifest.release_counter, 7);
        assert_eq!(manifest.entries.len(), 3);
        let anchor = load_anchor(&keys.join("trust-anchor.pub")).unwrap();
        assert_eq!(verify_manifest(&manifest, &anchor), Ok(()));
        verify_package(&package, &keys.join("trust-anchor.pub")).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sign_is_stable_across_repeated_runs_when_inputs_are_unchanged() {
        let dir = temp_dir("sign-stable");
        let package = make_package(&dir);
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        sign_package(&package, &keys, "3").unwrap();
        let first = fs::read_to_string(package.join("manifest.sig")).unwrap();
        let first_sums = fs::read_to_string(package.join("SHA256SUMS.txt")).unwrap();
        sign_package(&package, &keys, "3").unwrap();
        assert_eq!(
            first,
            fs::read_to_string(package.join("manifest.sig")).unwrap()
        );
        assert_eq!(
            first_sums,
            fs::read_to_string(package.join("SHA256SUMS.txt")).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sign_excludes_its_own_outputs() {
        let dir = temp_dir("sign-excludes");
        let package = make_package(&dir);
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        fs::write(package.join("custom.sig"), b"stale signature").unwrap();
        run(&[
            owned("sign"),
            owned("--package-dir"),
            path_text(&package),
            owned("--key"),
            path_text(&keys.join("signing-key.sec")),
            owned("--counter"),
            owned("1"),
            owned("--out"),
            path_text(&package.join("custom.sig")),
        ])
        .unwrap();
        let manifest =
            parse_signed_manifest(&fs::read_to_string(package.join("custom.sig")).unwrap())
                .unwrap();
        for entry in &manifest.entries {
            assert_ne!(entry.path, "manifest.sig");
            assert_ne!(entry.path, "SHA256SUMS.txt");
            assert_ne!(entry.path, "custom.sig");
        }
        assert_eq!(manifest.entries.len(), 3);
        let sums = fs::read_to_string(package.join("SHA256SUMS.txt")).unwrap();
        assert!(!sums.contains("manifest.sig"));
        assert!(!sums.contains("SHA256SUMS.txt"));
        assert!(!sums.contains("custom.sig"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verify_reports_a_tampered_file() {
        let dir = temp_dir("verify-tampered");
        let package = make_package(&dir);
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        sign_package(&package, &keys, "1").unwrap();
        write_package_file(&package, "Slots/A/true-tick.exe", b"tampered");
        let err = verify_package(&package, &keys.join("trust-anchor.pub")).unwrap_err();
        assert!(err.to_string().contains("Slots/A/true-tick.exe"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verify_reports_a_wrong_anchor() {
        let dir = temp_dir("verify-wrong-anchor");
        let package = make_package(&dir);
        let keys = dir.join("keys");
        let other_keys = dir.join("other-keys");
        gen_key(&keys).unwrap();
        gen_key(&other_keys).unwrap();
        sign_package(&package, &keys, "1").unwrap();
        let err = verify_package(&package, &other_keys.join("trust-anchor.pub")).unwrap_err();
        assert!(err.to_string().contains("signature"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sign_refuses_oversized_entry() {
        let dir = temp_dir("sign-oversized");
        let package = dir.join("package");
        write_package_file(&package, "big.bin", b"seed");
        let big = package.join("big.bin");
        fs::File::create(&big)
            .unwrap()
            .set_len(MAX_ENTRY_BYTES + 1)
            .unwrap();
        let keys = dir.join("keys");
        gen_key(&keys).unwrap();
        let err = sign_package(&package, &keys, "1").unwrap_err();
        assert!(err.to_string().contains("big.bin"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_rejects_missing_required_arguments() {
        assert!(run(&[]).is_err());
        assert!(run(&[owned("bogus")]).is_err());
        assert!(run(&[owned("gen-key")]).is_err());
        assert!(run(&[owned("sign")]).is_err());
        assert!(run(&[owned("sign"), owned("--package-dir"), owned("x")]).is_err());
        assert!(run(&[owned("verify")]).is_err());
        assert!(run(&[owned("verify"), owned("--package-dir"), owned("x")]).is_err());
    }
}
