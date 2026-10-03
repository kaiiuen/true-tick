# Diagnostic Copy and Export Transfer in diagnostic_transfer.rs

Source path: `true-tick/apps/true-tick/src/ui/diagnostic_transfer.rs`

```mermaid
flowchart TD
    Q["search_matches query name details"] --> TRIM["trim query"]
    TRIM --> EMPTY{"empty after trim"}
    EMPTY -- "yes" --> MATCHALL["return true for every row"]
    EMPTY -- "no" --> LOWER["lowercase query name and details"]
    LOWER --> CONTAINS{"name or details contains query"}
    CONTAINS -- "yes" --> MATCH["return true"]
    CONTAINS -- "no" --> NOMATCH["return false"]
    VIS["diagnostic_filtered_visible_events"] --> FILT["diagnostic_filtered_events by category and search"]
    FILT --> SEL["diagnostic_visible_selection of retained rows"]
    SEL --> ISEMPTY{"selection empty"}
    ISEMPTY -- "yes" --> VIANONE["return empty Vec"]
    ISEMPTY -- "no" --> SLICE["skip start minus 1 then take row_count"]
    CLIP["copy_tsv_to_clipboard"] --> OPEN{"OpenClipboard"}
    OPEN -- "fail" --> CLIPERR["NativeFailure stage OpenClipboard"]
    OPEN -- "ok" --> EMPTYC{"EmptyClipboard"}
    EMPTYC -- "fail" --> EMPTYERR["close clipboard then failure EmptyClipboard"]
    EMPTYC -- "ok" --> UTF16["encode UTF-16 with NUL terminator"]
    UTF16 --> ALLOC{"GlobalAlloc GMEM_MOVEABLE"}
    ALLOC -- "null" --> ALLOCERR["close clipboard then failure GlobalAlloc"]
    ALLOC -- "ok" --> LOCK{"GlobalLock"}
    LOCK -- "null" --> LOCKERR["free then close then failure GlobalLock"]
    LOCK -- "ok" --> COPY["copy bytes into locked memory"]
    COPY --> SET{"SetClipboardData CF_UNICODETEXT"}
    SET -- "null" --> SETERR["free then close then failure SetClipboardData"]
    SET -- "ok" --> CLOSE{"CloseClipboard"}
    CLOSE -- "fail" --> CLOSEERR["NativeFailure stage CloseClipboard"]
    CLOSE -- "ok" --> CLIPOK["return Ok"]
    CHOOSE["choose_export_path"] --> DIALOG{"GetSaveFileNameW result"}
    DIALOG -- "nonzero" --> PATH["read UTF-16 buffer to PathBuf"]
    DIALOG -- "zero and extended error zero" --> CANCEL["return Ok None"]
    DIALOG -- "zero with extended error" --> DLGERR["NativeFailure stage GetSaveFileNameW"]
    WRITE["write_export_tsv"] --> TMP["write bytes to path plus tmp suffix"]
    TMP --> TWOK{"write ok"}
    TWOK -- "no" --> TMPERR["remove tmp then failure write_temporary"]
    TWOK -- "yes" --> MOVE{"MoveFileExW replace and write through"}
    MOVE -- "fail" --> MOVERR["remove tmp then failure MoveFileExW"]
    MOVE -- "ok" --> WROK["return Ok"]
```

## Notes

- `search_matches` treats a missing or blank query as match all, otherwise lowercases the trimmed query and checks it against both the event name and the details.
- `diagnostic_filtered_visible_events` composes `diagnostic_filtered_events` with `diagnostic_visible_selection`, so an empty selection yields an empty vector instead of every retained row.
- The slice in `diagnostic_filtered_visible_events` uses `selection.start().saturating_sub(1)` and `take(selection.row_count())`, which converts the one based `RowSelection` to a zero based iterator offset.
- `copy_tsv_to_clipboard` allocates a moveable global block and transfers ownership to the clipboard only when `SetClipboardData` succeeds, freeing the block on every earlier failure and closing the clipboard on every path.
- `choose_export_path` maps a nonzero `GetSaveFileNameW` result to a path, a zero result with zero extended error to cancel, and a zero result with a nonzero extended error to a failure.
- `write_export_tsv` writes to a `.tmp` sibling first and then calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` and `MOVEFILE_WRITE_THROUGH` for an atomic replace, removing the temporary file when either step fails.
- `export_io_error` falls back to error code 1 when the underlying `std::io::Error` has no raw OS error or the value does not fit a `u32`.
- `diagnostic_transfer_source` resolves precedence as grid selection first, then range selection, then none, and the same precedence backs Ctrl C copy and export.
- The native Win32 declarations for clipboard and save dialog live in this module beside their callers, so no other module owns those handles.