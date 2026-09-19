//! Inter-Process Communication and wire protocol implementation for True Tick.
//!
//! Enforces zero-trust bounded frames, CRC-32 integrity validation,
//! strict timer interval boundaries, session token authentication,
//! and per-PID rate limiting.

use std::fmt;
use std::time::{Duration, Instant};
use tick_core::Hns;

/// Magic header bytes identifying True Tick IPC packets.
pub const MAGIC_BYTES: [u8; 4] = [0x54, 0x54, 0x49, 0x50];

/// Current protocol wire version.
pub const PROTOCOL_VERSION: u16 = 0x0001;

/// Maximum payload length allowed per frame in bytes.
pub const MAX_PAYLOAD_LEN: usize = 256;

/// Minimum allowed timer resolution in 100-nanosecond units (0.5 ms).
pub const MIN_INTERVAL_HNS: u64 = 5_000;

/// Maximum allowed timer resolution in 100-nanosecond units (15.625 ms).
pub const MAX_INTERVAL_HNS: u64 = 156_250;

/// Standard symbolic timer resolution preset in 100-nanosecond units (0.5 ms).
pub const SYMBOLIC_RESOLUTION_HNS: u64 = 5_000;

/// Fixed length of ephemeral session token in bytes.
pub const TOKEN_LEN: usize = 32;

/// Maximum allowed IPC requests per second for a single caller PID.
pub const MAX_REQUESTS_PER_SECOND: usize = 10;

/// Window duration for caller PID rate limiting.
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);

/// Standard CRC-32 implementation (IEEE 802.3 polynomial 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Ephemeral 256-bit session token representation.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct SessionToken(pub [u8; TOKEN_LEN]);

impl SessionToken {
    /// Creates a session token from an existing 32-byte array.
    pub const fn from_bytes(bytes: [u8; TOKEN_LEN]) -> Self {
        Self(bytes)
    }

    /// Accesses raw token bytes.
    pub const fn as_bytes(&self) -> &[u8; TOKEN_LEN] {
        &self.0
    }

    /// Validates an incoming token using constant-time comparison.
    pub fn matches(&self, candidate: &SessionToken) -> bool {
        let mut diff: u8 = 0;
        for i in 0..TOKEN_LEN {
            diff |= self.0[i] ^ candidate.0[i];
        }
        diff == 0
    }

    /// Generates an ephemeral session token using pseudo-random entropy.
    /// Uses system clock instant, duration counters, and a mixing state.
    pub fn generate_ephemeral() -> Self {
        let now = Instant::now();
        let elapsed = now.elapsed().as_nanos();
        let mut state: u64 = 0xA5A5_5A5A_1234_5678 ^ (elapsed as u64);
        let mut bytes = [0u8; TOKEN_LEN];
        for chunk in bytes.chunks_exact_mut(8) {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        Self(bytes)
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionToken([REDACTED])")
    }
}

/// Command verbs supported by the True Tick IPC wire protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CommandVerb {
    QueryStatus = 0x01,
    RequestAcquire = 0x02,
    RequestRelease = 0x03,
    ScheduleAction = 0x04,
    CancelSchedule = 0x05,
}

impl CommandVerb {
    /// Parses a raw u32 verb into a CommandVerb enum variant.
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            0x01 => Some(Self::QueryStatus),
            0x02 => Some(Self::RequestAcquire),
            0x03 => Some(Self::RequestRelease),
            0x04 => Some(Self::ScheduleAction),
            0x05 => Some(Self::CancelSchedule),
            _ => None,
        }
    }

    /// Converts the verb into its u32 discriminant.
    pub const fn to_u32(self) -> u32 {
        self as u32
    }
}

/// Validates whether a requested timer resolution interval in HNS is permitted.
/// External callers are strictly prohibited from requesting arbitrary intervals.
/// Only symbolic resolution or physical intervals between 5,000 HNS and 156,250 HNS are allowed.
pub const fn validate_interval_hns(interval_hns: u64) -> Result<Hns, IpcError> {
    if interval_hns == SYMBOLIC_RESOLUTION_HNS {
        return Ok(Hns::new(interval_hns));
    }
    if interval_hns >= MIN_INTERVAL_HNS && interval_hns <= MAX_INTERVAL_HNS {
        Ok(Hns::new(interval_hns))
    } else {
        Err(IpcError::IntervalOutOfBounds {
            requested: interval_hns,
        })
    }
}

/// Errors occurring during IPC frame processing or verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    InvalidMagic,
    UnsupportedVersion { found: u16 },
    PayloadTooLarge { length: usize },
    PayloadTruncated,
    InvalidCommandVerb { verb: u32 },
    IntervalOutOfBounds { requested: u64 },
    CrcMismatch { expected: u32, calculated: u32 },
    UnauthorizedToken,
    RateLimitExceeded { pid: u32 },
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "invalid IPC magic header bytes"),
            Self::UnsupportedVersion { found } => {
                write!(f, "unsupported protocol version: {found}")
            }
            Self::PayloadTooLarge { length } => write!(f, "payload exceeds limit: {length} bytes"),
            Self::PayloadTruncated => write!(f, "frame data truncated before completion"),
            Self::InvalidCommandVerb { verb } => {
                write!(f, "unknown or invalid command verb: {verb}")
            }
            Self::IntervalOutOfBounds { requested } => {
                write!(f, "requested interval {requested} HNS is out of bounds")
            }
            Self::CrcMismatch {
                expected,
                calculated,
            } => {
                write!(
                    f,
                    "CRC32 mismatch (expected {expected:08X}, calculated {calculated:08X})"
                )
            }
            Self::UnauthorizedToken => write!(f, "invalid or unauthorized session token"),
            Self::RateLimitExceeded { pid } => {
                write!(f, "rate limit exceeded for client PID {pid}")
            }
        }
    }
}

impl std::error::Error for IpcError {}

/// In-memory structured representation of an IPC frame.
///
/// Header structure on wire (44 bytes header + payload + 4 bytes CRC-32):
/// - 0x00..0x04: Magic header (4 bytes, "TTIP")
/// - 0x04..0x06: Protocol version (2 bytes, little-endian)
/// - 0x06..0x08: Payload length (2 bytes, little-endian, <= 256)
/// - 0x08..0x28: Session token (32 bytes)
/// - 0x28..0x2C: Command verb (4 bytes, little-endian)
/// - 0x2C..0x2C+len: Payload data (0 to 256 bytes)
/// - Trailer: CRC-32 over all preceding bytes (4 bytes, little-endian)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpcFrame {
    pub token: SessionToken,
    pub verb: CommandVerb,
    pub payload: Vec<u8>,
}

impl IpcFrame {
    /// Creates a new frame after verifying payload length bounds.
    pub fn new(token: SessionToken, verb: CommandVerb, payload: Vec<u8>) -> Result<Self, IpcError> {
        if payload.len() > MAX_PAYLOAD_LEN {
            return Err(IpcError::PayloadTooLarge {
                length: payload.len(),
            });
        }
        Ok(Self {
            token,
            verb,
            payload,
        })
    }

    /// Serializes the frame into wire format with trailing CRC-32.
    pub fn serialize(&self) -> Vec<u8> {
        let payload_len = self.payload.len() as u16;
        let total_size = 4 + 2 + 2 + TOKEN_LEN + 4 + self.payload.len() + 4;
        let mut buffer = Vec::with_capacity(total_size);

        buffer.extend_from_slice(&MAGIC_BYTES);
        buffer.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        buffer.extend_from_slice(&payload_len.to_le_bytes());
        buffer.extend_from_slice(self.token.as_bytes());
        buffer.extend_from_slice(&self.verb.to_u32().to_le_bytes());
        buffer.extend_from_slice(&self.payload);

        let checksum = crc32(&buffer);
        buffer.extend_from_slice(&checksum.to_le_bytes());
        buffer
    }

    /// Deserializes a raw binary slice into an IpcFrame with zero-trust validation.
    pub fn deserialize(data: &[u8]) -> Result<Self, IpcError> {
        // Minimum frame size: 4 (magic) + 2 (version) + 2 (len) + 32 (token) + 4 (verb) + 4 (crc) = 48 bytes
        const MIN_FRAME_SIZE: usize = 4 + 2 + 2 + TOKEN_LEN + 4 + 4;
        if data.len() < MIN_FRAME_SIZE {
            return Err(IpcError::PayloadTruncated);
        }

        // Verify magic bytes
        if data[0..4] != MAGIC_BYTES {
            return Err(IpcError::InvalidMagic);
        }

        // Verify version
        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != PROTOCOL_VERSION {
            return Err(IpcError::UnsupportedVersion { found: version });
        }

        // Verify payload length
        let payload_len = u16::from_le_bytes([data[6], data[7]]) as usize;
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(IpcError::PayloadTooLarge {
                length: payload_len,
            });
        }

        let expected_total_len = 4 + 2 + 2 + TOKEN_LEN + 4 + payload_len + 4;
        if data.len() < expected_total_len {
            return Err(IpcError::PayloadTruncated);
        }

        // Frame slice covering data protected by CRC
        let body_len = 4 + 2 + 2 + TOKEN_LEN + 4 + payload_len;
        let body_slice = &data[..body_len];
        let crc_slice = &data[body_len..body_len + 4];
        let expected_crc =
            u32::from_le_bytes([crc_slice[0], crc_slice[1], crc_slice[2], crc_slice[3]]);
        let calculated_crc = crc32(body_slice);

        if calculated_crc != expected_crc {
            return Err(IpcError::CrcMismatch {
                expected: expected_crc,
                calculated: calculated_crc,
            });
        }

        let mut token_bytes = [0u8; TOKEN_LEN];
        token_bytes.copy_from_slice(&data[8..40]);
        let token = SessionToken::from_bytes(token_bytes);

        let verb_raw = u32::from_le_bytes([data[40], data[41], data[42], data[43]]);
        let verb = CommandVerb::from_u32(verb_raw)
            .ok_or(IpcError::InvalidCommandVerb { verb: verb_raw })?;

        let payload = data[44..44 + payload_len].to_vec();

        Ok(Self {
            token,
            verb,
            payload,
        })
    }
}

/// Record of request timestamps for a given caller process.
#[derive(Clone, Debug)]
struct PidRecord {
    pid: u32,
    timestamps: Vec<Instant>,
}

/// Bounded rate limiter enforcing at most 10 requests per second per caller PID.
#[derive(Clone, Debug, Default)]
pub struct PidRateLimiter {
    records: Vec<PidRecord>,
}

impl PidRateLimiter {
    /// Creates a new rate limiter instance.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Checks whether a request from the given PID is allowed at current instant.
    /// Prunes timestamps older than one second and rejects requests when limit is reached.
    pub fn check_and_record(&mut self, pid: u32, now: Instant) -> Result<(), IpcError> {
        let cutoff = now.checked_sub(RATE_LIMIT_WINDOW).unwrap_or(now);

        // Find or create record for pid
        let record = match self.records.iter_mut().find(|r| r.pid == pid) {
            Some(entry) => entry,
            None => {
                self.records.push(PidRecord {
                    pid,
                    timestamps: Vec::with_capacity(MAX_REQUESTS_PER_SECOND + 1),
                });
                self.records.last_mut().unwrap()
            }
        };

        // Retain only requests within the sliding window
        record.timestamps.retain(|&t| t > cutoff);

        if record.timestamps.len() >= MAX_REQUESTS_PER_SECOND {
            return Err(IpcError::RateLimitExceeded { pid });
        }

        record.timestamps.push(now);
        Ok(())
    }

    /// Prunes stale records whose timestamps are all expired to keep memory bounded.
    pub fn prune_idle_callers(&mut self, now: Instant) {
        let cutoff = now.checked_sub(RATE_LIMIT_WINDOW).unwrap_or(now);
        for record in &mut self.records {
            record.timestamps.retain(|&t| t > cutoff);
        }
        self.records.retain(|record| !record.timestamps.is_empty());
    }

    /// Returns the number of tracked caller PIDs.
    pub fn active_caller_count(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_serialization() {
        let token = SessionToken::from_bytes([0x42; 32]);
        let payload = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let frame = IpcFrame::new(token, CommandVerb::RequestAcquire, payload.clone())
            .expect("frame creation should succeed");

        let serialized = frame.serialize();
        assert_eq!(&serialized[0..4], &MAGIC_BYTES);

        let deserialized =
            IpcFrame::deserialize(&serialized).expect("deserialization should succeed");

        assert_eq!(deserialized.token, token);
        assert_eq!(deserialized.verb, CommandVerb::RequestAcquire);
        assert_eq!(deserialized.payload, payload);
    }

    #[test]
    fn rejection_of_invalid_magic() {
        let token = SessionToken::from_bytes([0xAA; 32]);
        let frame = IpcFrame::new(token, CommandVerb::QueryStatus, vec![]).expect("frame creation");
        let mut bytes = frame.serialize();
        bytes[0] = b'X';

        let result = IpcFrame::deserialize(&bytes);
        assert_eq!(result, Err(IpcError::InvalidMagic));
    }

    #[test]
    fn rejection_of_unsupported_version() {
        let token = SessionToken::from_bytes([0xBB; 32]);
        let frame = IpcFrame::new(token, CommandVerb::QueryStatus, vec![]).expect("frame creation");
        let mut bytes = frame.serialize();
        bytes[4] = 0x02;
        bytes[5] = 0x00;

        // Recompute CRC so version check fails before CRC check
        let body_len = bytes.len() - 4;
        let new_crc = crc32(&bytes[..body_len]);
        bytes[body_len..body_len + 4].copy_from_slice(&new_crc.to_le_bytes());

        let result = IpcFrame::deserialize(&bytes);
        assert_eq!(result, Err(IpcError::UnsupportedVersion { found: 0x0002 }));
    }

    #[test]
    fn rejection_of_payload_exceeding_max_len() {
        let token = SessionToken::from_bytes([0x01; 32]);
        let oversize_payload = vec![0x7A; 257];

        let frame_result = IpcFrame::new(token, CommandVerb::ScheduleAction, oversize_payload);
        assert_eq!(frame_result, Err(IpcError::PayloadTooLarge { length: 257 }));

        // Also test crafted packet with declared length 257
        let valid_frame =
            IpcFrame::new(token, CommandVerb::ScheduleAction, vec![0x7A; 10]).unwrap();
        let mut raw = valid_frame.serialize();
        raw[6] = 0x01;
        raw[7] = 0x01; // 257 little-endian

        let result = IpcFrame::deserialize(&raw);
        assert_eq!(result, Err(IpcError::PayloadTooLarge { length: 257 }));
    }

    #[test]
    fn rejection_of_out_of_bounds_interval() {
        // Valid symbolic resolution
        assert!(validate_interval_hns(5_000).is_ok());

        // Valid physical bounds
        assert!(validate_interval_hns(10_000).is_ok());
        assert!(validate_interval_hns(156_250).is_ok());

        // Below minimum
        assert_eq!(
            validate_interval_hns(0),
            Err(IpcError::IntervalOutOfBounds { requested: 0 })
        );
        assert_eq!(
            validate_interval_hns(4_999),
            Err(IpcError::IntervalOutOfBounds { requested: 4_999 })
        );

        // Above maximum
        assert_eq!(
            validate_interval_hns(156_251),
            Err(IpcError::IntervalOutOfBounds { requested: 156_251 })
        );
        assert_eq!(
            validate_interval_hns(1_000_000),
            Err(IpcError::IntervalOutOfBounds {
                requested: 1_000_000
            })
        );
    }

    #[test]
    fn crc32_corruption_detection() {
        let token = SessionToken::from_bytes([0xCC; 32]);
        let frame = IpcFrame::new(token, CommandVerb::CancelSchedule, vec![0x10, 0x20]).unwrap();
        let mut bytes = frame.serialize();

        // Mutate a byte in the payload
        let last_payload_idx = bytes.len() - 5;
        bytes[last_payload_idx] ^= 0xFF;

        let result = IpcFrame::deserialize(&bytes);
        match result {
            Err(IpcError::CrcMismatch { .. }) => {}
            other => panic!("expected CrcMismatch, got {other:?}"),
        }
    }

    #[test]
    fn token_validation_and_matching() {
        let token_a = SessionToken::from_bytes([0x11; 32]);
        let token_b = SessionToken::from_bytes([0x11; 32]);
        let token_c = SessionToken::from_bytes([0x22; 32]);

        assert!(token_a.matches(&token_b));
        assert!(!token_a.matches(&token_c));

        let generated = SessionToken::generate_ephemeral();
        assert_eq!(generated.as_bytes().len(), 32);
        assert!(generated.matches(&generated));
    }

    #[test]
    fn rate_limiter_allows_ten_and_rejects_eleventh() {
        let mut limiter = PidRateLimiter::new();
        let pid = 1234;
        let base_instant = Instant::now();

        // 10 requests within same second should all succeed
        for i in 0..10 {
            let t = base_instant + Duration::from_millis(i * 50);
            assert!(limiter.check_and_record(pid, t).is_ok());
        }

        // 11th request within the window must be throttled
        let eleventh_instant = base_instant + Duration::from_millis(600);
        assert_eq!(
            limiter.check_and_record(pid, eleventh_instant),
            Err(IpcError::RateLimitExceeded { pid })
        );

        // After window expires, requests should succeed again
        let next_window = base_instant + Duration::from_millis(1100);
        assert!(limiter.check_and_record(pid, next_window).is_ok());
    }

    #[test]
    fn rate_limiter_isolates_separate_pids() {
        let mut limiter = PidRateLimiter::new();
        let pid1 = 100;
        let pid2 = 200;
        let now = Instant::now();

        for _ in 0..10 {
            assert!(limiter.check_and_record(pid1, now).is_ok());
        }

        // pid1 is exhausted
        assert_eq!(
            limiter.check_and_record(pid1, now),
            Err(IpcError::RateLimitExceeded { pid: pid1 })
        );

        // pid2 is unaffected
        assert!(limiter.check_and_record(pid2, now).is_ok());
    }

    #[test]
    fn rate_limiter_pruning() {
        let mut limiter = PidRateLimiter::new();
        let pid = 4321;
        let now = Instant::now();

        assert!(limiter.check_and_record(pid, now).is_ok());
        assert_eq!(limiter.active_caller_count(), 1);

        let future = now + Duration::from_secs(2);
        limiter.prune_idle_callers(future);
        assert_eq!(limiter.active_caller_count(), 0);
    }
}
