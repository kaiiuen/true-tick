use std::time::Duration;
use tick_diagnostics::{
    compute_entry_hash, diagnostic_grid_rows, format_csv, format_event, format_tsv, genesis_hash,
    sanitize, verify_event_chain, DiagnosticEvent, DiagnosticOutcome, DiagnosticPhase,
    DiagnosticSource, DiagnosticStore, NativeOutcome, RowSelection,
};

fn create_sample_events() -> Vec<DiagnosticEvent> {
    let store = DiagnosticStore::new(16);
    store.record("system.startup", "boot sequence started");
    store.record("timer.request", "requested_hns=5000");
    store.record("power.transition", "source=battery");
    store.record("timer.release", "released to baseline");
    store.snapshot()
}

#[test]
fn tamper_single_byte_in_event_name_fails_at_exact_index() {
    let events = create_sample_events();
    assert!(verify_event_chain(&events).is_ok());

    for target_idx in 0..events.len() {
        let mut tampered = events.clone();
        let mut bytes = tampered[target_idx].name.clone().into_bytes();
        bytes[0] ^= 0x01;
        tampered[target_idx].name = String::from_utf8(bytes).unwrap();

        assert_eq!(
            verify_event_chain(&tampered),
            Err((target_idx, "entry hash mismatch"))
        );
    }
}

#[test]
fn tamper_details_string_fails_at_exact_index() {
    let events = create_sample_events();
    assert!(verify_event_chain(&events).is_ok());

    for target_idx in 0..events.len() {
        let mut tampered = events.clone();
        tampered[target_idx].details.push_str(" corrupted");

        assert_eq!(
            verify_event_chain(&tampered),
            Err((target_idx, "entry hash mismatch"))
        );
    }
}

#[test]
fn tamper_sequence_or_timestamp_fails_at_exact_index() {
    let events = create_sample_events();
    assert!(verify_event_chain(&events).is_ok());

    for target_idx in 0..events.len() {
        let mut tampered_seq = events.clone();
        tampered_seq[target_idx].sequence += 100;
        assert_eq!(
            verify_event_chain(&tampered_seq),
            Err((target_idx, "entry hash mismatch"))
        );

        let mut tampered_time = events.clone();
        tampered_time[target_idx].elapsed += Duration::from_nanos(1);
        assert_eq!(
            verify_event_chain(&tampered_time),
            Err((target_idx, "entry hash mismatch"))
        );
    }
}

#[test]
fn corrupted_previous_hash_breaks_chain_linkage() {
    let events = create_sample_events();
    assert!(verify_event_chain(&events).is_ok());

    let mut tampered = events.clone();
    tampered[2].prev_hash[0] ^= 0xFF;
    assert_eq!(
        verify_event_chain(&tampered),
        Err((2, "entry hash mismatch"))
    );

    let recomputed_hash = compute_entry_hash(
        tampered[2].prev_hash,
        tampered[2].sequence,
        tampered[2].elapsed.as_nanos(),
        &tampered[2].name,
        &tampered[2].details,
    );
    tampered[2].entry_hash = recomputed_hash;

    assert_eq!(
        verify_event_chain(&tampered),
        Err((2, "previous hash link mismatch"))
    );
}

#[test]
fn corrupted_genesis_seed_fails_first_link() {
    let events = create_sample_events();
    assert_eq!(events[0].prev_hash, genesis_hash());

    let mut tampered = events.clone();
    tampered[0].prev_hash = [0u8; 32];
    assert_eq!(
        verify_event_chain(&tampered),
        Err((0, "entry hash mismatch"))
    );
}

#[test]
fn persistence_failure_resilience_empty_or_corrupted_snapshot() {
    let empty_events: Vec<DiagnosticEvent> = Vec::new();
    assert!(verify_event_chain(&empty_events).is_ok());
    let empty_rows = diagnostic_grid_rows(&empty_events, RowSelection::all(empty_events.len()));
    assert!(format_tsv(&empty_rows).starts_with("Row\tSequence\tElapsed"));
    assert!(format_csv(&empty_rows).starts_with("Row,Sequence,Elapsed"));

    let store = DiagnosticStore::new(10);
    store.record("startup.init", "clean");
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 1);

    let rows = diagnostic_grid_rows(&snapshot, RowSelection::all(snapshot.len()));
    let tsv_output = format_tsv(&rows);
    assert!(tsv_output.contains("startup.init"));
    assert!(tsv_output.contains("clean"));

    let formatted_event = format_event(&snapshot[0]);
    assert!(formatted_event.contains("startup.init"));

    let synthetic_corrupted_event = DiagnosticEvent {
        sequence: 1,
        elapsed: Duration::from_millis(100),
        operation_id: 1,
        parent_operation_id: None,
        correlation_id: 1,
        phase: DiagnosticPhase::Observe,
        source: DiagnosticSource::Startup,
        outcome: DiagnosticOutcome::Completed,
        native: NativeOutcome {
            ntstatus: None,
            win32_last_error: None,
            requested_hns: None,
            selected_hns: None,
            effective_hns: None,
        },
        name: "test\tcorrupted\nname".to_string(),
        details: "payload\r\nwith\0bad_chars".to_string(),
        prev_hash: [0xAA; 32],
        entry_hash: [0xBB; 32],
    };

    let single_corrupted = vec![synthetic_corrupted_event];
    assert_eq!(
        verify_event_chain(&single_corrupted),
        Err((0, "entry hash mismatch"))
    );

    let sanitized_details = sanitize(&single_corrupted[0].details);
    assert!(!sanitized_details.contains('\0'));
    assert!(!sanitized_details.contains('\r'));
    assert!(!sanitized_details.contains('\n'));

    let corrupted_rows =
        diagnostic_grid_rows(&single_corrupted, RowSelection::all(single_corrupted.len()));
    let tsv_rendered = format_tsv(&corrupted_rows);
    assert!(!tsv_rendered.contains('\0'));
    assert!(!tsv_rendered.contains("\r\nwith"));
}
