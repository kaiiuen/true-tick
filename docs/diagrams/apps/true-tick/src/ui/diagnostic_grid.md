# Diagnostic Grid Initialization and Detail Fit in diagnostic_grid.rs

Source path: `true-tick/apps/true-tick/src/ui/diagnostic_grid.rs`

```mermaid
flowchart TD
    INIT["initialize_diagnostic_list list"] --> EXT["LVM_SETEXTENDEDLISTVIEWSTYLE gridlines full row select double buffer"]
    EXT --> LOOP{"for each REPORT_COLUMNS label"}
    LOOP -- "next column" --> COL["build ListViewColumn with LVCF_TEXT and LVCF_WIDTH"]
    COL --> INSERT{"LVM_INSERTCOLUMNW index"}
    INSERT -- "negative" --> ERR["return Err GetLastError"]
    INSERT -- "ok" --> LOOP
    LOOP -- "done" --> OK["return Ok"]
    FIT["fit_diagnostic_details_to_viewport list client_width dpi"] --> NULL{"list null"}
    NULL -- "yes" --> RET["return without changes"]
    NULL -- "no" --> POS["read horizontal scroll with GetScrollPos SB_HORZ"]
    POS --> LOOP2{"for each column except the details column"}
    LOOP2 -- "next" --> WIDTH["read LVM_GETCOLUMNWIDTH then clamp to scaled min and max"]
    WIDTH --> SETW["LVM_SETCOLUMNWIDTH clamped width"]
    SETW --> ACC["accumulate fixed_width"]
    ACC --> LOOP2
    LOOP2 -- "done" --> DETAILS["details width is client_width minus fixed_width floored at scaled minimum"]
    DETAILS --> SETD["LVM_SETCOLUMNWIDTH details column"]
    SETD --> RESTORE["SetScrollPos SB_HORZ restored position"]
```

## Notes

- `initialize_diagnostic_list` applies the extended style once and then inserts one column per `REPORT_COLUMNS` entry, using `DIAGNOSTIC_COLUMN_WIDTHS` for each initial width.
- A column insert that returns a negative value stops the loop and returns `Err` from `GetLastError`, so a partially built list is reported rather than silently accepted.
- `fit_diagnostic_details_to_viewport` is a no-op when the list handle is null, which keeps the caller free of null checks.
- Every non details column is clamped into `DIAGNOSTIC_COLUMN_MIN_WIDTHS` and `DIAGNOSTIC_COLUMN_MAX_WIDTHS` after DPI scaling before it is written back.
- The details column is always the last `REPORT_COLUMNS` entry and receives all remaining client width, floored at its scaled minimum so it never collapses below the minimum.
- The horizontal scroll position is read before any width change and restored with `SetScrollPos` after the details column is written, so resizing does not jump the viewport.
- `diagnostic_list_style` removes `LVS_SINGLESEL` so the grid supports multi row drag selection, and the extended style adds gridlines, full row select, and double buffering.
- The display text helper `diagnostic_grid_cell_into_scratch` borrows the wide scratch buffer owned by window state so the pointer returned to `LVN_GETDISPINFO` outlives the notification.