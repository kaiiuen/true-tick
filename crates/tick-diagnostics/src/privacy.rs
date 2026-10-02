//! Bounded PII detection and redaction for diagnostic record fields.
//!
//! The diagnostic store claims a zero PII guarantee for recorded events. This
//! module enforces that guarantee by scanning detail text for common machine
//! and user identity patterns before storage. Detection is heuristic and
//! bounded: at most `MAX_PII_FINDINGS` findings are returned for any input so a
//! pathological payload cannot produce unbounded output. Every detected span
//! is replaced with the fixed token `[redacted]` so no identity bytes remain
//! in the stored record.

/// Whether the store runs PII sanitization over recorded detail fields.
pub const PII_SCAN_ENABLED: bool = true;

/// Maximum number of PII findings returned by a single scan.
pub const MAX_PII_FINDINGS: usize = 32;

/// Classification of a detected PII pattern.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PiiKind {
    /// A Windows user profile path such as `C:\Users\name`.
    UserProfilePath,
    /// A machine name token such as `COMPUTERNAME` or a `\\HOST` share prefix.
    MachineName,
    /// A percent delimited environment variable such as `%USERNAME%`.
    EnvironmentVariable,
    /// A dotted decimal IPv4 address.
    IpAddress,
    /// An email address with a local part, at sign, and dotted domain.
    EmailAddress,
    /// A domain and account pair such as `DOMAIN\name`.
    WindowsAccountName,
}

/// A single detected PII span.
///
/// `offset` is the byte offset of the span start within the scanned text.
/// Offsets refer to positions in the original input, not to any sanitized
/// output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PiiFinding {
    pub kind: PiiKind,
    pub field: &'static str,
    pub offset: usize,
}

/// A detected span with its byte extent within the scanned text.
#[derive(Clone, Copy, Debug)]
struct PiiSpan {
    start: usize,
    end: usize,
    kind: PiiKind,
}

/// Returns true when the byte can appear inside a path or share segment.
fn is_segment_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace()
        && !matches!(
            byte,
            b'"' | b'\'' | b'<' | b'>' | b'|' | b'?' | b'*' | b':' | b';'
        )
}

/// Returns true when the byte can appear inside an identifier or host label.
fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

/// Returns true when the byte can appear inside an email local part.
fn is_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'.' | b'_'
                | b'%'
                | b'+'
                | b'-'
                | b'='
                | b'!'
                | b'#'
                | b'$'
                | b'\''
                | b'*'
                | b'/'
                | b'?'
                | b'^'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
                | b'&'
        )
}

/// Returns true when the byte can appear inside a domain label.
fn is_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// Scans for drive rooted and relative Windows user profile paths.
///
/// Detects `X:\Users\` for any ASCII drive letter and any other occurrence of
/// the literal `\Users\` fragment. The reported span covers the matched path
/// text including trailing segment names when present.
fn scan_user_profile_path(bytes: &[u8], spans: &mut Vec<PiiSpan>, cap: usize) {
    let mut index = 0;
    while index < bytes.len() {
        let base_end = if index + 9 <= bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && bytes[index + 2] == b'\\'
            && bytes[index + 3..index + 9].eq_ignore_ascii_case(b"users\\")
        {
            index + 9
        } else if index + 7 <= bytes.len()
            && bytes[index] == b'\\'
            && bytes[index + 1..index + 7].eq_ignore_ascii_case(b"users\\")
        {
            index + 7
        } else {
            index += 1;
            continue;
        };
        let mut end = base_end;
        while end < bytes.len() && is_segment_byte(bytes[end]) {
            end += 1;
        }
        while end > index && matches!(bytes[end - 1], b'\\' | b'/' | b'.' | b',') {
            end -= 1;
        }
        spans.push(PiiSpan {
            start: index,
            end,
            kind: PiiKind::UserProfilePath,
        });
        if spans.len() >= cap {
            return;
        }
        index = end;
    }
}

/// Scans for machine name tokens and UNC share prefixes.
///
/// Detects the literal `COMPUTERNAME` token and any `\\` prefix followed by a
/// hostname pattern. The percent delimited form `%COMPUTERNAME%` is reported
/// by the environment variable scanner and is skipped here.
fn scan_machine_name(bytes: &[u8], spans: &mut Vec<PiiSpan>, cap: usize) {
    let mut index = 0;
    while index < bytes.len() {
        if index + 12 <= bytes.len()
            && bytes[index..index + 12].eq_ignore_ascii_case(b"COMPUTERNAME")
            && (index == 0 || bytes[index - 1] != b'%')
        {
            let mut end = index + 12;
            if end < bytes.len() && bytes[end] == b'%' {
                end += 1;
            }
            spans.push(PiiSpan {
                start: index,
                end,
                kind: PiiKind::MachineName,
            });
            if spans.len() >= cap {
                return;
            }
            index = end;
            continue;
        }
        if index + 2 < bytes.len()
            && bytes[index] == b'\\'
            && bytes[index + 1] == b'\\'
            && is_ident_byte(bytes[index + 2])
        {
            let mut end = index + 2;
            while end < bytes.len() && is_ident_byte(bytes[end]) {
                end += 1;
            }
            spans.push(PiiSpan {
                start: index,
                end,
                kind: PiiKind::MachineName,
            });
            if spans.len() >= cap {
                return;
            }
            index = end;
            continue;
        }
        index += 1;
    }
}

/// Scans for percent delimited environment variable tokens such as `%PATH%`.
fn scan_environment_variable(bytes: &[u8], spans: &mut Vec<PiiSpan>, cap: usize) {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && bytes[end] != b'%' {
            if bytes[end].is_ascii_whitespace() || end - index > 64 {
                break;
            }
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'%' && end > index + 1 {
            spans.push(PiiSpan {
                start: index,
                end: end + 1,
                kind: PiiKind::EnvironmentVariable,
            });
            if spans.len() >= cap {
                return;
            }
            index = end + 1;
            continue;
        }
        index += 1;
    }
}

/// Scans for dotted decimal IPv4 addresses with four octets of one to three
/// digits each.
fn scan_ip_address(bytes: &[u8], spans: &mut Vec<PiiSpan>, cap: usize) {
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let mut position = index;
        let mut octets = 0usize;
        let mut valid = true;
        let mut end = index;
        while octets < 4 {
            let octet_start = position;
            while position < bytes.len()
                && bytes[position].is_ascii_digit()
                && position - octet_start < 3
            {
                position += 1;
            }
            let digit_count = position - octet_start;
            if digit_count == 0 {
                valid = false;
                break;
            }
            if position < bytes.len() && bytes[position].is_ascii_digit() {
                valid = false;
                break;
            }
            octets += 1;
            end = position;
            if octets < 4 {
                if position < bytes.len() && bytes[position] == b'.' {
                    position += 1;
                } else {
                    valid = false;
                    break;
                }
            }
        }
        if valid && octets == 4 {
            let before_ok =
                index == 0 || !(bytes[index - 1].is_ascii_digit() || bytes[index - 1] == b'.');
            let after_ok =
                end >= bytes.len() || !(bytes[end].is_ascii_digit() || bytes[end] == b'.');
            if before_ok && after_ok {
                spans.push(PiiSpan {
                    start: index,
                    end,
                    kind: PiiKind::IpAddress,
                });
                if spans.len() >= cap {
                    return;
                }
                index = end;
                continue;
            }
        }
        index += 1;
    }
}

/// Scans for email addresses with a local part, an at sign, and a dotted
/// domain with no whitespace inside the span.
fn scan_email_address(bytes: &[u8], spans: &mut Vec<PiiSpan>, cap: usize) {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        let mut start = index;
        while start > 0 && is_local_byte(bytes[start - 1]) {
            start -= 1;
        }
        if start == index {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && (is_domain_byte(bytes[end]) || bytes[end] == b'.') {
            end += 1;
        }
        let domain = &bytes[index + 1..end];
        let has_dot = domain.contains(&b'.');
        let ends_clean = !domain.is_empty()
            && domain[0] != b'.'
            && domain[domain.len() - 1] != b'.'
            && !domain.windows(2).any(|pair| pair == b"..");
        if has_dot && ends_clean {
            spans.push(PiiSpan {
                start,
                end,
                kind: PiiKind::EmailAddress,
            });
            if spans.len() >= cap {
                return;
            }
            index = end;
            continue;
        }
        index += 1;
    }
}

/// Scans for a domain separator pattern of the form `DOMAIN\name`.
///
/// Candidates whose domain portion is `Users` or is preceded by a backslash
/// or a colon are treated as path segments rather than account names.
fn scan_windows_account_name(bytes: &[u8], spans: &mut Vec<PiiSpan>, cap: usize) {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }
        if index > 0 && bytes[index - 1] == b'\\' {
            index += 1;
            continue;
        }
        let mut domain_start = index;
        while domain_start > 0 && is_ident_byte(bytes[domain_start - 1]) {
            domain_start -= 1;
        }
        if domain_start == index {
            index += 1;
            continue;
        }
        if bytes[domain_start - 1] == b'\\' || bytes[domain_start - 1] == b':' {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && is_ident_byte(bytes[end]) {
            end += 1;
        }
        if end == index + 1 {
            index += 1;
            continue;
        }
        let domain = &bytes[domain_start..index];
        if domain.eq_ignore_ascii_case(b"users") {
            index += 1;
            continue;
        }
        spans.push(PiiSpan {
            start: domain_start,
            end,
            kind: PiiKind::WindowsAccountName,
        });
        if spans.len() >= cap {
            return;
        }
        index = end;
    }
}

/// Collects all detected PII spans in `text`, sorted by start offset with
/// overlaps resolved in favor of the earlier and longer span.
fn collect_spans(text: &str) -> Vec<PiiSpan> {
    let bytes = text.as_bytes();
    let cap = MAX_PII_FINDINGS;
    let mut spans = Vec::new();
    scan_user_profile_path(bytes, &mut spans, cap);
    scan_machine_name(bytes, &mut spans, cap);
    scan_environment_variable(bytes, &mut spans, cap);
    scan_ip_address(bytes, &mut spans, cap);
    scan_email_address(bytes, &mut spans, cap);
    scan_windows_account_name(bytes, &mut spans, cap);
    spans.sort_by(|left, right| left.start.cmp(&right.start).then(right.end.cmp(&left.end)));
    let mut merged: Vec<PiiSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = merged.last() {
            if span.start < last.end {
                continue;
            }
        }
        merged.push(span);
        if merged.len() >= cap {
            break;
        }
    }
    merged
}

/// Scans `text` for PII patterns and returns bounded findings.
///
/// `field` names the record field being scanned and is copied verbatim into
/// every returned finding so callers can attribute detections without
/// allocating. The returned vector is capped at `MAX_PII_FINDINGS`.
pub fn scan_for_pii(field: &'static str, text: &str) -> Vec<PiiFinding> {
    collect_spans(text)
        .into_iter()
        .map(|span| PiiFinding {
            kind: span.kind,
            field,
            offset: span.start,
        })
        .collect()
}

/// Replaces every detected PII span in `text` with `[redacted]` and returns
/// the sanitized string together with the findings.
///
/// Replacement is applied from the end of the string toward the start so the
/// byte offsets of earlier findings stay valid against the original input.
/// The surrounding text outside each span is preserved byte for byte.
pub fn sanitize_pii(field: &'static str, text: &str) -> (String, Vec<PiiFinding>) {
    let spans = collect_spans(text);
    if spans.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    let findings: Vec<PiiFinding> = spans
        .iter()
        .map(|span| PiiFinding {
            kind: span.kind,
            field,
            offset: span.start,
        })
        .collect();
    let mut sanitized = text.to_owned();
    for span in spans.iter().rev() {
        sanitized.replace_range(span.start..span.end, "[redacted]");
    }
    (sanitized, findings)
}
