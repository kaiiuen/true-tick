# tick-diagnostics lib architecture

Source: `crates/tick-diagnostics/src/lib.rs`

```mermaid
flowchart TD
    subgraph EM["Event model and labels"]
        EC["EventCategory, from_event_name, 6 categories"]
        PH["DiagnosticPhase, 12 values"]
        SO["DiagnosticSource, 14 values"]
        OC["DiagnosticOutcome, 8 values"]
        NC["NativeOutcome, ntstatus win32 requested selected effective hns"]
        CX["OperationContext, operation parent correlation"]
        SR["StatusRecord, status plus Evidence"]
        DR["DiagnosticRecord, context phase source outcome native"]
        EC --> DE["DiagnosticEvent, sequence elapsed context phase source outcome native name details prev_hash entry_hash"]
        PH --> DE
        SO --> DE
        OC --> DE
        NC --> DE
        CX --> DE
        SR --> DE
        DR --> DE
    end

    GEN["genesis_hash, SHA-256 of TrueTick-Genesis-v1"] --> CH["compute_entry_hash, prev_hash then sequence then nanos then name then details"]
    DE --> CH
    CH --> EH["entry_hash stored on each event"]
    EH --> VE["verify_event_chain over a snapshot"]
    VE --> V1{"recomputed entry_hash matches?"}
    V1 -- "No" --> E1["Err, entry hash mismatch"]
    V1 -- "Yes" --> GAP{"sequence equals previous plus one?"}
    GAP -- "No" --> GOK["Eviction gap, link not checked"]
    GAP -- "Yes" --> LNK{"previous entry_hash equals event prev_hash?"}
    LNK -- "No" --> E2["Err, previous hash link mismatch"]
    LNK -- "Yes" --> VOK["Ok"]
    GOK --> VOK

    STORE["DiagnosticStore::new, cap clamped to 2 through 512"] --> STATE["StoreState, next_sequence next_operation_id events truncation_recorded last_entry_hash coalesced_details"]
    STATE --> REC["record_with_context"]
    REC --> SAN["sanitize, CR LF tab to space, other control to question mark, truncate 160"]
    SAN --> PII{"privacy scan enabled?"}
    PII -- "Yes" --> RED["sanitize_pii, spans become redacted token, append pii_redacted count"]
    PII -- "No" --> COAL
    RED --> COAL{"name is diagnostic.layout.error?"}
    COAL -- "Yes" --> CF["coalesce_repeated_layout_event"]
    COAL -- "No" --> CAP
    CF -- "Repeat proven by provenance marker" --> RET["Return, no new event appended"]
    CF -- "Not a repeat" --> CAP
    CAP{"events at or above cap?"}
    CAP -- "No" --> APP["compute entry_hash then push event"]
    CAP -- "Yes" --> EVICT["evict oldest, insert diagnostic.log_truncated marker at index 0 once, then evict index 1"]
    EVICT --> APP

    DO["derive_outcome, name plus details lowercased, whole word tokens"] --> T1{"timedout or timeout?"}
    T1 -- "Yes" --> OTO["TimedOut"]
    T1 -- "No" --> T2{"suppressed?"}
    T2 -- "Yes" --> OSP["Suppressed"]
    T2 -- "No" --> T3{"contains cancel?"}
    T3 -- "Yes" --> OCA["Cancelled"]
    T3 -- "No" --> T4{"unverified or uncertain?"}
    T4 -- "Yes" --> OUV["Unverified"]
    T4 -- "No" --> T5{"degraded?"}
    T5 -- "Yes" --> ODG["Degraded"]
    T5 -- "No" --> T6{"error token present and not error=none errors=none errors.cleared error=0 errors=0?"}
    T6 -- "Yes" --> OFA["Failed"]
    T6 -- "No" --> T7{"started or pending?"}
    T7 -- "Yes" --> OIP["InProgress"]
    T7 -- "No" --> OCO["Completed"]

    SEL["parse_row_selection, empty or all, N, or N-M, max 64 chars"] --> RS["RowSelection, start end, row_count"]
    DL["parse_display_limit, positive count capped at 512"] --> LRS["latest_row_selection, newest retained rows"]
    LRS --> RS
    RS --> GRS["diagnostic_grid_rows, skip then take"]
    RS --> SEQ["selected_event_sequences, selection to sequences"]
    SEQ --> RSEQ["selected_event_sequences_for_sequences"]
    GRS --> GR["diagnostic_grid_row, 11 cells, elapsed as plus ms, native details, truncate 160"]
    GR --> FE["format_event, bounded at 1024 bytes"]
    GRS --> TSV["format_tsv, tab header, CRLF, sanitized cells, cap 512 rows"]
    GRS --> CSV["format_csv, comma header, csv_field quoting and formula neutralization, cap 512 rows"]
    COL["REPORT_COLUMNS, 11 header labels Row through Details"] --> TSV
    COL --> CSV
```

## Notes

* The store is in memory only for a single process session, it never writes files, logs to disk, or inspects machine and user state.
* `DEFAULT_MAX_EVENTS` and `HARD_MAX_EVENTS` are both 512 and `DiagnosticStore::new` clamps the requested cap into the range 2 to 512.
* `MAX_FIELD_LENGTH` is 160 bytes and `MAX_RENDERED_EVENT_BYTES` is 1024 bytes, both truncation paths stop on a valid UTF-8 boundary and append an ellipsis.
* `genesis_hash` is the SHA-256 digest of the fixed seed `TrueTick-Genesis-v1` and seeds `last_entry_hash` at store construction.
* `compute_entry_hash` hashes prev_hash, little endian sequence, little endian elapsed nanoseconds, name, and details in that fixed order.
* `verify_event_chain` recomputes every entry hash first, then checks the previous hash link only when the retained sequence numbers are adjacent, so a gap marks an expected ring buffer eviction boundary instead of tampering.
* Truncation is detected by `snapshot_is_truncated`, which checks whether the first retained event is named `diagnostic.log_truncated`, and the marker is inserted at index 0 with source Diagnostic and outcome Completed.
* `derive_outcome` uses whole word tokens so informational text such as `errors.cleared` and `error=none` stays Completed, while `cancel` is a plain substring check and call sites that know the real result use `record_with_outcome`.
* `sanitize` maps CR, LF, and tab to a space, other control characters to a question mark, and then bounds the field to 160 bytes.
* Sanitized details run through `privacy::sanitize_pii`, each detected span is replaced with the fixed redacted token, and the finding count is appended as `pii_redacted`.
* Coalescing applies only to `diagnostic.layout.error`, rewrites a `repeat_count` suffix only when the existing string matches the `coalesced_details` provenance marker, and otherwise preserves the suffix as a fresh event.
* `REPORT_COLUMNS` holds 11 columns, CSV fields are quoted and formula injection prefixes are neutralized, and both TSV and CSV output are capped at 512 rows with byte budgets derived from the column count and field length.
