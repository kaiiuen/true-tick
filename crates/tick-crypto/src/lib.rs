//! Ed25519 signed manifest verification for True Tick release packages.
//!
//! This crate implements the verification half of the D-20 and D-35 release
//! integrity contract. It parses a deterministic canonical text manifest,
//! verifies the Ed25519 signature over the canonical byte sequence, checks
//! per-entry SHA-256 digests, and enforces a strictly increasing release
//! counter for downgrade refusal.
//!
//! The signing tool itself lives in `apps/manifest-tool`, which reuses the
//! producer helpers exported from this crate.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fmt;

/// Length in bytes of an Ed25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// Length in bytes of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// Maximum number of `entry` lines accepted in a manifest.
pub const MAX_MANIFEST_ENTRIES: usize = 256;

/// Maximum byte length of a single manifest line.
pub const MAX_MANIFEST_LINE_BYTES: usize = 4096;

/// Length in bytes of an Ed25519 secret signing seed.
pub const SECRET_KEY_LEN: usize = 32;

/// Generates a fresh Ed25519 keypair from operating system entropy.
///
/// Returns the 32-byte secret signing seed first and the 32-byte public
/// verifying key second. The secret seed is the value persisted as the
/// signing key by the manifest tool.
pub fn generate_keypair() -> ([u8; SECRET_KEY_LEN], [u8; PUBLIC_KEY_LEN]) {
    let signing = SigningKey::generate(&mut rand_core::OsRng);
    let secret = signing.to_bytes();
    (secret, signing.verifying_key().to_bytes())
}

/// Rebuilds an Ed25519 signing key from its raw 32-byte secret seed.
pub fn signing_key_from_bytes(secret: &[u8]) -> Result<SigningKey, CryptoError> {
    let bytes: &[u8; SECRET_KEY_LEN] =
        secret.try_into().map_err(|_| CryptoError::WrongKeyLength)?;
    Ok(SigningKey::from_bytes(bytes))
}

/// Signs `message` with `secret` and returns the raw 64-byte signature.
pub fn sign_bytes(secret: &[u8], message: &[u8]) -> Result<[u8; SIGNATURE_LEN], CryptoError> {
    let signing = signing_key_from_bytes(secret)?;
    Ok(signing.sign(message).to_bytes())
}

/// Renders the canonical manifest text for `entries` signed with `secret`.
///
/// The output is exactly the `release_counter` line, one `entry` line per
/// path-sorted entry, and the `signature` line, each newline terminated.
/// `entries` are copied, sorted, and deduplicated before signing so the
/// rendered manifest always parses cleanly.
pub fn sign_manifest(
    secret: &[u8],
    release_counter: u64,
    entries: &[ManifestEntry],
) -> Result<String, CryptoError> {
    let mut manifest = SignedManifest {
        release_counter,
        entries: entries.to_vec(),
        signature: [0u8; SIGNATURE_LEN],
    };
    manifest
        .entries
        .sort_by(|left, right| left.path.cmp(&right.path));
    manifest
        .entries
        .dedup_by(|left, right| left.path == right.path);
    let canonical = canonical_bytes(&manifest);
    manifest.signature = sign_bytes(secret, &canonical)?;

    let mut hex = String::with_capacity(SIGNATURE_LEN * 2);
    for byte in manifest.signature {
        hex.push_str(&format!("{byte:02x}"));
    }
    let mut text = String::from_utf8(canonical).map_err(|_| CryptoError::MalformedManifest)?;
    text.push_str(&format!("signature {hex}\n"));
    Ok(text)
}

/// A trusted Ed25519 public key used to verify release manifests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustAnchor {
    pub key: [u8; PUBLIC_KEY_LEN],
}

impl TrustAnchor {
    /// Builds a trust anchor from raw key bytes, rejecting wrong lengths.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != PUBLIC_KEY_LEN {
            return Err(CryptoError::WrongKeyLength);
        }
        let mut key = [0u8; PUBLIC_KEY_LEN];
        key.copy_from_slice(bytes);
        Ok(Self { key })
    }
}

/// Errors produced by manifest parsing and verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CryptoError {
    WrongKeyLength,
    WrongSignatureLength,
    MalformedManifest,
    SignatureInvalid,
    DigestMismatch,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKeyLength => write!(formatter, "trust anchor key has wrong length"),
            Self::WrongSignatureLength => {
                write!(formatter, "manifest signature has wrong length")
            }
            Self::MalformedManifest => write!(formatter, "manifest is malformed"),
            Self::SignatureInvalid => write!(formatter, "manifest signature is invalid"),
            Self::DigestMismatch => write!(formatter, "entry digest does not match"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// One manifest entry binding a package-relative path to a SHA-256 digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: [u8; 32],
}

/// A parsed signed manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedManifest {
    pub release_counter: u64,
    pub entries: Vec<ManifestEntry>,
    pub signature: [u8; SIGNATURE_LEN],
}

fn decode_hex<const N: usize>(text: &str) -> Result<[u8; N], CryptoError> {
    if text.len() != N * 2 {
        return Err(CryptoError::MalformedManifest);
    }
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CryptoError::MalformedManifest);
    }
    let mut out = [0u8; N];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
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

/// Parses the canonical text manifest format.
///
/// The accepted grammar is exactly:
/// - `release_counter <decimal u64>` exactly once
/// - `entry <path> <64 lowercase hex sha256>` zero to `MAX_MANIFEST_ENTRIES` times
/// - `signature <128 lowercase hex>` exactly once
///
/// Blank lines are rejected. Any other line, duplicate keys, duplicate entry
/// paths, non lowercase hex, wrong hex lengths, and lines longer than
/// `MAX_MANIFEST_LINE_BYTES` are rejected as `MalformedManifest`.
pub fn parse_signed_manifest(text: &str) -> Result<SignedManifest, CryptoError> {
    let mut release_counter: Option<u64> = None;
    let mut signature: Option<[u8; SIGNATURE_LEN]> = None;
    let mut entries: Vec<ManifestEntry> = Vec::new();

    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_MANIFEST_LINE_BYTES {
            return Err(CryptoError::MalformedManifest);
        }
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("release_counter ") {
            if release_counter.is_some() {
                return Err(CryptoError::MalformedManifest);
            }
            let value = rest
                .parse::<u64>()
                .map_err(|_| CryptoError::MalformedManifest)?;
            release_counter = Some(value);
        } else if let Some(rest) = line.strip_prefix("entry ") {
            if entries.len() >= MAX_MANIFEST_ENTRIES {
                return Err(CryptoError::MalformedManifest);
            }
            let (path, hex_digest) = rest
                .rsplit_once(' ')
                .ok_or(CryptoError::MalformedManifest)?;
            if path.is_empty() || path.contains(char::is_whitespace) {
                return Err(CryptoError::MalformedManifest);
            }
            let sha256 = decode_hex::<32>(hex_digest)?;
            if entries.iter().any(|entry| entry.path == path) {
                return Err(CryptoError::MalformedManifest);
            }
            entries.push(ManifestEntry {
                path: path.to_owned(),
                sha256,
            });
        } else if let Some(rest) = line.strip_prefix("signature ") {
            if signature.is_some() {
                return Err(CryptoError::MalformedManifest);
            }
            if rest.len() != SIGNATURE_LEN * 2 {
                return Err(CryptoError::WrongSignatureLength);
            }
            signature = Some(decode_hex::<SIGNATURE_LEN>(rest)?);
        } else {
            return Err(CryptoError::MalformedManifest);
        }
    }

    Ok(SignedManifest {
        release_counter: release_counter.ok_or(CryptoError::MalformedManifest)?,
        entries,
        signature: signature.ok_or(CryptoError::MalformedManifest)?,
    })
}

/// Produces the exact byte sequence covered by the manifest signature.
///
/// The sequence is the `release_counter <n>` line followed by every
/// `entry <path> <hex sha256>` line sorted lexicographically by path, each
/// terminated by a single `\n`. The `signature` line is excluded.
pub fn canonical_bytes(manifest: &SignedManifest) -> Vec<u8> {
    let mut sorted: Vec<&ManifestEntry> = manifest.entries.iter().collect();
    sorted.sort_by(|left, right| left.path.cmp(&right.path));

    let mut bytes = Vec::new();
    bytes.extend_from_slice(format!("release_counter {}\n", manifest.release_counter).as_bytes());
    for entry in sorted {
        let mut hex = String::with_capacity(64);
        for byte in entry.sha256 {
            hex.push_str(&format!("{byte:02x}"));
        }
        bytes.extend_from_slice(format!("entry {} {hex}\n", entry.path).as_bytes());
    }
    bytes
}

/// Verifies the manifest Ed25519 signature over `canonical_bytes`.
pub fn verify_manifest(manifest: &SignedManifest, anchor: &TrustAnchor) -> Result<(), CryptoError> {
    let key = VerifyingKey::from_bytes(&anchor.key).map_err(|_| CryptoError::SignatureInvalid)?;
    let signature = Signature::from_bytes(&manifest.signature);
    key.verify(&canonical_bytes(manifest), &signature)
        .map_err(|_| CryptoError::SignatureInvalid)
}

/// Computes the SHA-256 of `bytes` and compares it to the manifest entry for
/// `path`. Returns `DigestMismatch` when the path is absent or the digest
/// differs.
pub fn verify_entry_digest(
    manifest: &SignedManifest,
    path: &str,
    bytes: &[u8],
) -> Result<(), CryptoError> {
    let entry = manifest
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .ok_or(CryptoError::DigestMismatch)?;
    let actual = Sha256::digest(bytes);
    if actual.as_slice() == entry.sha256 {
        Ok(())
    } else {
        Err(CryptoError::DigestMismatch)
    }
}

/// Enforces strictly increasing release counters so downgrade is refused.
///
/// A manifest whose counter is not strictly greater than `previously_seen` is
/// rejected with `SignatureInvalid` because the manifest fails to authorize a
/// forward release transition.
pub fn check_release_counter(
    manifest: &SignedManifest,
    previously_seen: u64,
) -> Result<(), CryptoError> {
    if manifest.release_counter > previously_seen {
        Ok(())
    } else {
        Err(CryptoError::SignatureInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn test_keypair() -> (SigningKey, TrustAnchor) {
        let secret = [7u8; 32];
        let signing = SigningKey::from_bytes(&secret);
        let anchor = TrustAnchor {
            key: signing.verifying_key().to_bytes(),
        };
        (signing, anchor)
    }

    fn sign_manifest_text(signing: &SigningKey, unsigned: &str) -> String {
        let signature = signing.sign(unsigned.as_bytes());
        let mut hex = String::with_capacity(128);
        for byte in signature.to_bytes() {
            hex.push_str(&format!("{byte:02x}"));
        }
        format!("{unsigned}signature {hex}\n")
    }

    fn unsigned_manifest(counter: u64, entries: &[(&str, &str)]) -> String {
        let mut text = format!("release_counter {counter}\n");
        for (path, hash) in entries {
            text.push_str(&format!("entry {path} {hash}\n"));
        }
        text
    }

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn generate_keypair_produces_matching_secret_and_anchor() {
        let (secret, public) = generate_keypair();
        assert_eq!(secret.len(), SECRET_KEY_LEN);
        assert_eq!(public.len(), PUBLIC_KEY_LEN);
        let signing = signing_key_from_bytes(&secret).unwrap();
        assert_eq!(signing.verifying_key().to_bytes(), public);
    }

    #[test]
    fn signing_key_from_bytes_rejects_wrong_length() {
        assert_eq!(
            signing_key_from_bytes(&[0u8; 31]).unwrap_err(),
            CryptoError::WrongKeyLength
        );
        assert_eq!(
            signing_key_from_bytes(&[0u8; 33]).unwrap_err(),
            CryptoError::WrongKeyLength
        );
    }

    #[test]
    fn sign_manifest_round_trips_through_parser_and_verifier() {
        let (secret, public) = generate_keypair();
        let anchor = TrustAnchor { key: public };
        let entries = vec![
            ManifestEntry {
                path: "zeta.bin".to_owned(),
                sha256: [0xaau8; 32],
            },
            ManifestEntry {
                path: "alpha.bin".to_owned(),
                sha256: [0xbbu8; 32],
            },
        ];
        let text = sign_manifest(&secret, 7, &entries).unwrap();
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(manifest.release_counter, 7);
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(verify_manifest(&manifest, &anchor), Ok(()));
    }

    #[test]
    fn sign_manifest_deduplicates_and_sorts_entries() {
        let (secret, public) = generate_keypair();
        let anchor = TrustAnchor { key: public };
        let entries = vec![
            ManifestEntry {
                path: "b.bin".to_owned(),
                sha256: [1u8; 32],
            },
            ManifestEntry {
                path: "a.bin".to_owned(),
                sha256: [2u8; 32],
            },
            ManifestEntry {
                path: "b.bin".to_owned(),
                sha256: [3u8; 32],
            },
        ];
        let text = sign_manifest(&secret, 1, &entries).unwrap();
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.entries[0].path, "a.bin");
        assert_eq!(verify_manifest(&manifest, &anchor), Ok(()));
    }

    #[test]
    fn rejects_wrong_key_length() {
        assert_eq!(
            TrustAnchor::from_bytes(&[0u8; 31]),
            Err(CryptoError::WrongKeyLength)
        );
        assert_eq!(
            TrustAnchor::from_bytes(&[0u8; 33]),
            Err(CryptoError::WrongKeyLength)
        );
        assert!(TrustAnchor::from_bytes(&[0u8; 32]).is_ok());
    }

    #[test]
    fn rejects_wrong_signature_length() {
        let short = "release_counter 1\nsignature abcd\n";
        assert_eq!(
            parse_signed_manifest(short),
            Err(CryptoError::WrongSignatureLength)
        );
        let long = format!("release_counter 1\nsignature {}\n", "ab".repeat(65));
        assert_eq!(
            parse_signed_manifest(&long),
            Err(CryptoError::WrongSignatureLength)
        );
    }

    #[test]
    fn parses_valid_manifest() {
        let text = format!(
            "release_counter 42\nentry Slots/A/true-tick.exe {HASH_A}\nentry Recovery/true-tick.exe {HASH_B}\nsignature {}\n",
            "cc".repeat(64)
        );
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(manifest.release_counter, 42);
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.entries[0].path, "Slots/A/true-tick.exe");
        assert_eq!(manifest.entries[0].sha256, [0xaau8; 32]);
        assert_eq!(manifest.signature, [0xccu8; 64]);
    }

    #[test]
    fn rejects_duplicate_keys_and_unknown_lines() {
        let dup_counter = format!(
            "release_counter 1\nrelease_counter 2\nsignature {}\n",
            "aa".repeat(64)
        );
        assert_eq!(
            parse_signed_manifest(&dup_counter),
            Err(CryptoError::MalformedManifest)
        );

        let dup_signature = format!(
            "release_counter 1\nsignature {}\nsignature {}\n",
            "aa".repeat(64),
            "bb".repeat(64)
        );
        assert_eq!(
            parse_signed_manifest(&dup_signature),
            Err(CryptoError::MalformedManifest)
        );

        let dup_entry = format!(
            "release_counter 1\nentry p {HASH_A}\nentry p {HASH_B}\nsignature {}\n",
            "cc".repeat(64)
        );
        assert_eq!(
            parse_signed_manifest(&dup_entry),
            Err(CryptoError::MalformedManifest)
        );

        let unknown = format!(
            "release_counter 1\nbogus line\nsignature {}\n",
            "aa".repeat(64)
        );
        assert_eq!(
            parse_signed_manifest(&unknown),
            Err(CryptoError::MalformedManifest)
        );
    }

    #[test]
    fn rejects_non_lowercase_hex() {
        let upper_entry = format!(
            "release_counter 1\nentry p {}\nsignature {}\n",
            HASH_A.to_uppercase(),
            "aa".repeat(64)
        );
        assert_eq!(
            parse_signed_manifest(&upper_entry),
            Err(CryptoError::MalformedManifest)
        );

        let upper_sig = format!("release_counter 1\nsignature {}\n", "AB".repeat(64));
        assert_eq!(
            parse_signed_manifest(&upper_sig),
            Err(CryptoError::MalformedManifest)
        );
    }

    #[test]
    fn canonical_bytes_sort_entries_by_path() {
        let text = format!(
            "release_counter 9\nentry zeta {HASH_A}\nentry alpha {HASH_B}\nsignature {}\n",
            "dd".repeat(64)
        );
        let manifest = parse_signed_manifest(&text).unwrap();
        let canonical = canonical_bytes(&manifest);
        let expected = format!("release_counter 9\nentry alpha {HASH_B}\nentry zeta {HASH_A}\n");
        assert_eq!(canonical, expected.as_bytes());
        assert!(!canonical.windows(10).any(|w| w == b"signature "));
    }

    #[test]
    fn verify_accepts_valid_signature() {
        let (signing, anchor) = test_keypair();
        let unsigned = unsigned_manifest(3, &[("Slots/A/true-tick.exe", HASH_A)]);
        let text = sign_manifest_text(&signing, &unsigned);
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(verify_manifest(&manifest, &anchor), Ok(()));
    }

    #[test]
    fn verify_rejects_tampered_entry() {
        let (signing, anchor) = test_keypair();
        let unsigned = unsigned_manifest(3, &[("Slots/A/true-tick.exe", HASH_A)]);
        let text = sign_manifest_text(&signing, &unsigned);
        let tampered = text.replace(HASH_A, HASH_B);
        let manifest = parse_signed_manifest(&tampered).unwrap();
        assert_eq!(
            verify_manifest(&manifest, &anchor),
            Err(CryptoError::SignatureInvalid)
        );
    }

    #[test]
    fn verify_entry_digest_matches_and_mismatches() {
        let payload = b"fixture executable bytes";
        let digest = Sha256::digest(payload);
        let mut hex = String::with_capacity(64);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        let text = format!(
            "release_counter 1\nentry Slots/A/true-tick.exe {hex}\nsignature {}\n",
            "aa".repeat(64)
        );
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(
            verify_entry_digest(&manifest, "Slots/A/true-tick.exe", payload),
            Ok(())
        );
        assert_eq!(
            verify_entry_digest(&manifest, "Slots/A/true-tick.exe", b"other"),
            Err(CryptoError::DigestMismatch)
        );
        assert_eq!(
            verify_entry_digest(&manifest, "Slots/B/true-tick.exe", payload),
            Err(CryptoError::DigestMismatch)
        );
    }

    #[test]
    fn release_counter_refuses_equal_or_lower() {
        let text = format!("release_counter 10\nsignature {}\n", "aa".repeat(64));
        let manifest = parse_signed_manifest(&text).unwrap();
        assert_eq!(check_release_counter(&manifest, 9), Ok(()));
        assert!(check_release_counter(&manifest, 10).is_err());
        assert!(check_release_counter(&manifest, 11).is_err());
    }
}
