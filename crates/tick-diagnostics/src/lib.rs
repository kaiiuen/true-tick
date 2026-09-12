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

pub const REPORT_COLUMNS: [&str; 11] = [
    "Row",
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

pub const MAX_TSV_ROWS: usize = HARD_MAX_EVENTS;
pub const MAX_TSV_BYTES: usize =
    REPORT_COLUMNS.len() * (MAX_FIELD_LENGTH + 3) * MAX_TSV_ROWS + MAX_TSV_ROWS * 2 + 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayLimitError {
    Empty,
    Negative,
    Zero,
    Malformed,
    OutOfRange,
    TooLong,
}

impl fmt::Display for DisplayLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("enter a positive row count"),
            Self::Negative => formatter.write_str("negative rows are not allowed"),
            Self::Zero => formatter.write_str("row count must be positive"),
            Self::Malformed => formatter.write_str("use a positive whole number"),
            Self::OutOfRange => write!(formatter, "row count must be at most {HARD_MAX_EVENTS}"),
            Self::TooLong => formatter.write_str("the row count is too long"),
        }
    }
}

pub fn parse_display_limit(input: &str) -> Result<usize, DisplayLimitError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(DisplayLimitError::Empty);
    }
    if input.len() > 64 {
        return Err(DisplayLimitError::TooLong);
    }
    if input.starts_with('-') {
        return Err(DisplayLimitError::Negative);
    }
    if !input.chars().all(|character| character.is_ascii_digit()) {
        return Err(DisplayLimitError::Malformed);
    }
    let limit = input
        .parse::<usize>()
        .map_err(|_| DisplayLimitError::Malformed)?;
    if limit == 0 {
        return Err(DisplayLimitError::Zero);
    }
    if limit > HARD_MAX_EVENTS {
        return Err(DisplayLimitError::OutOfRange);
    }
    Ok(limit)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowSelection {
    start: usize,
    end: usize,
}

pub fn latest_row_selection(display_limit: usize, retained_rows: usize) -> RowSelection {
    if retained_rows == 0 {
        return RowSelection::all(0);
    }
    let visible_rows = display_limit.clamp(1, HARD_MAX_EVENTS).min(retained_rows);
    RowSelection::new(
        retained_rows.saturating_sub(visible_rows).saturating_add(1),
        retained_rows,
    )
}

impl RowSelection {
    pub fn all(retained_rows: usize) -> Self {
        if retained_rows == 0 {
            Self { start: 0, end: 0 }
        } else {
            Self {
                start: 1,
                end: retained_rows,
            }
        }
    }

    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn start(self) -> usize {
        self.start
    }

    pub fn end(self) -> usize {
        self.end
    }

    pub fn row_count(self) -> usize {
        if self.start == 0 || self.end < self.start {
            0
        } else {
            self.end.saturating_sub(self.start).saturating_add(1)
        }
    }

    pub fn is_empty(self) -> bool {
        self.row_count() == 0
    }

    pub fn is_all(self, retained_rows: usize) -> bool {
        self == Self::all(retained_rows)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowSelectionError {
    Malformed,
    Negative,
    Zero,
    Reversed,
    OutOfRange { retained_rows: usize },
    TooLong,
}

impl fmt::Display for RowSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("use a row number or start-end"),
            Self::Negative => formatter.write_str("negative rows are not allowed"),
            Self::Zero => formatter.write_str("row numbers start at 1"),
            Self::Reversed => formatter.write_str("the range must be in ascending order"),
            Self::OutOfRange { retained_rows } => {
                write!(formatter, "row must be between 1 and {retained_rows}")
            }
            Self::TooLong => formatter.write_str("the range is too long"),
        }
    }
}

fn parse_row_number(value: &str) -> Result<usize, RowSelectionError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RowSelectionError::Malformed);
    }
    if value.starts_with('-') {
        return Err(RowSelectionError::Negative);
    }
    if value == "0" {
        return Err(RowSelectionError::Zero);
    }
    if !value.chars().all(|character| character.is_ascii_digit()) {
        return Err(RowSelectionError::Malformed);
    }
    let number = value
        .parse::<usize>()
        .map_err(|_| RowSelectionError::Malformed)?;
    if number == 0 {
        Err(RowSelectionError::Zero)
    } else {
        Ok(number)
    }
}

pub fn parse_row_selection(
    input: &str,
    retained_rows: usize,
) -> Result<RowSelection, RowSelectionError> {
    let input = input.trim();
    if input.len() > 64 {
        return Err(RowSelectionError::TooLong);
    }
    if input.is_empty() || input.eq_ignore_ascii_case("all") {
        return Ok(RowSelection::all(retained_rows));
    }
    if input.starts_with('-') {
        return Err(RowSelectionError::Negative);
    }
    let mut parts = input.split('-');
    let first = parts.next().ok_or(RowSelectionError::Malformed)?;
    let Some(second) = parts.next() else {
        let row = parse_row_number(first)?;
        if row > retained_rows {
            return Err(RowSelectionError::OutOfRange { retained_rows });
        }
        return Ok(RowSelection::new(row, row));
    };
    if parts.next().is_some() {
        return Err(RowSelectionError::Malformed);
    }
    let start = parse_row_number(first)?;
    let end = parse_row_number(second)?;
    if start > end {
        return Err(RowSelectionError::Reversed);
    }
    if start > retained_rows || end > retained_rows {
        return Err(RowSelectionError::OutOfRange { retained_rows });
    }
    Ok(RowSelection::new(start, end))
}

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

pub fn diagnostic_grid_row(row_position: usize, event: &DiagnosticEvent) -> DiagnosticGridRow {
    let details = if event.details.is_empty() {
        native_details(event.native)
    } else {
        format!("{} {}", event.details, native_details(event.native))
    };
    DiagnosticGridRow {
        cells: vec![
            row_position.to_string(),
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

pub fn diagnostic_grid_rows(
    events: &[DiagnosticEvent],
    selection: RowSelection,
) -> Vec<DiagnosticGridRow> {
    if selection.is_empty() {
        return Vec::new();
    }
    events
        .iter()
        .enumerate()
        .skip(selection.start().saturating_sub(1))
        .take(selection.row_count())
        .map(|(index, event)| diagnostic_grid_row(index.saturating_add(1), event))
        .collect()
}

pub fn selected_event_sequences(events: &[DiagnosticEvent], selection: RowSelection) -> Vec<u64> {
    if selection.is_empty() {
        return Vec::new();
    }
    events
        .iter()
        .skip(selection.start().saturating_sub(1))
        .take(selection.row_count())
        .map(|event| event.sequence)
        .collect()
}

pub fn selected_event_sequences_for_positions(
    events: &[DiagnosticEvent],
    visible_selection: RowSelection,
    positions: &[usize],
) -> Vec<u64> {
    let mut positions = positions.to_vec();
    positions.sort_unstable();
    positions.dedup();
    positions
        .into_iter()
        .filter_map(|position| {
            let row = visible_selection.start().saturating_add(position);
            (row > 0 && row <= visible_selection.end())
                .then(|| {
                    events
                        .get(row.saturating_sub(1))
                        .map(|event| event.sequence)
                })
                .flatten()
        })
        .collect()
}

pub fn selected_event_sequences_for_sequences(
    events: &[DiagnosticEvent],
    sequences: &[u64],
) -> Vec<u64> {
    events
        .iter()
        .filter(|event| sequences.contains(&event.sequence))
        .map(|event| event.sequence)
        .collect()
}

pub fn visible_positions_for_sequences(
    events: &[DiagnosticEvent],
    visible_selection: RowSelection,
    sequences: &[u64],
) -> Vec<usize> {
    if visible_selection.is_empty() {
        return Vec::new();
    }
    events
        .iter()
        .enumerate()
        .skip(visible_selection.start().saturating_sub(1))
        .take(visible_selection.row_count())
        .filter_map(|(index, event)| {
            sequences
                .contains(&event.sequence)
                .then_some(index.saturating_sub(visible_selection.start().saturating_sub(1)))
        })
        .collect()
}

pub fn row_selection_for_sequences(
    events: &[DiagnosticEvent],
    sequences: &[u64],
) -> Option<RowSelection> {
    if sequences.is_empty() {
        return None;
    }
    let positions = sequences
        .iter()
        .map(|sequence| {
            events
                .iter()
                .position(|event| event.sequence == *sequence)
                .map(|index| index.saturating_add(1))
        })
        .collect::<Option<Vec<_>>>()?;
    if positions
        .windows(2)
        .all(|window| window[1] == window[0].saturating_add(1))
    {
        Some(RowSelection::new(
            positions[0],
            *positions.last().expect("non-empty positions"),
        ))
    } else {
        None
    }
}

pub fn format_tsv(rows: &[DiagnosticGridRow]) -> String {
    let mut output = REPORT_COLUMNS.join("\t");
    output.push_str("\r\n");
    for row in rows.iter().take(MAX_TSV_ROWS) {
        for index in 0..REPORT_COLUMNS.len() {
            if index > 0 {
                output.push('\t');
            }
            let cell = row.cells.get(index).map_or("", String::as_str);
            output.push_str(&sanitize(cell));
        }
        output.push_str("\r\n");
    }
    output
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
    fn display_limit_parser_accepts_positive_counts_and_rejects_invalid_input() {
        assert_eq!(parse_display_limit("100"), Ok(100));
        assert_eq!(parse_display_limit(" 200 "), Ok(200));
        assert_eq!(parse_display_limit(""), Err(DisplayLimitError::Empty));
        assert_eq!(
            parse_display_limit("all"),
            Err(DisplayLimitError::Malformed)
        );
        assert_eq!(parse_display_limit("0"), Err(DisplayLimitError::Zero));
        assert_eq!(parse_display_limit("-1"), Err(DisplayLimitError::Negative));
        assert_eq!(
            parse_display_limit("12.5"),
            Err(DisplayLimitError::Malformed)
        );
        assert_eq!(
            parse_display_limit("513"),
            Err(DisplayLimitError::OutOfRange)
        );
    }

    #[test]
    fn latest_row_slice_keeps_newest_retained_rows_in_chronological_order() {
        assert_eq!(latest_row_selection(3, 10), RowSelection::new(8, 10));
        assert_eq!(latest_row_selection(100, 4), RowSelection::all(4));
        assert_eq!(latest_row_selection(3, 0), RowSelection::all(0));
        let events = (1..=10).map(test_event).collect::<Vec<_>>();
        let rows = diagnostic_grid_rows(&events, latest_row_selection(3, events.len()));
        assert_eq!(
            rows.iter()
                .map(|row| row.cells[0].as_str())
                .collect::<Vec<_>>(),
            ["8", "9", "10"]
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.cells[1].as_str())
                .collect::<Vec<_>>(),
            ["8", "9", "10"]
        );
    }

    #[test]
    fn row_selection_parser_accepts_empty_all_single_range_and_whitespace() {
        assert_eq!(parse_row_selection("", 4), Ok(RowSelection::new(1, 4)));
        assert_eq!(parse_row_selection(" all ", 4), Ok(RowSelection::new(1, 4)));
        assert_eq!(parse_row_selection(" 2 ", 4), Ok(RowSelection::new(2, 2)));
        assert_eq!(
            parse_row_selection(" 2 - 4 ", 4),
            Ok(RowSelection::new(2, 4))
        );
        assert_eq!(parse_row_selection("", 0), Ok(RowSelection::all(0)));
        assert_eq!(
            parse_row_selection("2", 4).expect("single row").row_count(),
            1
        );
    }

    fn test_event(sequence: u64) -> DiagnosticEvent {
        DiagnosticEvent {
            sequence,
            elapsed: Duration::from_millis(sequence),
            operation_id: sequence,
            parent_operation_id: None,
            correlation_id: sequence,
            phase: DiagnosticPhase::Observe,
            source: DiagnosticSource::Diagnostic,
            outcome: DiagnosticOutcome::Completed,
            native: NativeOutcome::default(),
            name: "test.event".to_owned(),
            details: String::new(),
        }
    }

    #[test]
    fn row_selection_parser_rejects_malformed_reversed_zero_negative_and_out_of_range() {
        assert_eq!(
            parse_row_selection("x", 4),
            Err(RowSelectionError::Malformed)
        );
        assert_eq!(
            parse_row_selection("2-x", 4),
            Err(RowSelectionError::Malformed)
        );
        assert_eq!(
            parse_row_selection("4-2", 4),
            Err(RowSelectionError::Reversed)
        );
        assert_eq!(parse_row_selection("0", 4), Err(RowSelectionError::Zero));
        assert_eq!(
            parse_row_selection("-1", 4),
            Err(RowSelectionError::Negative)
        );
        assert_eq!(
            parse_row_selection("5", 4),
            Err(RowSelectionError::OutOfRange { retained_rows: 4 })
        );
        assert_eq!(
            parse_row_selection("1-5", 4),
            Err(RowSelectionError::OutOfRange { retained_rows: 4 })
        );
    }

    #[test]
    fn retained_row_mapping_keeps_row_and_sequence_distinct_for_the_report() {
        let events = (401..=912).map(test_event).collect::<Vec<_>>();
        let selection = parse_row_selection("353-354", events.len()).expect("sample range");
        let rows = diagnostic_grid_rows(&events, selection);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cells[0], "353");
        assert_eq!(rows[0].cells[1], "753");
        assert_eq!(rows[1].cells[0], "354");
        assert_eq!(rows[1].cells[1], "754");
        assert_eq!(selected_event_sequences(&events, selection), [753, 754]);
        assert_eq!(
            row_selection_for_sequences(&events, &[753, 754]),
            Some(RowSelection::new(353, 354))
        );
    }

    #[test]
    fn multi_row_positions_map_in_chronological_order_even_when_drag_order_is_not_sorted() {
        let events = (10..=14).map(test_event).collect::<Vec<_>>();
        let visible = RowSelection::new(2, 5);
        assert_eq!(
            selected_event_sequences_for_positions(&events, visible, &[3, 0, 2]),
            [11, 13, 14]
        );
        assert_eq!(
            visible_positions_for_sequences(&events, visible, &[14, 12]),
            [1, 3]
        );
    }

    #[test]
    fn selection_sequences_are_retained_in_event_order() {
        let events = (10..=14).map(test_event).collect::<Vec<_>>();
        assert_eq!(
            selected_event_sequences_for_sequences(&events, &[14, 11, 13]),
            [11, 13, 14]
        );
    }

    #[test]
    fn truncation_keeps_report_rows_numbered_from_the_retained_snapshot() {
        let store = DiagnosticStore::new(4);
        for sequence in 1..=8 {
            store.record("event", sequence.to_string());
        }
        let events = store.snapshot();
        let rows = diagnostic_grid_rows(&events, RowSelection::all(events.len()));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].cells[0], "1");
        assert_eq!(rows[1].cells[0], "2");
        assert_eq!(rows[2].cells[0], "3");
        assert_eq!(rows[3].cells[0], "4");
        assert_eq!(events[0].name, "diagnostic.log_truncated");
        assert_ne!(rows[0].cells[0], rows[0].cells[1]);
    }

    #[test]
    fn sequence_identity_preserves_a_range_when_retained_rows_shift() {
        let old_events = (401..=912).map(test_event).collect::<Vec<_>>();
        let old_selection = RowSelection::new(353, 354);
        let sequences = selected_event_sequences(&old_events, old_selection);
        let mut new_events = old_events[1..].to_vec();
        new_events.push(test_event(913));
        assert_eq!(
            row_selection_for_sequences(&new_events, &sequences),
            Some(RowSelection::new(352, 353))
        );
        assert_eq!(
            row_selection_for_sequences(&new_events, &[753, 754, 912]),
            None
        );
    }

    #[test]
    fn refresh_updates_latest_display_slice_while_transfer_rows_keep_their_mapping() {
        let first = (1..=5).map(test_event).collect::<Vec<_>>();
        let second = (2..=6).map(test_event).collect::<Vec<_>>();
        let first_visible = diagnostic_grid_rows(&first, latest_row_selection(3, first.len()));
        let second_visible = diagnostic_grid_rows(&second, latest_row_selection(3, second.len()));
        let transfer = parse_row_selection("1-2", second.len()).expect("transfer range");
        assert_eq!(
            first_visible
                .iter()
                .map(|row| row.cells[1].as_str())
                .collect::<Vec<_>>(),
            ["3", "4", "5"]
        );
        assert_eq!(
            second_visible
                .iter()
                .map(|row| row.cells[1].as_str())
                .collect::<Vec<_>>(),
            ["4", "5", "6"]
        );
        assert_eq!(selected_event_sequences(&second, transfer), [2, 3]);
    }

    #[test]
    fn transfer_selection_is_independent_and_can_include_hidden_retained_rows() {
        let events = (1..=10).map(test_event).collect::<Vec<_>>();
        let display = latest_row_selection(3, events.len());
        let transfer = parse_row_selection("1-2", events.len()).expect("hidden transfer range");
        assert_eq!(display, RowSelection::new(8, 10));
        let rows = diagnostic_grid_rows(&events, transfer);
        assert_eq!(rows[0].cells[0], "1");
        assert_eq!(rows[0].cells[1], "1");
        assert_eq!(rows[1].cells[0], "2");
        assert_eq!(rows[1].cells[1], "2");
        let output = format_tsv(&rows);
        assert!(output.contains("1\t1\t"));
        assert!(output.contains("2\t2\t"));
        assert!(!output.contains("8\t8\t"));
    }

    #[test]
    fn selected_tsv_contains_row_and_sequence_for_only_the_selected_rows() {
        let events = (401..=912).map(test_event).collect::<Vec<_>>();
        let selection = RowSelection::new(353, 354);
        let output = format_tsv(&diagnostic_grid_rows(&events, selection));
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(
            lines[0],
            "Row\tSequence\tElapsed\tOperation\tParent\tCorrelation\tPhase\tSource\tOutcome\tEvent\tDetails"
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[1].split('\t').take(2).collect::<Vec<_>>(),
            ["353", "753"]
        );
        assert_eq!(
            lines[2].split('\t').take(2).collect::<Vec<_>>(),
            ["354", "754"]
        );
    }

    #[test]
    fn tsv_formatter_has_the_report_header_and_bounded_sanitized_cells() {
        let row = DiagnosticGridRow {
            cells: vec!["one\ttwo\nthree".to_owned(), "x".repeat(10_000)],
        };
        let output = format_tsv(&[row]);
        assert!(output.starts_with("Row\tSequence\tElapsed\tOperation\tParent\tCorrelation\tPhase\tSource\tOutcome\tEvent\tDetails\r\n"));
        assert!(output.contains("one two three"));
        assert_eq!(output.matches('\n').count(), 2);
        assert!(output.len() <= MAX_TSV_BYTES);
        assert_eq!(output.lines().count(), 2);
        assert_eq!(output.lines().nth(1).unwrap().split('\t').count(), 11);
    }

    #[test]
    fn report_columns_and_typed_grid_row_preserve_operation_lineage() {
        assert_eq!(REPORT_COLUMNS.len(), 11);
        assert_eq!(REPORT_COLUMNS[0], "Row");
        assert_eq!(REPORT_COLUMNS[1], "Sequence");
        assert_eq!(REPORT_COLUMNS[10], "Details");
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
        let row = diagnostic_grid_row(4, &event);
        assert_eq!(row.cells.len(), REPORT_COLUMNS.len());
        assert_eq!(row.cells[0], "4");
        assert_eq!(row.cells[1], "7");
        assert_eq!(row.cells[2], "+42ms");
        assert_eq!(row.cells[3], "11");
        assert_eq!(row.cells[4], "3");
        assert_eq!(row.cells[5], "2");
        assert_eq!(row.cells[6], "Release");
        assert_eq!(row.cells[7], "Handoff");
        assert_eq!(row.cells[8], "Unverified");
        assert_eq!(row.cells[9], "timer.release");
        assert_eq!(
            row.cells[10],
            "command=stop ntstatus=-7 win32_last_error=5 requested_hns=5000 selected_hns=5000 effective_hns=9966"
        );
    }

    #[test]
    fn diagnostic_store_snapshot_converts_to_typed_grid_row() {
        let store = DiagnosticStore::new(8);
        let context = store.begin_operation(DiagnosticSource::TrayCommand);
        store.record_with_context(
            DiagnosticRecord {
                context,
                phase: DiagnosticPhase::Render,
                source: DiagnosticSource::Diagnostic,
                outcome: DiagnosticOutcome::Completed,
                native: NativeOutcome {
                    win32_last_error: Some(5),
                    ..NativeOutcome::default()
                },
            },
            "diagnostic.refresh",
            "rows=1",
        );
        let event = store.snapshot().into_iter().next().expect("snapshot row");
        let row = diagnostic_grid_row(1, &event);
        assert_eq!(row.cells.len(), REPORT_COLUMNS.len());
        assert_eq!(row.cells[0], "1");
        assert_eq!(row.cells[3], context.operation_id.to_string());
        assert_eq!(row.cells[6], "Render");
        assert_eq!(row.cells[9], "diagnostic.refresh");
        assert!(row.cells[10].contains("rows=1"));
        assert!(row.cells[10].contains("win32_last_error=5"));
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
        let row = diagnostic_grid_row(1, &event);
        assert!(row.cells[10].len() <= MAX_FIELD_LENGTH + 3);
        assert!(row.cells[10].contains("ntstatus=-1"));
        assert!(row.cells[10].contains("win32_last_error=5"));
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
