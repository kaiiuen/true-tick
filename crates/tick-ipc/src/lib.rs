//! Inter-Process Communication and wire protocol implementation for True Tick.
//!
//! Enforces zero-trust bounded frames, CRC-32 integrity validation,
//! strict timer interval boundaries, session token authentication,
//! and per-PID rate limiting.

use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};
use tick_core::Hns;

#[cfg(windows)]
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

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

/// File name used to publish the ephemeral session token for CLI clients.
pub const TOKEN_FILE_NAME: &str = "ipc-token";

/// Named pipe endpoint served by the tray process for IPC clients.
pub const PIPE_NAME: &str = r"\\.\pipe\TrueTick-Ipc-v1";

/// Tag byte identifying a Start action inside a schedule request payload.
pub const SCHEDULE_TAG_START: u8 = 1;

/// Tag byte identifying a Stop action inside a schedule request payload.
pub const SCHEDULE_TAG_STOP: u8 = 2;

/// Tag byte identifying a Pause action inside a schedule request payload.
pub const SCHEDULE_TAG_PAUSE: u8 = 3;

/// Maximum allowed IPC requests per second for a single caller PID.
pub const MAX_REQUESTS_PER_SECOND: usize = 10;

/// Window duration for caller PID rate limiting.
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);

/// Maximum number of distinct caller PIDs tracked by the rate limiter.
/// Once this bound is reached, the least recently seen record is evicted.
pub const MAX_TRACKED_PIDS: usize = 64;

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

    /// Generates an ephemeral session token using OS provided entropy
    /// backed by BCryptGenRandom on Windows with a secure OS fallback elsewhere
    pub fn generate_ephemeral() -> Self {
        let mut bytes = [0u8; TOKEN_LEN];
        fill_os_entropy(&mut bytes);
        Self(bytes)
    }

    /// Encodes the token as exactly 64 lowercase hexadecimal characters.
    pub fn to_hex(&self) -> String {
        const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(TOKEN_LEN * 2);
        for &byte in self.as_bytes() {
            out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
            out.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
        }
        out
    }

    /// Parses a token from exactly 64 lowercase hexadecimal characters.
    /// Any other input, including uppercase hex, is rejected as malformed.
    pub fn from_hex(text: &str) -> Result<Self, IpcError> {
        let bytes = text.as_bytes();
        if bytes.len() != TOKEN_LEN * 2 {
            return Err(IpcError::MalformedToken);
        }
        let mut out = [0u8; TOKEN_LEN];
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            let high = hex_nibble(pair[0]).ok_or(IpcError::MalformedToken)?;
            let low = hex_nibble(pair[1]).ok_or(IpcError::MalformedToken)?;
            out[index] = (high << 4) | low;
        }
        Ok(Self(out))
    }
}

/// Decodes one lowercase hexadecimal digit, rejecting every other byte.
const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(windows)]
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(handle: *mut u8, buffer: *mut u8, length: u32, flags: u32) -> i32;
}

/// Fills the token buffer with OS entropy on Windows using BCryptGenRandom
/// and fails securely without falling back to deterministic output
#[cfg(windows)]
fn fill_os_entropy(bytes: &mut [u8; TOKEN_LEN]) {
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    assert!(status == 0, "BCryptGenRandom failed to provide OS entropy");
}

/// Fills the token buffer with OS entropy outside Windows by reading urandom
/// and fails securely when OS entropy is unavailable
#[cfg(not(windows))]
fn fill_os_entropy(bytes: &mut [u8; TOKEN_LEN]) {
    use std::io::Read;
    let mut source = std::fs::File::open("/dev/urandom").expect("OS entropy unavailable");
    source.read_exact(bytes).expect("OS entropy unavailable");
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
    MalformedToken,
    MalformedResponse,
    TransportIo { raw_os_error: i32 },
    Unsupported,
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
            Self::MalformedToken => write!(f, "malformed session token encoding"),
            Self::MalformedResponse => write!(f, "malformed IPC response frame"),
            Self::TransportIo { raw_os_error } => {
                write!(f, "IPC transport I/O failure, OS error code {raw_os_error}")
            }
            Self::Unsupported => write!(f, "operation is not supported on this platform"),
        }
    }
}

impl IpcError {
    /// Stable numeric code for each variant used inside Error responses.
    /// Variant payloads are not carried by the code, only the discriminant.
    pub const fn code(&self) -> u32 {
        match self {
            Self::InvalidMagic => 1,
            Self::UnsupportedVersion { .. } => 2,
            Self::PayloadTooLarge { .. } => 3,
            Self::PayloadTruncated => 4,
            Self::InvalidCommandVerb { .. } => 5,
            Self::IntervalOutOfBounds { .. } => 6,
            Self::CrcMismatch { .. } => 7,
            Self::UnauthorizedToken => 8,
            Self::RateLimitExceeded { .. } => 9,
            Self::MalformedToken => 10,
            Self::MalformedResponse => 11,
            Self::TransportIo { .. } => 12,
            Self::Unsupported => 13,
        }
    }

    /// Reconstructs the variant for a wire code with zeroed payload fields.
    /// Unknown codes return None so callers can reject malformed replies.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::InvalidMagic),
            2 => Some(Self::UnsupportedVersion { found: 0 }),
            3 => Some(Self::PayloadTooLarge { length: 0 }),
            4 => Some(Self::PayloadTruncated),
            5 => Some(Self::InvalidCommandVerb { verb: 0 }),
            6 => Some(Self::IntervalOutOfBounds { requested: 0 }),
            7 => Some(Self::CrcMismatch {
                expected: 0,
                calculated: 0,
            }),
            8 => Some(Self::UnauthorizedToken),
            9 => Some(Self::RateLimitExceeded { pid: 0 }),
            10 => Some(Self::MalformedToken),
            11 => Some(Self::MalformedResponse),
            12 => Some(Self::TransportIo { raw_os_error: 0 }),
            13 => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// Bridges transport layer I/O failures into the protocol error type.
/// Only the raw OS error code is retained so IpcError stays Copy.
impl From<std::io::Error> for IpcError {
    fn from(error: std::io::Error) -> Self {
        Self::TransportIo {
            raw_os_error: error.raw_os_error().unwrap_or(-1),
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

/// Writes the session token to a file as 64 lowercase hex characters
/// without a trailing newline.
///
/// The file relies on the inherited directory ACL of the application state
/// directory, which is owner scoped under the existing portable DACL posture,
/// so the token is protected by the parent directory permissions rather than
/// a file level descriptor. Installing an explicit ACL on the token file is a
/// deliberate future refinement tracked by the hardening roadmap.
pub fn write_token_file(path: &Path, token: &SessionToken) -> std::io::Result<()> {
    std::fs::write(path, token.to_hex())
}

/// Reads a token file previously written by `write_token_file` and parses the
/// hex form back into a session token. Parse failures surface as an io error
/// wrapping IpcError so callers need only handle one error channel.
pub fn read_token_file(path: &Path) -> std::io::Result<SessionToken> {
    let text = std::fs::read_to_string(path)?;
    SessionToken::from_hex(text.trim_end_matches(&['\r', '\n'][..]))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// Server reply payloads shared between the tray process and IPC clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IpcResponse {
    Status(String),
    Acquired { effective_hns: u64 },
    Released,
    Scheduled { action_id: u32 },
    Cancelled,
    Error(IpcError),
}

/// Response wire format shared between the server and its clients.
///
/// Byte 0 is a single tag byte, followed by a fixed or variable payload:
/// - tag 1 Status, payload is the UTF-8 status text
/// - tag 2 Acquired, payload is 8 bytes little endian effective interval in HNS
/// - tag 3 Released, no payload
/// - tag 4 Scheduled, payload is 4 bytes little endian action identifier
/// - tag 5 Cancelled, no payload
/// - tag 6 Error, payload is 4 bytes little endian carrying IpcError::code()
///
/// Any other tag byte, a truncated payload, trailing bytes, or a Status
/// payload that is not valid UTF-8 is rejected as MalformedResponse.
pub fn encode_response(response: &IpcResponse) -> Vec<u8> {
    let mut out = Vec::new();
    match response {
        IpcResponse::Status(text) => {
            out.push(1u8);
            out.extend_from_slice(text.as_bytes());
        }
        IpcResponse::Acquired { effective_hns } => {
            out.push(2u8);
            out.extend_from_slice(&effective_hns.to_le_bytes());
        }
        IpcResponse::Released => out.push(3u8),
        IpcResponse::Scheduled { action_id } => {
            out.push(4u8);
            out.extend_from_slice(&action_id.to_le_bytes());
        }
        IpcResponse::Cancelled => out.push(5u8),
        IpcResponse::Error(error) => {
            out.push(6u8);
            out.extend_from_slice(&error.code().to_le_bytes());
        }
    }
    out
}

/// Decodes one response frame in the wire format described on
/// `encode_response`. Rejects unknown tags, truncated or trailing payload
/// bytes, non UTF-8 Status payloads, and unmapped error codes.
pub fn decode_response(bytes: &[u8]) -> Result<IpcResponse, IpcError> {
    let (&tag, payload) = bytes.split_first().ok_or(IpcError::MalformedResponse)?;
    match tag {
        1 => {
            let text = std::str::from_utf8(payload).map_err(|_| IpcError::MalformedResponse)?;
            Ok(IpcResponse::Status(text.to_owned()))
        }
        2 => {
            let fixed: &[u8; 8] = payload
                .try_into()
                .map_err(|_| IpcError::MalformedResponse)?;
            Ok(IpcResponse::Acquired {
                effective_hns: u64::from_le_bytes(*fixed),
            })
        }
        3 => {
            if payload.is_empty() {
                Ok(IpcResponse::Released)
            } else {
                Err(IpcError::MalformedResponse)
            }
        }
        4 => {
            let fixed: &[u8; 4] = payload
                .try_into()
                .map_err(|_| IpcError::MalformedResponse)?;
            Ok(IpcResponse::Scheduled {
                action_id: u32::from_le_bytes(*fixed),
            })
        }
        5 => {
            if payload.is_empty() {
                Ok(IpcResponse::Cancelled)
            } else {
                Err(IpcError::MalformedResponse)
            }
        }
        6 => {
            let fixed: &[u8; 4] = payload
                .try_into()
                .map_err(|_| IpcError::MalformedResponse)?;
            let code = u32::from_le_bytes(*fixed);
            let error = IpcError::from_code(code).ok_or(IpcError::MalformedResponse)?;
            Ok(IpcResponse::Error(error))
        }
        _ => Err(IpcError::MalformedResponse),
    }
}

/// Encodes the payload for a RequestAcquire frame after enforcing the same
/// interval bounds the server applies. Returns 8 bytes little endian.
pub fn encode_interval_payload(interval_hns: u64) -> Result<Vec<u8>, IpcError> {
    let validated = validate_interval_hns(interval_hns)?;
    Ok(validated.value().to_le_bytes().to_vec())
}

/// Kind of timed action a ScheduleAction request asks the server to arm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleActionKind {
    Start,
    Stop,
    Pause,
}

impl ScheduleActionKind {
    /// Tag byte placed at the start of a schedule request payload.
    pub const fn tag(self) -> u8 {
        match self {
            Self::Start => SCHEDULE_TAG_START,
            Self::Stop => SCHEDULE_TAG_STOP,
            Self::Pause => SCHEDULE_TAG_PAUSE,
        }
    }
}

/// Encodes a ScheduleAction request payload as one tag byte followed by
/// 4 bytes little endian carrying the delay in seconds.
pub fn encode_schedule_payload(kind: ScheduleActionKind, seconds: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    out.push(kind.tag());
    out.extend_from_slice(&seconds.to_le_bytes());
    out
}

/// Record of request timestamps for a given caller process.
#[derive(Clone, Debug)]
struct PidRecord {
    pid: u32,
    timestamps: Vec<Instant>,
    last_seen: Instant,
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
        let index = match self.records.iter().position(|r| r.pid == pid) {
            Some(existing) => existing,
            None => {
                if self.records.len() >= MAX_TRACKED_PIDS {
                    self.evict_least_recently_seen();
                }
                self.records.push(PidRecord {
                    pid,
                    timestamps: Vec::with_capacity(MAX_REQUESTS_PER_SECOND + 1),
                    last_seen: now,
                });
                self.records.len() - 1
            }
        };
        let record = &mut self.records[index];
        record.last_seen = now;

        // Retain only requests within the sliding window
        record.timestamps.retain(|&t| t > cutoff);

        if record.timestamps.len() >= MAX_REQUESTS_PER_SECOND {
            return Err(IpcError::RateLimitExceeded { pid });
        }

        record.timestamps.push(now);
        Ok(())
    }

    /// Removes the record with the oldest last_seen instant.
    /// On a tie, removes the record with the smallest PID for deterministic eviction.
    fn evict_least_recently_seen(&mut self) {
        let index = self
            .records
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.last_seen
                    .cmp(&b.last_seen)
                    .then_with(|| a.pid.cmp(&b.pid))
            })
            .map(|(index, _)| index);
        if let Some(index) = index {
            self.records.remove(index);
        }
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

// Client side transport over the named pipe endpoint.
//
// Windows builds use kernel32 FFI declared locally in the same style as the
// BCrypt declarations above. Non Windows builds expose a stub client whose
// constructor returns an io error wrapping IpcError::Unsupported.
#[cfg(windows)]
mod windows_pipe {
    use super::*;
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const PIPE_READMODE_MESSAGE: u32 = 0x0000_0002;
    const ERROR_PIPE_BUSY: i32 = 231;
    const INVALID_HANDLE_VALUE: isize = -1;

    type Handle = isize;
    type Bool = i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share_mode: u32,
            security_attributes: *mut u8,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: Handle,
        ) -> Handle;
        fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> Bool;
        fn SetNamedPipeHandleState(
            pipe: Handle,
            mode: *mut u32,
            max_collection_count: *mut u32,
            collect_data_timeout: *mut u32,
        ) -> Bool;
        fn GetLastError() -> u32;
        fn WriteFile(
            file: Handle,
            buffer: *const u8,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut u8,
        ) -> Bool;
        fn ReadFile(
            file: Handle,
            buffer: *mut u8,
            bytes_to_read: u32,
            bytes_read: *mut u32,
            overlapped: *mut u8,
        ) -> Bool;
        fn CloseHandle(handle: Handle) -> Bool;
    }

    /// Converts a Rust string into a null terminated UTF-16 buffer.
    fn wide_null(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// RAII wrapper closing the pipe handle exactly once on drop.
    struct PipeHandle(Handle);

    impl PipeHandle {
        fn raw(&self) -> Handle {
            self.0
        }
    }

    impl Drop for PipeHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Named pipe client speaking the True Tick IPC wire protocol.
    /// The handle is closed automatically when the client is dropped.
    pub struct IpcClient {
        handle: PipeHandle,
    }

    /// Connects to the tray server pipe, retrying while the server reports
    /// the pipe as busy. `busy_retry_ms` is the budget forwarded to
    /// WaitNamedPipeW for each busy retry.
    pub fn connect(pipe_name: &str, busy_retry_ms: u32) -> io::Result<IpcClient> {
        let wide = wide_null(pipe_name);
        loop {
            let raw = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };
            if raw != INVALID_HANDLE_VALUE {
                let mut mode = PIPE_READMODE_MESSAGE;
                let ok = unsafe {
                    SetNamedPipeHandleState(
                        raw,
                        &mut mode,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    let error = io::Error::last_os_error();
                    unsafe {
                        CloseHandle(raw);
                    }
                    return Err(error);
                }
                return Ok(IpcClient {
                    handle: PipeHandle(raw),
                });
            }

            let last = unsafe { GetLastError() } as i32;
            if last != ERROR_PIPE_BUSY {
                return Err(io::Error::last_os_error());
            }

            let waited = unsafe { WaitNamedPipeW(wide.as_ptr(), busy_retry_ms) };
            if waited == 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }

    // WaitNamedPipeW keeps the busy retry inside the caller supplied budget.
    // The pipe is opened in blocking byte agnostic mode then switched to
    // message mode so each ReadFile returns exactly one reply frame.

    impl IpcClient {
        /// Sends one request frame and reads exactly one message mode reply
        /// bounded by MAX_PAYLOAD_LEN, then decodes the response payload.
        /// Transport failures surface through the IpcError::TransportIo
        /// bridging variant so callers only match one error type.
        pub fn exchange(
            &mut self,
            token: &SessionToken,
            verb: CommandVerb,
            payload: &[u8],
        ) -> Result<IpcResponse, IpcError> {
            let frame = IpcFrame::new(*token, verb, payload.to_vec())?;
            let bytes = frame.serialize();

            let mut written: u32 = 0;
            let ok = unsafe {
                WriteFile(
                    self.handle.raw(),
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 || written as usize != bytes.len() {
                return Err(io::Error::last_os_error().into());
            }

            // One reply message can never exceed a full serialized frame, so
            // size the read buffer to the largest legal frame once.
            let mut buffer = vec![0u8; 4 + 2 + 2 + TOKEN_LEN + 4 + MAX_PAYLOAD_LEN + 4];
            let mut read: u32 = 0;
            let ok = unsafe {
                ReadFile(
                    self.handle.raw(),
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error().into());
            }
            buffer.truncate(read as usize);

            // The server writes the raw response encoding straight to the
            // pipe with no outer IpcFrame wrapper so decode it directly.
            decode_response(&buffer)
        }
    }
}

#[cfg(windows)]
pub use windows_pipe::{connect, IpcClient};

/// Stub transport for non Windows builds where the named pipe endpoint does
/// not exist. Construction always fails with an io error carrying
/// IpcError::Unsupported so portable callers get a clean failure channel.
#[cfg(not(windows))]
pub struct IpcClient {
    _private: (),
}

/// Non Windows stub that always fails with IpcError::Unsupported wrapped in
/// an io error, matching the signature of the Windows connect.
#[cfg(not(windows))]
pub fn connect(_pipe_name: &str, _busy_retry_ms: u32) -> std::io::Result<IpcClient> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        IpcError::Unsupported,
    ))
}

#[cfg(not(windows))]
impl IpcClient {
    /// Stub exchange that is unreachable because connect always fails, kept
    /// for API parity across platforms.
    pub fn exchange(
        &mut self,
        _token: &SessionToken,
        _verb: CommandVerb,
        _payload: &[u8],
    ) -> Result<IpcResponse, IpcError> {
        Err(IpcError::Unsupported)
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
    fn ephemeral_tokens_are_unique_across_1000_generations() {
        use std::collections::HashSet;
        let mut seen = HashSet::with_capacity(1000);
        for _ in 0..1000 {
            let token = SessionToken::generate_ephemeral();
            assert!(seen.insert(token));
        }
        assert_eq!(seen.len(), 1000);
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

    #[test]
    fn rate_limiter_evicts_least_recently_seen_at_capacity() {
        let mut limiter = PidRateLimiter::new();
        let base = Instant::now();

        // Fill the limiter to capacity with increasing last_seen instants
        for pid in 1..=MAX_TRACKED_PIDS as u32 {
            let t = base + Duration::from_millis(u64::from(pid));
            assert!(limiter.check_and_record(pid, t).is_ok());
        }
        assert_eq!(limiter.records.len(), MAX_TRACKED_PIDS);

        // New arrival must evict pid 1, which holds the oldest last_seen
        let arrival = base + Duration::from_secs(1);
        assert!(limiter.check_and_record(9999, arrival).is_ok());
        assert_eq!(limiter.records.len(), MAX_TRACKED_PIDS);
        assert!(!limiter.records.iter().any(|r| r.pid == 1));
        assert!(limiter.records.iter().any(|r| r.pid == 9999));
    }

    #[test]
    fn rate_limiter_never_evicts_an_existing_pid_for_a_new_one() {
        let mut limiter = PidRateLimiter::new();
        let base = Instant::now();

        for pid in 1..=MAX_TRACKED_PIDS as u32 {
            let t = base + Duration::from_millis(u64::from(pid));
            assert!(limiter.check_and_record(pid, t).is_ok());
        }

        // Refresh an existing pid so it is the most recently seen record
        let keep_pid = 7u32;
        let refresh = base + Duration::from_secs(2);
        assert!(limiter.check_and_record(keep_pid, refresh).is_ok());

        // A new arrival must evict the least recently seen record, not the
        // existing pid that was just refreshed
        let arrival = refresh + Duration::from_millis(1);
        assert!(limiter.check_and_record(10_000, arrival).is_ok());
        assert!(limiter.records.iter().any(|r| r.pid == keep_pid));
        assert!(!limiter.records.iter().any(|r| r.pid == 1));
        assert_eq!(limiter.records.len(), MAX_TRACKED_PIDS);

        // A request from an already tracked pid at capacity must never evict
        // the pid performing that request
        for pid in 1..=MAX_TRACKED_PIDS as u32 {
            let t = arrival + Duration::from_millis(u64::from(pid) + 1);
            if pid == 1 {
                continue;
            }
            let _ = limiter.check_and_record(pid, t);
            assert!(limiter.records.iter().any(|r| r.pid == pid));
        }
        assert_eq!(limiter.records.len(), MAX_TRACKED_PIDS);
    }

    #[test]
    fn rate_limiter_bounds_memory_under_many_distinct_pids() {
        let mut limiter = PidRateLimiter::new();
        let base = Instant::now();

        for pid in 0..5_000u32 {
            let t = base + Duration::from_micros(u64::from(pid));
            assert!(limiter.check_and_record(pid, t).is_ok());
            assert!(limiter.records.len() <= MAX_TRACKED_PIDS);
        }
        assert!(limiter.active_caller_count() <= MAX_TRACKED_PIDS);
    }

    #[test]
    fn eviction_tie_breaks_on_smallest_pid_deterministically() {
        let mut limiter = PidRateLimiter::new();
        let now = Instant::now();

        // Every record shares the same last_seen instant
        for pid in 50..50 + MAX_TRACKED_PIDS as u32 {
            assert!(limiter.check_and_record(pid, now).is_ok());
        }
        assert_eq!(limiter.records.len(), MAX_TRACKED_PIDS);

        // New arrival evicts pid 50, the smallest pid on the last_seen tie
        assert!(limiter.check_and_record(9999, now).is_ok());
        assert!(!limiter.records.iter().any(|r| r.pid == 50));
        assert!(limiter.records.iter().any(|r| r.pid == 9999));

        // Repeating the tie evicts the next smallest pid deterministically
        assert!(limiter.check_and_record(9998, now).is_ok());
        assert!(!limiter.records.iter().any(|r| r.pid == 51));
        assert_eq!(limiter.records.len(), MAX_TRACKED_PIDS);
    }

    #[test]
    fn token_hex_round_trip() {
        let token = SessionToken::generate_ephemeral();
        let hex = token.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let parsed = SessionToken::from_hex(&hex).expect("hex round trip should parse");
        assert!(token.matches(&parsed));

        let fixed = SessionToken::from_bytes([0xAB; 32]);
        assert_eq!(fixed.to_hex(), "ab".repeat(32));
    }

    #[test]
    fn token_from_hex_rejects_wrong_length_and_non_hex() {
        assert_eq!(SessionToken::from_hex(""), Err(IpcError::MalformedToken));
        assert_eq!(
            SessionToken::from_hex(&"a".repeat(63)),
            Err(IpcError::MalformedToken)
        );
        assert_eq!(
            SessionToken::from_hex(&"a".repeat(65)),
            Err(IpcError::MalformedToken)
        );
        // Non hex character in final position
        let mut bad = "a".repeat(64);
        bad.replace_range(63.., "g");
        assert_eq!(SessionToken::from_hex(&bad), Err(IpcError::MalformedToken));
        // Uppercase hex is rejected by the strict lowercase contract
        let upper = "A".repeat(64);
        assert_eq!(
            SessionToken::from_hex(&upper),
            Err(IpcError::MalformedToken)
        );
    }

    #[test]
    fn token_file_write_then_read_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "tick-ipc-token-test-{}-{}",
            std::process::id(),
            7u32
        ));
        std::fs::create_dir_all(&dir).expect("temp dir creation");
        let path = dir.join(TOKEN_FILE_NAME);

        let token = SessionToken::generate_ephemeral();
        write_token_file(&path, &token).expect("token file write");

        // File must contain exactly 64 hex characters with no trailing newline
        let raw = std::fs::read(&path).expect("token file read raw");
        assert_eq!(raw.len(), 64);

        let parsed = read_token_file(&path).expect("token file parse");
        assert!(token.matches(&parsed));

        std::fs::remove_file(&path).expect("cleanup token file");
        std::fs::remove_dir(&dir).expect("cleanup token dir");
    }

    #[test]
    fn response_round_trip_for_every_variant() {
        let cases = vec![
            IpcResponse::Status(String::from("resolution locked")),
            IpcResponse::Status(String::new()),
            IpcResponse::Acquired {
                effective_hns: 0x0123_4567_89AB_CDEF,
            },
            IpcResponse::Released,
            IpcResponse::Scheduled {
                action_id: 0xDEAD_BEEF,
            },
            IpcResponse::Cancelled,
            IpcResponse::Error(IpcError::UnauthorizedToken),
            IpcResponse::Error(IpcError::RateLimitExceeded { pid: 0 }),
        ];
        for response in cases {
            let encoded = encode_response(&response);
            let decoded = decode_response(&encoded).expect("response decode");
            assert_eq!(decoded, response);
        }
    }

    #[test]
    fn decode_response_rejects_unknown_tag() {
        assert_eq!(decode_response(&[]), Err(IpcError::MalformedResponse));
        assert_eq!(decode_response(&[0x00]), Err(IpcError::MalformedResponse));
        assert_eq!(
            decode_response(&[0x07, 0x01]),
            Err(IpcError::MalformedResponse)
        );
    }

    #[test]
    fn decode_response_rejects_truncated_payload() {
        // Acquired wants 8 payload bytes
        assert_eq!(
            decode_response(&[2, 0x01, 0x02, 0x03]),
            Err(IpcError::MalformedResponse)
        );
        // Scheduled wants 4 payload bytes
        assert_eq!(
            decode_response(&[4, 0x01, 0x02]),
            Err(IpcError::MalformedResponse)
        );
        // Error wants 4 payload bytes
        assert_eq!(
            decode_response(&[6, 0x08]),
            Err(IpcError::MalformedResponse)
        );
    }

    #[test]
    fn decode_response_rejects_trailing_bytes() {
        // Released and Cancelled carry no payload at all
        assert_eq!(
            decode_response(&[3, 0x00]),
            Err(IpcError::MalformedResponse)
        );
        assert_eq!(
            decode_response(&[5, 0xFF, 0xFF]),
            Err(IpcError::MalformedResponse)
        );
        // Fixed size payloads reject extra bytes too
        let mut acquired = encode_response(&IpcResponse::Acquired { effective_hns: 8 });
        acquired.push(0x99);
        assert_eq!(decode_response(&acquired), Err(IpcError::MalformedResponse));
    }

    #[test]
    fn decode_response_rejects_invalid_utf8_status() {
        let bytes = [1u8, 0xFF, 0xFE, 0x80];
        assert_eq!(decode_response(&bytes), Err(IpcError::MalformedResponse));
    }

    #[test]
    fn decode_response_rejects_unmapped_error_code() {
        let mut bytes = vec![6u8];
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_response(&bytes), Err(IpcError::MalformedResponse));
    }

    #[test]
    fn ipc_error_code_round_trip_for_every_variant() {
        let variants = [
            IpcError::InvalidMagic,
            IpcError::UnsupportedVersion { found: 0x0002 },
            IpcError::PayloadTooLarge { length: 512 },
            IpcError::PayloadTruncated,
            IpcError::InvalidCommandVerb { verb: 9 },
            IpcError::IntervalOutOfBounds { requested: 1 },
            IpcError::CrcMismatch {
                expected: 1,
                calculated: 2,
            },
            IpcError::UnauthorizedToken,
            IpcError::RateLimitExceeded { pid: 42 },
            IpcError::MalformedToken,
            IpcError::MalformedResponse,
            IpcError::TransportIo { raw_os_error: 5 },
            IpcError::Unsupported,
        ];
        let mut seen_codes = std::collections::HashSet::new();
        for error in variants {
            let code = error.code();
            assert!(seen_codes.insert(code), "codes must be distinct");
            let restored = IpcError::from_code(code).expect("known code maps back");
            assert_eq!(restored.code(), code, "code mapping must round trip");
            assert_eq!(
                std::mem::discriminant(&restored),
                std::mem::discriminant(&error)
            );
        }
        assert_eq!(IpcError::from_code(0), None);
        assert_eq!(IpcError::from_code(u32::MAX), None);
    }

    #[test]
    fn interval_payload_rejects_out_of_bounds_and_encodes_in_range() {
        assert_eq!(
            encode_interval_payload(MIN_INTERVAL_HNS - 1),
            Err(IpcError::IntervalOutOfBounds {
                requested: MIN_INTERVAL_HNS - 1
            })
        );
        assert_eq!(
            encode_interval_payload(MAX_INTERVAL_HNS + 1),
            Err(IpcError::IntervalOutOfBounds {
                requested: MAX_INTERVAL_HNS + 1
            })
        );

        let encoded = encode_interval_payload(SYMBOLIC_RESOLUTION_HNS)
            .expect("symbolic interval should encode");
        assert_eq!(encoded.len(), 8);
        assert_eq!(
            u64::from_le_bytes(encoded.try_into().unwrap()),
            SYMBOLIC_RESOLUTION_HNS
        );

        let upper = encode_interval_payload(MAX_INTERVAL_HNS).expect("max interval should encode");
        assert_eq!(
            u64::from_le_bytes(upper.try_into().unwrap()),
            MAX_INTERVAL_HNS
        );
    }

    #[test]
    fn schedule_payload_layout() {
        let seconds = 0x0102_0304u32;
        for (kind, tag) in [
            (ScheduleActionKind::Start, SCHEDULE_TAG_START),
            (ScheduleActionKind::Stop, SCHEDULE_TAG_STOP),
            (ScheduleActionKind::Pause, SCHEDULE_TAG_PAUSE),
        ] {
            let payload = encode_schedule_payload(kind, seconds);
            assert_eq!(payload.len(), 5);
            assert_eq!(payload[0], tag);
            assert_eq!(payload[1..], seconds.to_le_bytes());
        }
        // Tag values are stable and distinct
        assert_eq!(ScheduleActionKind::Start.tag(), 1);
        assert_eq!(ScheduleActionKind::Stop.tag(), 2);
        assert_eq!(ScheduleActionKind::Pause.tag(), 3);
    }
}
