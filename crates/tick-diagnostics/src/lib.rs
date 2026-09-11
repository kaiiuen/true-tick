//! Truthful, in-memory diagnostic records.
//!
//! Formatting is local and pure. This crate does not create files, log to disk,
//! inspect machine/user state, or infer effective platform behavior.

use std::fmt;
use tick_core::Status;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Evidence {
    NotCollected,
    Unsupported,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusRecord {
    pub status: Status,
    pub evidence: Evidence,
}

impl fmt::Display for StatusRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "status={:?}; evidence={:?}",
            self.status, self.evidence
        )
    }
}

pub fn format_status(record: StatusRecord) -> String {
    record.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_is_not_formatted_as_success() {
        let record = StatusRecord {
            status: Status::Unsupported,
            evidence: Evidence::Unsupported,
        };
        assert_eq!(
            format_status(record),
            "status=Unsupported; evidence=Unsupported"
        );
    }
}
