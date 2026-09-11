//! Truthful, bounded, in-memory diagnostic records.
//!
//! The store is local to one process session. It does not create files, log to
//! disk, inspect machine or user state, or infer effective platform behavior.
//! Disk persistence is deliberately deferred.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tick_core::Status;

pub const DEFAULT_MAX_EVENTS: usize = 512;
const MAX_FIELD_LENGTH: usize = 160;

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
            "status={:?}, evidence={:?}",
            self.status, self.evidence
        )
    }
}

pub fn format_status(record: StatusRecord) -> String {
    record.to_string()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticEvent {
    pub sequence: u64,
    pub elapsed: Duration,
    pub name: String,
    pub details: String,
}

pub fn format_event(event: &DiagnosticEvent) -> String {
    format!(
        "#{:06} +{:>8}ms {}{}",
        event.sequence,
        event.elapsed.as_millis(),
        event.name,
        if event.details.is_empty() {
            String::new()
        } else {
            format!(" {}", event.details)
        }
    )
}

#[derive(Clone, Debug)]
struct StoreState {
    next_sequence: u64,
    events: Vec<DiagnosticEvent>,
    truncation_recorded: bool,
}

#[derive(Debug)]
struct StoreInner {
    started: Instant,
    maximum_events: usize,
    state: Mutex<StoreState>,
}

#[derive(Clone, Debug)]
pub struct DiagnosticStore {
    inner: Arc<StoreInner>,
}

impl DiagnosticStore {
    pub fn new(maximum_events: usize) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                started: Instant::now(),
                maximum_events: maximum_events.max(2),
                state: Mutex::new(StoreState {
                    next_sequence: 1,
                    events: Vec::new(),
                    truncation_recorded: false,
                }),
            }),
        }
    }

    pub fn record(&self, name: &str, details: impl AsRef<str>) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("diagnostic store mutex poisoned");
        if state.events.len() >= self.inner.maximum_events {
            let oldest_event = if state.truncation_recorded { 1 } else { 0 };
            state.events.remove(oldest_event);
            if !state.truncation_recorded {
                let sequence = state.next_sequence;
                let elapsed = self.inner.started.elapsed();
                let details = format!(
                    "retaining newest {} events",
                    self.inner.maximum_events.saturating_sub(1)
                );
                state.events.insert(
                    0,
                    DiagnosticEvent {
                        sequence,
                        elapsed,
                        name: "diagnostic.log_truncated".to_owned(),
                        details,
                    },
                );
                state.next_sequence = state.next_sequence.saturating_add(1);
                state.truncation_recorded = true;
            }
            if state.events.len() >= self.inner.maximum_events {
                state.events.remove(1);
            }
        }
        let event = DiagnosticEvent {
            sequence: state.next_sequence,
            elapsed: self.inner.started.elapsed(),
            name: sanitize(name),
            details: sanitize(details.as_ref()),
        };
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.events.push(event);
    }

    pub fn snapshot(&self) -> Vec<DiagnosticEvent> {
        self.inner
            .state
            .lock()
            .expect("diagnostic store mutex poisoned")
            .events
            .clone()
    }

    pub fn maximum_events(&self) -> usize {
        self.inner.maximum_events
    }
}

impl Default for DiagnosticStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_EVENTS)
    }
}

pub fn sanitize(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            _ if character.is_control() => '?',
            _ => character,
        })
        .collect::<String>();
    if sanitized.len() > MAX_FIELD_LENGTH {
        sanitized.truncate(MAX_FIELD_LENGTH);
        sanitized.push_str("...");
    }
    sanitized
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
            "status=Unsupported, evidence=Unsupported"
        );
    }

    #[test]
    fn event_format_is_structured_and_stable() {
        let event = DiagnosticEvent {
            sequence: 7,
            elapsed: Duration::from_millis(42),
            name: "tray.command".to_owned(),
            details: "command=start".to_owned(),
        };
        assert_eq!(
            format_event(&event),
            "#000007 +      42ms tray.command command=start"
        );
    }

    #[test]
    fn sequence_and_elapsed_order_are_preserved() {
        let store = DiagnosticStore::new(8);
        store.record("first", "");
        store.record("second", "");
        let events = store.snapshot();
        assert!(events[0].sequence < events[1].sequence);
        assert!(events[0].elapsed <= events[1].elapsed);
    }

    #[test]
    fn retention_keeps_newest_events_and_one_marker() {
        let store = DiagnosticStore::new(3);
        for index in 0..6 {
            store.record("event", index.to_string());
        }
        let events = store.snapshot();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].name, "diagnostic.log_truncated");
        assert_eq!(events[2].details, "5");
        assert!(events[0].sequence < events[1].sequence);
        assert!(events[1].sequence < events[2].sequence);
    }

    #[test]
    fn sanitization_removes_control_layout_and_bounds_fields() {
        let input = format!("secret\n{}", "x".repeat(300));
        let output = sanitize(&input);
        assert!(!output.contains('\n'));
        assert!(output.ends_with("..."));
        assert!(output.len() <= MAX_FIELD_LENGTH + 3);
    }
}
