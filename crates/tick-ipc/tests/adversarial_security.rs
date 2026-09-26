//! Adversarial protocol and security fuzzing suite for `tick-ipc`.
//!
//! Exercises malformed magic and version injection, token tampering and
//! bit flipping, CRC-32 corruption detection, payload length hard limits,
//! and token file parser hardening.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tick_ipc::{
    crc32, read_token_file, CommandVerb, IpcError, IpcFrame, SessionToken, MAGIC_BYTES,
    MAX_PAYLOAD_LEN, PROTOCOL_VERSION, TOKEN_LEN,
};

/// Returns a unique temporary path inside the crate test temp space.
fn temp_token_path(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    path.push(format!(
        "tick_ipc_adv_{}_{}_{}.tok",
        std::process::id(),
        nanos,
        tag
    ));
    path
}

/// Builds a raw frame buffer by hand with caller controlled magic, version,
/// declared payload length, token, verb, and payload bytes.
fn craft_raw_frame(
    magic: &[u8; 4],
    version: u16,
    declared_len: u16,
    token: &[u8; TOKEN_LEN],
    verb: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(48 + payload.len());
    buf.extend_from_slice(magic);
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&declared_len.to_le_bytes());
    buf.extend_from_slice(token);
    buf.extend_from_slice(&verb.to_le_bytes());
    buf.extend_from_slice(payload);
    let checksum = crc32(&buf);
    buf.extend_from_slice(&checksum.to_le_bytes());
    buf
}

// ---------------------------------------------------------------------------
// 1. Malformed magic and version injection
// ---------------------------------------------------------------------------

#[test]
fn garbage_magic_prefixes_are_rejected() {
    let token = SessionToken::generate_ephemeral();
    let token_bytes = *token.as_bytes();

    let candidates: &[[u8; 4]] = &[
        [0x00, 0x00, 0x00, 0x00],
        [0xFF, 0xFF, 0xFF, 0xFF],
        [0x54, 0x54, 0x49, 0x51], // TTIP with last byte off by one
        [0x00, 0x54, 0x49, 0x50], // shifted magic
        [0xDE, 0xAD, 0xBE, 0xEF],
        *b"TTIP", // ascii still correct, control case
    ];

    for (index, magic) in candidates.iter().enumerate() {
        let raw = craft_raw_frame(magic, PROTOCOL_VERSION, 0, &token_bytes, 1, &[]);
        let result = IpcFrame::deserialize(&raw);
        if *magic == MAGIC_BYTES {
            assert!(result.is_ok(), "control frame {index} must parse");
        } else {
            assert_eq!(
                result,
                Err(IpcError::InvalidMagic),
                "garbage magic candidate {index} must be rejected"
            );
        }
    }
}

#[test]
fn every_magic_byte_position_flip_is_rejected() {
    let token = SessionToken::generate_ephemeral();
    let token_bytes = *token.as_bytes();

    for position in 0..4 {
        for bit in 0..8 {
            let mut magic = MAGIC_BYTES;
            magic[position] ^= 1 << bit;
            let raw = craft_raw_frame(&magic, PROTOCOL_VERSION, 0, &token_bytes, 1, &[]);
            assert_eq!(
                IpcFrame::deserialize(&raw),
                Err(IpcError::InvalidMagic),
                "magic bit flip at byte {position} bit {bit} must be rejected"
            );
        }
    }
}

#[test]
fn partial_headers_are_rejected_as_truncated() {
    let frame = IpcFrame::new(
        SessionToken::generate_ephemeral(),
        CommandVerb::QueryStatus,
        vec![0x01, 0x02],
    )
    .expect("frame creation");
    let full = frame.serialize();

    for truncated_len in 0..48 {
        let slice = &full[..truncated_len];
        let result = IpcFrame::deserialize(slice);
        assert_eq!(
            result,
            Err(IpcError::PayloadTruncated),
            "truncated frame of {truncated_len} bytes must be rejected"
        );
    }
}

#[test]
fn future_and_past_versions_are_rejected() {
    let token = SessionToken::generate_ephemeral();
    let token_bytes = *token.as_bytes();

    let versions: &[u16] = &[0x0000, 0x0002, 0x00FF, 0x0100, 0x7FFF, 0x8000, 0xFFFF];

    for &version in versions {
        let raw = craft_raw_frame(&MAGIC_BYTES, version, 0, &token_bytes, 1, &[]);
        assert_eq!(
            IpcFrame::deserialize(&raw),
            Err(IpcError::UnsupportedVersion { found: version }),
            "version {version:#06X} must be rejected"
        );
    }
}

#[test]
fn wrong_version_is_checked_before_crc() {
    let token = SessionToken::generate_ephemeral();
    let token_bytes = *token.as_bytes();
    let mut raw = craft_raw_frame(&MAGIC_BYTES, 0x9999, 0, &token_bytes, 1, &[]);
    // Corrupt the trailer so a naive implementation would report CRC first.
    let len = raw.len();
    raw[len - 1] ^= 0xFF;
    assert_eq!(
        IpcFrame::deserialize(&raw),
        Err(IpcError::UnsupportedVersion { found: 0x9999 })
    );
}

// ---------------------------------------------------------------------------
// 2. Token tampering and bit flipping
// ---------------------------------------------------------------------------

#[test]
fn legitimate_token_passes_integrity_and_matching() {
    let token = SessionToken::generate_ephemeral();
    let frame =
        IpcFrame::new(token, CommandVerb::RequestAcquire, vec![0x42; 8]).expect("frame creation");
    let bytes = frame.serialize();
    let parsed = IpcFrame::deserialize(&bytes).expect("legitimate frame must parse");
    assert!(parsed.token.matches(&token));
}

#[test]
fn every_token_bit_flip_is_detected_by_crc() {
    let token = SessionToken::generate_ephemeral();
    let frame = IpcFrame::new(token, CommandVerb::QueryStatus, vec![]).expect("frame creation");
    let base = frame.serialize();

    // Flip each bit inside the 32 byte token field at offset 8..40.
    for byte_index in 0..TOKEN_LEN {
        for bit in 0..8 {
            let mut mutated = base.clone();
            mutated[8 + byte_index] ^= 1 << bit;
            match IpcFrame::deserialize(&mutated) {
                Err(IpcError::CrcMismatch { .. }) => {}
                other => panic!(
                    "token bit flip at byte {byte_index} bit {bit} must yield CrcMismatch, got {other:?}"
                ),
            }
        }
    }
}

#[test]
fn forged_token_with_recomputed_crc_still_fails_authorization() {
    let real_token = SessionToken::generate_ephemeral();
    let mut forged_bytes = *real_token.as_bytes();
    forged_bytes[0] ^= 0x01;
    let forged = SessionToken::from_bytes(forged_bytes);

    // A forged token inside a well formed frame parses structurally but the
    // constant time matcher must reject authorization.
    assert!(!real_token.matches(&forged));

    let frame = IpcFrame::new(forged, CommandVerb::QueryStatus, vec![]).expect("frame creation");
    let raw = frame.serialize();
    let parsed = IpcFrame::deserialize(&raw).expect("forged frame still parses");
    assert!(!parsed.token.matches(&real_token));
}

#[test]
fn all_zero_and_all_ff_tokens_do_not_match_real_token() {
    let real_token = SessionToken::generate_ephemeral();
    let zero = SessionToken::from_bytes([0x00; TOKEN_LEN]);
    let ff = SessionToken::from_bytes([0xFF; TOKEN_LEN]);
    assert!(!real_token.matches(&zero));
    assert!(!real_token.matches(&ff));
    assert!(!zero.matches(&ff));
}

#[test]
fn token_hex_parsing_rejects_truncated_and_invalid_forms() {
    let token = SessionToken::generate_ephemeral();
    let hex = token.to_hex();
    assert_eq!(hex.len(), TOKEN_LEN * 2);

    // Truncated hex strings of every length below 64 are malformed.
    for cut in [0usize, 1, 2, 31, 62, 63] {
        assert_eq!(
            SessionToken::from_hex(&hex[..cut]),
            Err(IpcError::MalformedToken),
            "hex truncated to {cut} chars must be malformed"
        );
    }

    // Oversized hex strings are malformed.
    for extra in [1usize, 2, 64] {
        let mut oversized = hex.clone();
        oversized.push_str(&"a".repeat(extra));
        assert_eq!(
            SessionToken::from_hex(&oversized),
            Err(IpcError::MalformedToken),
            "hex extended by {extra} chars must be malformed"
        );
    }

    // Non hex characters anywhere in the string are malformed.
    for position in [0usize, 1, 31, 63] {
        let mut bad = hex.clone().into_bytes();
        bad[position] = b'z';
        let bad_str = String::from_utf8(bad).expect("ascii");
        assert_eq!(
            SessionToken::from_hex(&bad_str),
            Err(IpcError::MalformedToken),
            "non hex char at {position} must be malformed"
        );
    }

    // Uppercase hex is rejected by strict lowercase policy.
    let upper = hex.to_uppercase();
    assert_eq!(
        SessionToken::from_hex(&upper),
        Err(IpcError::MalformedToken)
    );
}

// ---------------------------------------------------------------------------
// 3. CRC-32 corruption
// ---------------------------------------------------------------------------

#[test]
fn every_payload_bit_flip_is_detected_by_crc() {
    let token = SessionToken::generate_ephemeral();
    let payload: Vec<u8> = (0..64).map(|i| i as u8 ^ 0xA5).collect();
    let frame = IpcFrame::new(token, CommandVerb::ScheduleAction, payload).expect("frame creation");
    let base = frame.serialize();

    for byte_index in 0..64 {
        for bit in 0..8 {
            let mut mutated = base.clone();
            mutated[44 + byte_index] ^= 1 << bit;
            match IpcFrame::deserialize(&mutated) {
                Err(IpcError::CrcMismatch { .. }) => {}
                other => panic!(
                    "payload bit flip at byte {byte_index} bit {bit} must yield CrcMismatch, got {other:?}"
                ),
            }
        }
    }
}

#[test]
fn crc_trailer_bit_flips_are_detected() {
    let token = SessionToken::generate_ephemeral();
    let frame =
        IpcFrame::new(token, CommandVerb::QueryStatus, vec![0x11; 4]).expect("frame creation");
    let base = frame.serialize();
    let body_len = base.len() - 4;

    for byte_index in 0..4 {
        for bit in 0..8 {
            let mut mutated = base.clone();
            mutated[body_len + byte_index] ^= 1 << bit;
            match IpcFrame::deserialize(&mutated) {
                Err(IpcError::CrcMismatch { .. }) => {}
                other => panic!(
                    "crc trailer bit flip at byte {byte_index} bit {bit} must yield CrcMismatch, got {other:?}"
                ),
            }
        }
    }
}

#[test]
fn crc32_known_vectors() {
    // IEEE CRC-32 check values.
    assert_eq!(crc32(b""), 0x0000_0000);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(
        crc32(b"The quick brown fox jumps over the lazy dog"),
        0x414F_A339
    );
}

// ---------------------------------------------------------------------------
// 4. Payload length limits
// ---------------------------------------------------------------------------

#[test]
fn declared_len_above_max_is_rejected_before_body_checks() {
    let token_bytes = *SessionToken::generate_ephemeral().as_bytes();

    for declared in [(MAX_PAYLOAD_LEN + 1) as u16, 300u16, 1024u16, u16::MAX] {
        let raw = craft_raw_frame(
            &MAGIC_BYTES,
            PROTOCOL_VERSION,
            declared,
            &token_bytes,
            1,
            &[],
        );
        assert_eq!(
            IpcFrame::deserialize(&raw),
            Err(IpcError::PayloadTooLarge {
                length: declared as usize
            }),
            "declared length {declared} must be rejected"
        );
    }
}

#[test]
fn constructor_rejects_oversize_payload() {
    let token = SessionToken::generate_ephemeral();
    let oversize = vec![0xAB; MAX_PAYLOAD_LEN + 1];
    assert_eq!(
        IpcFrame::new(token, CommandVerb::ScheduleAction, oversize),
        Err(IpcError::PayloadTooLarge {
            length: MAX_PAYLOAD_LEN + 1
        })
    );

    // Boundary case at exactly MAX is accepted.
    let at_max = vec![0xCD; MAX_PAYLOAD_LEN];
    assert!(IpcFrame::new(token, CommandVerb::ScheduleAction, at_max).is_ok());
}

#[test]
fn declared_len_oversize_does_not_read_past_buffer() {
    // A frame claiming 257 bytes of payload but carrying only 48 bytes must be
    // rejected at the length gate before any payload slicing occurs.
    let token_bytes = *SessionToken::generate_ephemeral().as_bytes();
    let raw = craft_raw_frame(
        &MAGIC_BYTES,
        PROTOCOL_VERSION,
        (MAX_PAYLOAD_LEN + 1) as u16,
        &token_bytes,
        1,
        &[],
    );
    assert_eq!(raw.len(), 48);
    assert_eq!(
        IpcFrame::deserialize(&raw),
        Err(IpcError::PayloadTooLarge {
            length: MAX_PAYLOAD_LEN + 1
        })
    );
}

// ---------------------------------------------------------------------------
// 5. Token file parser hardening
// ---------------------------------------------------------------------------

/// Writes raw bytes to a temp file, invokes the parser, and removes the file.
fn parse_token_bytes(tag: &str, bytes: &[u8]) -> std::io::Result<SessionToken> {
    let path = temp_token_path(tag);
    {
        let mut file = std::fs::File::create(&path).expect("create temp token file");
        file.write_all(bytes).expect("write temp token file");
    }
    let result = read_token_file(&path);
    let _ = std::fs::remove_file(&path);
    result
}

/// Asserts the io error wraps IpcError::MalformedToken.
fn assert_malformed_token(result: std::io::Result<SessionToken>, case: &str) {
    match result {
        Ok(_) => panic!("{case} unexpectedly parsed as a valid token"),
        Err(error) => {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::InvalidData,
                "{case} must surface InvalidData"
            );
            let inner = error.into_inner().expect("inner error present");
            let ipc_error = inner
                .downcast_ref::<IpcError>()
                .copied()
                .unwrap_or_else(|| panic!("{case} inner error must be IpcError"));
            assert_eq!(
                ipc_error,
                IpcError::MalformedToken,
                "{case} must wrap MalformedToken"
            );
        }
    }
}

#[test]
fn token_file_zero_byte_is_malformed() {
    assert_malformed_token(parse_token_bytes("empty", b""), "0 byte file");
}

#[test]
fn token_file_truncated_hex_is_malformed() {
    let token = SessionToken::generate_ephemeral();
    let hex = token.to_hex();
    assert_malformed_token(
        parse_token_bytes("short63", &hex.as_bytes()[..63]),
        "63 char hex",
    );
    assert_malformed_token(
        parse_token_bytes("short1", &hex.as_bytes()[..1]),
        "1 char hex",
    );
}

#[test]
fn token_file_oversized_hex_is_malformed() {
    let token = SessionToken::generate_ephemeral();
    let mut oversized = token.to_hex().into_bytes();
    oversized.push(b'0');
    assert_malformed_token(parse_token_bytes("long65", &oversized), "65 char hex");

    let mut double = token.to_hex().into_bytes();
    double.extend_from_slice(&token.to_hex().into_bytes());
    assert_malformed_token(parse_token_bytes("long128", &double), "128 char hex");
}

#[test]
fn token_file_non_hex_characters_are_malformed() {
    let garbage = b"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
    assert_malformed_token(parse_token_bytes("nonhex", garbage), "z filled file");

    let token = SessionToken::generate_ephemeral();
    let mut bad = token.to_hex().into_bytes();
    bad[10] = b'!';
    assert_malformed_token(parse_token_bytes("bang", &bad), "embedded punctuation");
}

#[test]
fn token_file_whitespace_handling() {
    let token = SessionToken::generate_ephemeral();
    let hex = token.to_hex();

    // Trailing LF and CRLF are tolerated by trim_end_matches.
    let mut lf = hex.clone().into_bytes();
    lf.push(b'\n');
    let parsed = parse_token_bytes("lf", &lf).expect("LF terminated file parses");
    assert!(parsed.matches(&token));

    let mut crlf = hex.clone().into_bytes();
    crlf.extend_from_slice(b"\r\n");
    let parsed = parse_token_bytes("crlf", &crlf).expect("CRLF terminated file parses");
    assert!(parsed.matches(&token));

    // Leading whitespace is not stripped and must fail.
    let mut leading = b"\n".to_vec();
    leading.extend_from_slice(hex.as_bytes());
    assert_malformed_token(parse_token_bytes("leadin", &leading), "leading newline");

    // Interior whitespace breaks the hex stream and must fail.
    let mut interior = hex.clone().into_bytes();
    interior.insert(32, b' ');
    assert_malformed_token(parse_token_bytes("space", &interior), "interior space");

    let mut interior_nl = hex.clone().into_bytes();
    interior_nl.insert(10, b'\n');
    assert_malformed_token(parse_token_bytes("midnl", &interior_nl), "interior newline");
}

#[test]
fn token_file_binary_gibberish_is_malformed_or_io() {
    // Non UTF-8 binary content fails at read_to_string with InvalidData and
    // never reaches the hex parser. That is still a clean io error.
    let mut gibberish = Vec::with_capacity(64);
    for i in 0..64u8 {
        gibberish.push(i.wrapping_mul(37).wrapping_add(0x80));
    }
    let result = parse_token_bytes("binary", &gibberish);
    match result {
        Ok(_) => panic!("binary gibberish must not parse"),
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::InvalidData),
    }

    // UTF-8 valid binary looking gibberish that is not hex still maps to
    // MalformedToken through the io error channel.
    let utf8_gibberish: Vec<u8> = std::iter::repeat_n(b'\x07', 64).collect();
    let result = parse_token_bytes("bells", &utf8_gibberish);
    match result {
        Ok(_) => panic!("control character file must not parse"),
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::InvalidData),
    }
}

#[test]
fn token_file_roundtrip_still_works() {
    let token = SessionToken::generate_ephemeral();
    let path = temp_token_path("roundtrip");
    tick_ipc::write_token_file(&path, &token).expect("write token file");
    let parsed = read_token_file(&path).expect("read token file");
    let _ = std::fs::remove_file(&path);
    assert!(parsed.matches(&token));
}
