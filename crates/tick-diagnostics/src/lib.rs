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
pub const HARD_MAX_EVENTS: usize = 512;
pub const MAX_FIELD_LENGTH: usize = 160;
pub const MAX_RENDERED_EVENT_BYTES: usize = 1_024;

pub fn truncate_utf8(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let end = value
        .char_indices()
        .take_while(|(index, character)| {
            index.saturating_add(character.len_utf8()) <= maximum_bytes
        })
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    let mut truncated = value[..end].to_owned();
    truncated.push_str("...");
    truncated
}

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
            "status={}, evidence={}",
            status_label(self.status),
            evidence_label(self.evidence)
        )
    }
}

pub fn format_status(record: StatusRecord) -> String {
    record.to_string()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticPhase {
    Begin,
    Observe,
    Decide,
    Acquire,
    Release,
    Verify,
    Handoff,
    Timer,
    Render,
    Persist,
    Shutdown,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSource {
    TrayCommand,
    PowerEvent,
    Startup,
    Pause,
    Resume,
    Handoff,
    Shutdown,
    Policy,
    Ownership,
    Platform,
    Native,
    Timer,
    Diagnostic,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticOutcome {
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Suppressed,
    TimedOut,
    Unverified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeOutcome {
    pub ntstatus: Option<i32>,
    pub win32_last_error: Option<u32>,
    pub requested_hns: Option<u64>,
    pub selected_hns: Option<u64>,
    pub effective_hns: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub operation_id: u64,
    pub parent_operation_id: Option<u64>,
    pub correlation_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub context: OperationContext,
    pub phase: DiagnosticPhase,
    pub source: DiagnosticSource,
    pub outcome: DiagnosticOutcome,
    pub native: NativeOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticEvent {
    pub sequence: u64,
    pub elapsed: Duration,
    pub operation_id: u64,
    pub parent_operation_id: Option<u64>,
    pub correlation_id: u64,
    pub phase: DiagnosticPhase,
    pub source: DiagnosticSource,
    pub outcome: DiagnosticOutcome,
    pub native: NativeOutcome,
    pub name: String,
    pub details: String,
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Released => "Released",
        Status::Active => "Active",
        Status::Requested => "Requested",
        Status::Warning => "Warning",
        Status::Blocked => "Blocked",
        Status::Error => "Error",
        Status::Unknown => "Unknown",
        Status::Unsupported => "Unsupported",
    }
}

fn evidence_label(evidence: Evidence) -> &'static str {
    match evidence {
        Evidence::NotCollected => "NotCollected",
        Evidence::Unsupported => "Unsupported",
        Evidence::Inconclusive => "Inconclusive",
    }
}

macro_rules! stable_labels {
    ($type:ty, { $($variant:path => $label:literal),+ $(,)? }) => {
        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                let label = match self {
                    $($variant => $label,)+
                };
                formatter.write_str(label)
            }
        }
    };
}

stable_labels!(DiagnosticPhase, {
    DiagnosticPhase::Begin => "Begin",
    DiagnosticPhase::Observe => "Observe",
    DiagnosticPhase::Decide => "Decide",
    DiagnosticPhase::Acquire => "Acquire",
    DiagnosticPhase::Release => "Release",
    DiagnosticPhase::Verify => "Verify",
    DiagnosticPhase::Handoff => "Handoff",
    DiagnosticPhase::Timer => "Timer",
    DiagnosticPhase::Render => "Render",
    DiagnosticPhase::Persist => "Persist",
    DiagnosticPhase::Shutdown => "Shutdown",
    DiagnosticPhase::Complete => "Complete",
});

stable_labels!(DiagnosticSource, {
    DiagnosticSource::TrayCommand => "TrayCommand",
    DiagnosticSource::PowerEvent => "PowerEvent",
    DiagnosticSource::Startup => "Startup",
    DiagnosticSource::Pause => "Pause",
    DiagnosticSource::Resume => "Resume",
    DiagnosticSource::Handoff => "Handoff",
    DiagnosticSource::Shutdown => "Shutdown",
    DiagnosticSource::Policy => "Policy",
    DiagnosticSource::Ownership => "Ownership",
    DiagnosticSource::Platform => "Platform",
    DiagnosticSource::Native => "Native",
    DiagnosticSource::Timer => "Timer",
    DiagnosticSource::Diagnostic => "Diagnostic",
    DiagnosticSource::Internal => "Internal",
});

stable_labels!(DiagnosticOutcome, {
    DiagnosticOutcome::InProgress => "InProgress",
    DiagnosticOutcome::Completed => "Completed",
    DiagnosticOutcome::Failed => "Failed",
    DiagnosticOutcome::Cancelled => "Cancelled",
    DiagnosticOutcome::Suppressed => "Suppressed",
    DiagnosticOutcome::TimedOut => "TimedOut",
    DiagnosticOutcome::Unverified => "Unverified",
});

pub const REPORT_COLUMNS: [&str; 10] = [
    "Sequence",
    "Elapsed",
    "Operation",
    "Parent",
    "Correlation",
    "Phase",
    "Source",
    "Outcome",
    "Event",
    "Details",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticGridRow {
    pub cells: Vec<String>,
}

fn optional_number<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

fn native_details(native: NativeOutcome) -> String {
    format!(
        "ntstatus={} win32_last_error={} requested_hns={} selected_hns={} effective_hns={}",
        optional_number(native.ntstatus),
        optional_number(native.win32_last_error),
        optional_number(native.requested_hns),
        optional_number(native.selected_hns),
        optional_number(native.effective_hns),
    )
}

pub fn diagnostic_grid_row(event: &DiagnosticEvent) -> DiagnosticGridRow {
    let details = if event.details.is_empty() {
        native_details(event.native)
    } else {
        format!("{} {}", event.details, native_details(event.native))
    };
    DiagnosticGridRow {
        cells: vec![
            event.sequence.to_string(),
            format!("+{}ms", event.elapsed.as_millis()),
            event.operation_id.to_string(),
            optional_number(event.parent_operation_id),
            event.correlation_id.to_string(),
            event.phase.to_string(),
            event.source.to_string(),
            event.outcome.to_string(),
            event.name.clone(),
            truncate_utf8(&details, MAX_FIELD_LENGTH),
        ],
    }
}

pub fn format_event(event: &DiagnosticEvent) -> String {
    let rendered = format!(
        "#{:06} +{:>8}ms op={} parent={} corr={} phase={} source={} outcome={} native_ntstatus={} native_win32={} requested_hns={} selected_hns={} effective_hns={} {}{}",
        event.sequence,
        event.elapsed.as_millis(),
        event.operation_id,
        event
            .parent_operation_id
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        event.correlation_id,
        event.phase,
        event.source,
        event.outcome,
        event.native
            .ntstatus
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        event
            .native
            .win32_last_error
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        event
            .native
            .requested_hns
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        event
            .native
            .selected_hns
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        event
            .native
            .effective_hns
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        event.name,
        if event.details.is_empty() {
            String::new()
        } else {
            format!(" {}", event.details)
        }
    );
    truncate_utf8(&rendered, MAX_RENDERED_EVENT_BYTES)
}

#[derive(Clone, Debug)]
struct StoreState {
    next_sequence: u64,
    next_operation_id: u64,
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
                maximum_events: maximum_events.clamp(2, HARD_MAX_EVENTS),
                state: Mutex::new(StoreState {
                    next_sequence: 1,
                    next_operation_id: 1,
                    events: Vec::new(),
                    truncation_recorded: false,
                }),
            }),
        }
    }

    pub fn begin_operation(&self, source: DiagnosticSource) -> OperationContext {
        self.allocate_operation(None, source)
    }

    pub fn child_operation(
        &self,
        parent: OperationContext,
        source: DiagnosticSource,
    ) -> OperationContext {
        self.allocate_operation(Some(parent), source)
    }

    fn allocate_operation(
        &self,
        parent: Option<OperationContext>,
        _source: DiagnosticSource,
    ) -> OperationContext {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("diagnostic store mutex poisoned");
        let operation_id = state.next_operation_id;
        state.next_operation_id = state.next_operation_id.saturating_add(1);
        OperationContext {
            operation_id,
            parent_operation_id: parent.map(|value| value.operation_id),
            correlation_id: parent.map_or(operation_id, |value| value.correlation_id),
        }
    }

    pub fn record(&self, name: &str, details: impl AsRef<str>) {
        let context = self.begin_operation(DiagnosticSource::Internal);
        self.record_with_context(
            DiagnosticRecord {
                context,
                phase: DiagnosticPhase::Complete,
                source: DiagnosticSource::Internal,
                outcome: DiagnosticOutcome::Completed,
                native: NativeOutcome::default(),
            },
            name,
            details,
        );
    }

    pub fn record_with_context(
        &self,
        record: DiagnosticRecord,
        name: &str,
        details: impl AsRef<str>,
    ) {
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
                        operation_id: record.context.operation_id,
                        parent_operation_id: record.context.parent_operation_id,
                        correlation_id: record.context.correlation_id,
                        phase: DiagnosticPhase::Complete,
                        source: DiagnosticSource::Diagnostic,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
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
            operation_id: record.context.operation_id,
            parent_operation_id: record.context.parent_operation_id,
            correlation_id: record.context.correlation_id,
            phase: record.phase,
            source: record.source,
            outcome: record.outcome,
            native: record.native,
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
    let sanitized = value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            _ if character.is_control() => '?',
            _ => character,
        })
        .collect::<String>();
    truncate_utf8(&sanitized, MAX_FIELD_LENGTH)
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
    fn report_columns_and_typed_grid_row_preserve_operation_lineage() {
        assert_eq!(REPORT_COLUMNS.len(), 10);
        assert_eq!(REPORT_COLUMNS[0], "Sequence");
        assert_eq!(REPORT_COLUMNS[9], "Details");
        let event = DiagnosticEvent {
            sequence: 7,
            elapsed: Duration::from_millis(42),
            operation_id: 11,
            parent_operation_id: Some(3),
            correlation_id: 2,
            phase: DiagnosticPhase::Release,
            source: DiagnosticSource::Handoff,
            outcome: DiagnosticOutcome::Unverified,
            native: NativeOutcome {
                ntstatus: Some(-7),
                win32_last_error: Some(5),
                requested_hns: Some(5_000),
                selected_hns: Some(5_000),
                effective_hns: Some(9_966),
            },
            name: "timer.release".to_owned(),
            details: "command=stop".to_owned(),
        };
        let row = diagnostic_grid_row(&event);
        assert_eq!(row.cells.len(), REPORT_COLUMNS.len());
        assert_eq!(row.cells[0], "7");
        assert_eq!(row.cells[1], "+42ms");
        assert_eq!(row.cells[2], "11");
        assert_eq!(row.cells[3], "3");
        assert_eq!(row.cells[4], "2");
        assert_eq!(row.cells[5], "Release");
        assert_eq!(row.cells[6], "Handoff");
        assert_eq!(row.cells[7], "Unverified");
        assert!(row.cells[9].contains("ntstatus=-7"));
        assert!(row.cells[9].contains("win32_last_error=5"));
        assert!(row.cells[9].contains("requested_hns=5000"));
    }

    #[test]
    fn grid_details_remain_bounded_and_native_error_types_stay_distinct() {
        let event = DiagnosticEvent {
            sequence: 1,
            elapsed: Duration::ZERO,
            operation_id: 1,
            parent_operation_id: None,
            correlation_id: 1,
            phase: DiagnosticPhase::Observe,
            source: DiagnosticSource::Native,
            outcome: DiagnosticOutcome::Failed,
            native: NativeOutcome {
                ntstatus: Some(-1),
                win32_last_error: Some(5),
                ..NativeOutcome::default()
            },
            name: "query".to_owned(),
            details: "x".to_owned(),
        };
        let row = diagnostic_grid_row(&event);
        assert!(row.cells[9].len() <= MAX_FIELD_LENGTH + 3);
        assert!(row.cells[9].contains("ntstatus=-1"));
        assert!(row.cells[9].contains("win32_last_error=5"));
    }

    #[test]
    fn event_format_contains_bounded_chain_of_custody_fields() {
        let event = DiagnosticEvent {
            sequence: 7,
            elapsed: Duration::from_millis(42),
            operation_id: 11,
            parent_operation_id: Some(3),
            correlation_id: 2,
            phase: DiagnosticPhase::Release,
            source: DiagnosticSource::Handoff,
            outcome: DiagnosticOutcome::Unverified,
            native: NativeOutcome {
                ntstatus: Some(-7),
                win32_last_error: Some(5),
                requested_hns: Some(5_000),
                selected_hns: Some(5_000),
                effective_hns: Some(9_966),
            },
            name: "timer.release".to_owned(),
            details: "command=stop".to_owned(),
        };
        let rendered = format_event(&event);
        assert!(rendered.contains("op=11"));
        assert!(rendered.contains("parent=3"));
        assert!(rendered.contains("corr=2"));
        assert!(rendered.contains("ntstatus=-7"));
        assert!(rendered.contains("native_win32=5"));
        assert!(rendered.len() <= MAX_RENDERED_EVENT_BYTES + 3);
    }

    #[test]
    fn operation_contexts_are_local_parented_and_correlated_without_registry() {
        let store = DiagnosticStore::new(8);
        let root = store.begin_operation(DiagnosticSource::TrayCommand);
        let child = store.child_operation(root, DiagnosticSource::Native);
        assert_ne!(root.operation_id, child.operation_id);
        assert_eq!(child.parent_operation_id, Some(root.operation_id));
        assert_eq!(child.correlation_id, root.correlation_id);
        store.record_with_context(
            DiagnosticRecord {
                context: child,
                phase: DiagnosticPhase::Release,
                source: DiagnosticSource::Native,
                outcome: DiagnosticOutcome::Failed,
                native: NativeOutcome {
                    ntstatus: Some(-1),
                    ..NativeOutcome::default()
                },
            },
            "native.release",
            "bounded",
        );
        assert_eq!(store.snapshot()[0].operation_id, child.operation_id);
    }

    #[test]
    fn duration_schedule_chain_retains_deadline_timing_and_policy_fields() {
        let store = DiagnosticStore::new(16);
        let root = store.begin_operation(DiagnosticSource::TrayCommand);
        let deadline = store.child_operation(root, DiagnosticSource::Timer);
        store.record_with_context(
            DiagnosticRecord {
                context: deadline,
                phase: DiagnosticPhase::Timer,
                source: DiagnosticSource::Timer,
                outcome: DiagnosticOutcome::InProgress,
                native: NativeOutcome::default(),
            },
            "duration.schedule",
            "policy=deferred action=start duration_minutes=5 generation=4 deadline_monotonic_ms=300000 remaining_ms=300000 timing_snapshot=valid=true requested_hns=unknown selected_hns=unknown effective_hns=unknown raw_status=unknown",
        );
        let event = &store.snapshot()[0];
        assert_eq!(event.operation_id, deadline.operation_id);
        assert_eq!(event.parent_operation_id, Some(root.operation_id));
        assert_eq!(event.correlation_id, root.correlation_id);
        assert!(event.details.contains("generation=4"));
        assert!(event.details.contains("remaining_ms=300000"));
        assert!(event.details.contains("policy=deferred"));
    }

    #[test]
    fn event_format_is_bounded_for_large_fields() {
        let store = DiagnosticStore::new(8);
        store.record("event", "x".repeat(10_000));
        assert!(format_event(&store.snapshot()[0]).len() <= MAX_RENDERED_EVENT_BYTES + 3);
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
    fn retention_clamps_to_the_hard_maximum() {
        assert_eq!(
            DiagnosticStore::new(usize::MAX).maximum_events(),
            HARD_MAX_EVENTS
        );
        assert_eq!(DiagnosticStore::new(0).maximum_events(), 2);
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

    #[test]
    fn truncation_stops_at_a_valid_unicode_boundary() {
        let input = "aé界🙂z";
        let output = truncate_utf8(input, 4);
        assert_eq!(output, "aé...");
        assert!(std::str::from_utf8(output.as_bytes()).is_ok());
    }

    #[test]
    fn invalid_utf8_is_replaced_before_sanitization() {
        let input = String::from_utf8_lossy(b"ok\xff\xfe");
        let output = sanitize(&input);
        assert_eq!(output, "ok��");
    }

    #[test]
    fn huge_ascii_is_bounded_without_panicking() {
        let output = sanitize(&"x".repeat(10_000));
        assert!(output.ends_with("..."));
        assert!(output.len() <= MAX_FIELD_LENGTH + 3);
    }
}
