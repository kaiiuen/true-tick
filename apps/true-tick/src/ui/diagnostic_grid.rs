//! Owner data grid backing for the diagnostic window.
//!
//! Holds the virtual listview row buffer, display scratch storage, column
//! sizing helpers, selection plumbing, and the WM_NOTIFY dispatch used by the
//! diagnostic surface. Call sites in `diagnostic_window` reach every item here
//! through a wildcard re-export so paths stay unchanged.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;

use tick_diagnostics::{selected_event_sequences_for_sequences, DiagnosticGridRow, REPORT_COLUMNS};

use crate::tray::list_view_native::*;
use crate::tray::{
    scale_logical, wide, App, GetKeyState, GetLastError, SendMessageW, WS_BORDER, WS_CHILD,
    WS_CLIPCHILDREN, WS_VISIBLE,
};
use crate::ui::diagnostic_window::{
    diagnostic_selection_summary, diagnostic_toolbar_action, record_diagnostic_refresh_event,
    set_diagnostic_action_enabled, set_diagnostic_message, set_diagnostic_selection_summary,
};

#[link(name = "user32")]
extern "system" {
    fn GetScrollPos(window: *mut c_void, bar: i32) -> i32;
    fn SetScrollPos(window: *mut c_void, bar: i32, position: i32, redraw: i32) -> i32;
}

// Owner data row cap for the diagnostic grid. The listview is virtual so this
// bound drives memory directly rather than the native item store.
pub(crate) const EVENT_BUFFER_CAP: usize = 4096;

// Native listview constants for owner data mode. Kept local because the shared
// list_view_native module is also used by call sites outside this file.
pub(crate) const LVS_OWNERDATA: u32 = 0x0000_1000;
pub(crate) const LVM_SETITEMCOUNT: u32 = LVM_FIRST + 47;
pub(crate) const LVN_GETDISPINFO: i32 = -177;
pub(crate) const LVSICF_NOINVALIDATEALL: usize = 0x0001;

pub(crate) const SB_HORZ: i32 = 0;
pub(crate) const WS_HSCROLL: u32 = 0x00100000;

pub(crate) const DIAGNOSTIC_COLUMN_WIDTHS: [i32; 11] =
    [54, 70, 78, 78, 70, 86, 82, 90, 86, 160, 240];
pub(crate) const DIAGNOSTIC_COLUMN_MIN_WIDTHS: [i32; 11] =
    [36, 52, 64, 64, 52, 64, 60, 68, 64, 84, 240];
pub(crate) const DIAGNOSTIC_COLUMN_MAX_WIDTHS: [i32; 11] =
    [84, 120, 140, 144, 120, 144, 112, 124, 124, 260, 640];

#[repr(C)]
pub(crate) struct ListViewDisplayInfoNotification {
    header: NotifyHeader,
    item: ListViewItem,
}

// Bounded ring buffer of displayed rows for the owner data listview. Push
// evicts the oldest entry once the cap is reached.
pub(crate) struct DiagnosticGridRowBuffer {
    rows: VecDeque<DiagnosticGridRow>,
}

impl DiagnosticGridRowBuffer {
    fn new() -> Self {
        Self {
            rows: VecDeque::new(),
        }
    }

    pub(crate) fn push(&mut self, row: DiagnosticGridRow) {
        if self.rows.len() >= EVENT_BUFFER_CAP {
            self.rows.pop_front();
        }
        self.rows.push_back(row);
    }

    fn get(&self, index: usize) -> Option<&DiagnosticGridRow> {
        self.rows.get(index)
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn clear(&mut self) {
        self.rows.clear();
    }
}

// Scratch storage for LVN_GETDISPINFO text. The wide buffer must outlive the
// notification dispatch so it is owned by window state rather than a local.
pub(crate) struct DiagnosticDisplayScratch {
    cells: Vec<u16>,
}

impl DiagnosticDisplayScratch {
    fn new() -> Self {
        Self { cells: Vec::new() }
    }

    fn load(&mut self, text: &str) -> *mut u16 {
        self.cells.clear();
        self.cells.extend(text.encode_utf16());
        self.cells.push(0);
        self.cells.as_mut_ptr()
    }
}

// Owner data backing store and scratch for the diagnostic list. Window state
// that cannot live on App because it is only touched from this module.
pub(crate) struct DiagnosticOwnerData {
    buffer: DiagnosticGridRowBuffer,
    scratch: RefCell<DiagnosticDisplayScratch>,
}

thread_local! {
    static DIAGNOSTIC_OWNER_DATA: RefCell<Option<DiagnosticOwnerData>> = const {
        RefCell::new(None)
    };
}

pub(crate) fn diagnostic_owner_data_with<R>(
    f: impl FnOnce(&mut DiagnosticOwnerData) -> R,
) -> Option<R> {
    DIAGNOSTIC_OWNER_DATA.with(|slot| {
        let mut borrow = slot.borrow_mut();
        if borrow.is_none() {
            *borrow = Some(DiagnosticOwnerData {
                buffer: DiagnosticGridRowBuffer::new(),
                scratch: RefCell::new(DiagnosticDisplayScratch::new()),
            });
        }
        borrow.as_mut().map(f)
    })
}

pub(crate) fn diagnostic_owner_buffer_with<R>(
    f: impl FnOnce(&mut DiagnosticGridRowBuffer) -> R,
) -> Option<R> {
    diagnostic_owner_data_with(|data| f(&mut data.buffer))
}

// Fills the display info item text from the ring buffer row at the requested
// index and column. Returns true when the row and column resolve to text.
fn diagnostic_grid_cell_into_scratch(
    scratch: &mut DiagnosticDisplayScratch,
    buffer: &DiagnosticGridRowBuffer,
    index: usize,
    column: usize,
) -> Option<*mut u16> {
    let row = buffer.get(index)?;
    let cell = row.cells.get(column)?;
    Some(scratch.load(cell))
}

pub(crate) fn diagnostic_displayed_item_count() -> usize {
    diagnostic_owner_buffer_with(|buffer| buffer.len()).unwrap_or(0)
}

pub(crate) const fn diagnostic_list_style() -> u32 {
    (WS_CHILD
        | WS_VISIBLE
        | WS_CLIPCHILDREN
        | WS_BORDER
        | WS_HSCROLL
        | LVS_REPORT
        | LVS_OWNERDATA
        | LVS_SHOWSELALWAYS)
        & !LVS_SINGLESEL
}

const fn diagnostic_list_extended_style() -> usize {
    LVS_EX_GRIDLINES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn initialize_diagnostic_list(list: *mut c_void) -> Result<(), u32> {
    let extended_style = diagnostic_list_extended_style();
    let _ = SendMessageW(
        list,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        extended_style,
        extended_style as isize,
    );
    for (index, label) in REPORT_COLUMNS.iter().enumerate() {
        let mut text = wide(label);
        let column = ListViewColumn {
            mask: LVCF_TEXT | LVCF_WIDTH,
            format: LVCFMT_LEFT,
            width: DIAGNOSTIC_COLUMN_WIDTHS[index],
            text: text.as_mut_ptr(),
            text_maximum: text.len() as i32,
            subitem: index as i32,
            image: 0,
            order: index as i32,
            minimum_width: 0,
            default_width: 0,
            ideal_width: 0,
        };
        let insert_result = SendMessageW(
            list,
            LVM_INSERTCOLUMNW,
            index,
            (&column as *const ListViewColumn).cast::<c_void>() as isize,
        );
        if insert_result < 0 {
            return Err(GetLastError());
        }
    }
    Ok(())
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn fit_diagnostic_details_to_viewport(
    list: *mut c_void,
    client_width: i32,
    dpi: u32,
) {
    if list.is_null() {
        return;
    }
    let horizontal_scroll = GetScrollPos(list, SB_HORZ);
    let mut fixed_width = 0i32;
    for index in 0..REPORT_COLUMNS.len() - 1 {
        let current = SendMessageW(list, LVM_GETCOLUMNWIDTH, index, 0).max(0) as i32;
        let minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[index], dpi);
        let maximum = scale_logical(DIAGNOSTIC_COLUMN_MAX_WIDTHS[index], dpi);
        let width = current.clamp(minimum, maximum);
        let _ = SendMessageW(list, LVM_SETCOLUMNWIDTH, index, width as isize);
        fixed_width = fixed_width.saturating_add(width);
    }
    let details_index = REPORT_COLUMNS.len() - 1;
    let details_minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[details_index], dpi);
    let details_width = client_width
        .saturating_sub(fixed_width)
        .max(details_minimum);
    let _ = SendMessageW(
        list,
        LVM_SETCOLUMNWIDTH,
        details_index,
        details_width as isize,
    );
    let _ = SetScrollPos(list, SB_HORZ, horizontal_scroll, 0);
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn auto_fit_diagnostic_columns(list: *mut c_void, client_width: i32, dpi: u32) {
    let mut measured = [0i32; REPORT_COLUMNS.len()];
    for (index, value) in measured.iter_mut().enumerate() {
        let _ = SendMessageW(list, LVM_SETCOLUMNWIDTH, index, LVSCW_AUTOSIZE as isize);
        let content_width = SendMessageW(list, LVM_GETCOLUMNWIDTH, index, 0).max(0) as i32;
        let _ = SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            index,
            LVSCW_AUTOSIZE_USEHEADER as isize,
        );
        let header_width = SendMessageW(list, LVM_GETCOLUMNWIDTH, index, 0).max(0) as i32;
        *value = content_width.max(header_width);
    }

    let mut fixed_width = 0i32;
    for index in 0..REPORT_COLUMNS.len() - 1 {
        let minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[index], dpi);
        let maximum = scale_logical(DIAGNOSTIC_COLUMN_MAX_WIDTHS[index], dpi);
        let width = measured[index].clamp(minimum, maximum);
        let _ = SendMessageW(list, LVM_SETCOLUMNWIDTH, index, width as isize);
        fixed_width = fixed_width.saturating_add(width);
    }

    let details_index = REPORT_COLUMNS.len() - 1;
    let details_minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[details_index], dpi);
    let details_maximum = scale_logical(DIAGNOSTIC_COLUMN_MAX_WIDTHS[details_index], dpi);
    let measured_details = measured[details_index].clamp(details_minimum, details_maximum);
    let remaining_width = client_width.saturating_sub(fixed_width);
    let details_width = measured_details.max(details_minimum).max(remaining_width);
    let _ = SendMessageW(
        list,
        LVM_SETCOLUMNWIDTH,
        details_index,
        details_width as isize,
    );
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn list_selected_item_positions(list: *mut c_void) -> Vec<usize> {
    let mut previous = -1i32;
    let mut positions = Vec::new();
    loop {
        let next = SendMessageW(
            list,
            LVM_GETNEXTITEM,
            previous as usize,
            LVNI_SELECTED as isize,
        );
        if next < 0 {
            break;
        }
        let next = next as usize;
        positions.push(next);
        previous = next as i32;
    }
    positions
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn update_diagnostic_grid_selection(app: &mut App) {
    if app.diagnostic_ui.diagnostic_refreshing {
        return;
    }
    let Some(list) = app.diagnostic_ui.diagnostic_list else {
        return;
    };
    let events = app.diagnostics.snapshot();
    let positions = list_selected_item_positions(list);
    let buffer_len = diagnostic_displayed_item_count();
    let events_by_sequence: std::collections::HashMap<u64, &tick_diagnostics::DiagnosticEvent> =
        events.iter().map(|event| (event.sequence, event)).collect();
    let mut selected = Vec::new();
    for pos in positions {
        if pos >= buffer_len {
            continue;
        }
        let sequence = diagnostic_owner_buffer_with(|buffer| {
            buffer.get(pos).and_then(|row| row.cells.get(1)).cloned()
        })
        .flatten()
        .and_then(|cell| cell.parse::<u64>().ok());
        if let Some(sequence) = sequence {
            if events_by_sequence.contains_key(&sequence) {
                selected.push(sequence);
            }
        }
    }
    app.diagnostic_ui.diagnostic_grid_selection_sequences = selected;
    app.diagnostic_ui.diagnostic_grid_selection_reset = false;
    if app
        .diagnostic_ui
        .diagnostic_message_text
        .starts_with("Selection reset:")
    {
        set_diagnostic_message(app, "");
    }
    set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, events.len()));
    set_diagnostic_action_enabled(
        app,
        !app.diagnostic_ui
            .diagnostic_grid_selection_sequences
            .is_empty()
            || app
                .diagnostic_ui
                .diagnostic_selection
                .is_some_and(|selection| !selection.is_empty()),
    );
}

pub(crate) fn preserve_diagnostic_grid_selection(
    app: &mut App,
    events: &[tick_diagnostics::DiagnosticEvent],
) {
    if app
        .diagnostic_ui
        .diagnostic_grid_selection_sequences
        .is_empty()
    {
        return;
    }
    let previous_count = app.diagnostic_ui.diagnostic_grid_selection_sequences.len();
    let selected = selected_event_sequences_for_sequences(
        events,
        &app.diagnostic_ui.diagnostic_grid_selection_sequences,
    );
    let invalid_count = previous_count.saturating_sub(selected.len());
    app.diagnostic_ui.diagnostic_grid_selection_sequences = selected;
    if invalid_count > 0 {
        app.diagnostic_ui.diagnostic_grid_selection_reset = true;
        let message = if invalid_count == 1 {
            "Selection reset: 1 selected row is no longer retained."
        } else {
            "Selection reset: selected rows are no longer retained."
        };
        unsafe {
            // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
            set_diagnostic_message(app, message);
        }
        record_diagnostic_refresh_event(
            app,
            "diagnostic.selection.reset",
            format!(
                "source=grid-selection invalid_rows={invalid_count} retained_rows={}",
                events.len()
            ),
        );
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn apply_diagnostic_grid_selection(
    app: &App,
    list: *mut c_void,
    _events: &[tick_diagnostics::DiagnosticEvent],
) {
    if app
        .diagnostic_ui
        .diagnostic_grid_selection_sequences
        .is_empty()
    {
        return;
    }
    let positions: Vec<usize> = diagnostic_owner_buffer_with(|buffer| {
        (0..buffer.len())
            .filter(|index| {
                buffer
                    .get(*index)
                    .and_then(|row| row.cells.get(1))
                    .and_then(|cell| cell.parse::<u64>().ok())
                    .is_some_and(|sequence| {
                        app.diagnostic_ui
                            .diagnostic_grid_selection_sequences
                            .contains(&sequence)
                    })
            })
            .collect()
    })
    .unwrap_or_default();
    for position in positions {
        let item = ListViewItem {
            mask: LVIF_STATE,
            item: position as i32,
            subitem: 0,
            state: LVIS_SELECTED,
            state_mask: LVIS_SELECTED,
            text: std::ptr::null_mut(),
            text_maximum: 0,
            image: 0,
            parameter: 0,
            indent: 0,
            group_id: 0,
            columns: 0,
            column_indices: std::ptr::null_mut(),
            column_formats: std::ptr::null_mut(),
            group: 0,
        };
        let _ = SendMessageW(
            list,
            LVM_SETITEMSTATE,
            position,
            (&item as *const ListViewItem).cast::<c_void>() as isize,
        );
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn select_all_diagnostic_rows(app: &mut App) {
    let Some(list) = app.diagnostic_ui.diagnostic_list else {
        return;
    };
    let item = ListViewItem {
        mask: LVIF_STATE,
        item: -1,
        subitem: 0,
        state: LVIS_SELECTED,
        state_mask: LVIS_SELECTED,
        text: std::ptr::null_mut(),
        text_maximum: 0,
        image: 0,
        parameter: 0,
        indent: 0,
        group_id: 0,
        columns: 0,
        column_indices: std::ptr::null_mut(),
        column_formats: std::ptr::null_mut(),
        group: 0,
    };
    let _ = SendMessageW(
        list,
        LVM_SETITEMSTATE,
        usize::MAX,
        (&item as *const ListViewItem).cast::<c_void>() as isize,
    );
    update_diagnostic_grid_selection(app);
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn set_list_view_item_selected(list: *mut c_void, index: usize, selected: bool) {
    let state = if selected { LVIS_SELECTED } else { 0 };
    let item = ListViewItem {
        mask: LVIF_STATE,
        item: index as i32,
        subitem: 0,
        state,
        state_mask: LVIS_SELECTED,
        text: std::ptr::null_mut(),
        text_maximum: 0,
        image: 0,
        parameter: 0,
        indent: 0,
        group_id: 0,
        columns: 0,
        column_indices: std::ptr::null_mut(),
        column_formats: std::ptr::null_mut(),
        group: 0,
    };
    let _ = SendMessageW(
        list,
        LVM_SETITEMSTATE,
        index,
        (&item as *const ListViewItem).cast::<c_void>() as isize,
    );
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn handle_diagnostic_notify(app: &mut App, l_param: isize) -> bool {
    if l_param == 0 {
        return false;
    }
    let header = &*(l_param as *const NotifyHeader);
    match header.code {
        LVN_GETDISPINFO => {
            let notification = &mut *(l_param as *mut ListViewDisplayInfoNotification);
            if notification.item.mask & LVIF_TEXT != 0 {
                let item_index = notification.item.item.max(0) as usize;
                let column = notification.item.subitem.max(0) as usize;
                let text = diagnostic_owner_data_with(|data| {
                    diagnostic_grid_cell_into_scratch(
                        &mut data.scratch.borrow_mut(),
                        &data.buffer,
                        item_index,
                        column,
                    )
                })
                .flatten();
                match text {
                    Some(pointer) => {
                        notification.item.text = pointer;
                    }
                    None => {
                        if !notification.item.text.is_null() && notification.item.text_maximum > 0 {
                            *notification.item.text = 0;
                        }
                    }
                }
            }
            true
        }
        LVN_ITEMCHANGED => {
            if app.diagnostic_ui.diagnostic_refreshing {
                return false;
            }
            let notification = &*(l_param as *const ListViewNotification);
            if notification.changed & LVIF_STATE != 0
                && ((notification.old_state ^ notification.new_state) & LVIS_SELECTED) != 0
            {
                update_diagnostic_grid_selection(app);
            }
            true
        }
        LVN_KEYDOWN => {
            if app.diagnostic_ui.diagnostic_refreshing {
                return false;
            }
            let notification = &*(l_param as *const ListViewKeyDownNotification);
            if GetKeyState(VK_CONTROL) < 0 {
                match notification.virtual_key {
                    key if key == b'C' as u16 => {
                        diagnostic_toolbar_action(app, "copy");
                        return true;
                    }
                    key if key == b'A' as u16 => {
                        select_all_diagnostic_rows(app);
                        return true;
                    }
                    _ => {}
                }
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn diagnostic_grid_refresh_message(
    snapshot_rows: usize,
    displayed_rows: usize,
    item_count: isize,
    evicted_rows: usize,
) -> String {
    format!(
        "snapshot_rows={snapshot_rows} displayed_rows={displayed_rows} item_count={item_count} evicted_rows={evicted_rows}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn diagnostic_grid_style_supports_visible_multi_row_drag_selection() {
        let style = diagnostic_list_style();
        let extended_style = diagnostic_list_extended_style();
        assert_ne!(style & LVS_REPORT, 0);
        assert_eq!(style & LVS_SINGLESEL, 0);
        assert_ne!(style & LVS_SHOWSELALWAYS, 0);
        assert_ne!(extended_style & LVS_EX_GRIDLINES, 0);
        assert_ne!(extended_style & LVS_EX_FULLROWSELECT, 0);
        assert_ne!(extended_style & LVS_EX_DOUBLEBUFFER, 0);
        assert_eq!(LVN_ITEMCHANGED, -101);
        assert_eq!(LVN_KEYDOWN, -155);
        assert_eq!(WM_NOTIFY, 0x004E);
        assert!(
            size_of::<NotifyHeader>()
                >= size_of::<*mut c_void>() + size_of::<usize>() + size_of::<i32>()
        );
        assert!(
            size_of::<ListViewNotification>()
                >= size_of::<NotifyHeader>()
                    + 3 * size_of::<i32>()
                    + size_of::<crate::tray::Point>()
                    + size_of::<isize>()
        );
        assert!(
            size_of::<ListViewKeyDownNotification>()
                >= size_of::<NotifyHeader>() + size_of::<u16>() + size_of::<u32>()
        );
    }

    #[test]
    fn diagnostic_grid_message_contract_uses_correct_native_messages_and_bounded_counts() {
        assert_eq!(LVM_SETITEMCOUNT, LVM_FIRST + 47);
        assert_eq!(LVN_GETDISPINFO, -177);
        assert_eq!(LVS_OWNERDATA, 0x0000_1000);
        assert_ne!(diagnostic_list_style() & LVS_OWNERDATA, 0);
        assert_eq!(EVENT_BUFFER_CAP, 4096);
        assert_eq!(
            diagnostic_grid_refresh_message(12, 11, 11, 2),
            "snapshot_rows=12 displayed_rows=11 item_count=11 evicted_rows=2"
        );
    }

    fn test_grid_row(first_cell: &str, sequence: &str) -> DiagnosticGridRow {
        let mut cells = vec![first_cell.to_owned(), sequence.to_owned()];
        for index in 2..REPORT_COLUMNS.len() {
            cells.push(format!("cell-{index}"));
        }
        DiagnosticGridRow { cells }
    }

    #[test]
    fn ring_buffer_bounds_at_capacity_and_evicts_oldest() {
        let mut buffer = DiagnosticGridRowBuffer::new();
        for index in 0..(EVENT_BUFFER_CAP + 8) {
            buffer.push(test_grid_row(&index.to_string(), &index.to_string()));
        }
        assert_eq!(buffer.len(), EVENT_BUFFER_CAP);
        assert_eq!(buffer.get(0).map(|row| row.cells[0].as_str()), Some("8"));
        assert_eq!(
            buffer
                .get(EVENT_BUFFER_CAP - 1)
                .map(|row| row.cells[0].as_str()),
            Some((EVENT_BUFFER_CAP + 7).to_string().as_str())
        );
        assert!(buffer.get(EVENT_BUFFER_CAP).is_none());
    }

    #[test]
    fn ring_buffer_indexed_accessor_matches_insertion_order() {
        let mut buffer = DiagnosticGridRowBuffer::new();
        for index in 0..5usize {
            buffer.push(test_grid_row(&index.to_string(), "7"));
        }
        for index in 0..5usize {
            assert_eq!(
                buffer.get(index).map(|row| row.cells[0].as_str()),
                Some(index.to_string().as_str())
            );
        }
        assert!(buffer.get(5).is_none());
    }

    #[test]
    fn displayed_item_count_tracks_buffer_length() {
        DIAGNOSTIC_OWNER_DATA.with(|slot| {
            *slot.borrow_mut() = Some(DiagnosticOwnerData {
                buffer: DiagnosticGridRowBuffer::new(),
                scratch: RefCell::new(DiagnosticDisplayScratch::new()),
            });
        });
        assert_eq!(diagnostic_displayed_item_count(), 0);
        diagnostic_owner_buffer_with(|buffer| {
            buffer.push(test_grid_row("1", "11"));
            buffer.push(test_grid_row("2", "12"));
        });
        assert_eq!(diagnostic_displayed_item_count(), 2);
        diagnostic_owner_buffer_with(|buffer| buffer.clear());
        assert_eq!(diagnostic_displayed_item_count(), 0);
    }

    #[test]
    fn getdispinfo_text_matches_buffer_row_for_valid_index() {
        let mut buffer = DiagnosticGridRowBuffer::new();
        buffer.push(test_grid_row("row-one", "42"));
        let mut scratch = DiagnosticDisplayScratch::new();
        for (column, expected) in [(0usize, "row-one"), (1usize, "42"), (4usize, "cell-4")] {
            let pointer = diagnostic_grid_cell_into_scratch(&mut scratch, &buffer, 0, column)
                .expect("valid cell must produce a pointer");
            let wide_text: Vec<u16> = {
                let mut text = Vec::new();
                let mut cursor = pointer;
                unsafe {
                    // SAFETY: pointer references the NUL terminated scratch buffer
                    while *cursor != 0 {
                        text.push(*cursor);
                        cursor = cursor.add(1);
                    }
                }
                text
            };
            assert_eq!(
                String::from_utf16_lossy(&wide_text),
                expected,
                "column {column} text must match"
            );
        }
    }

    #[test]
    fn getdispinfo_handles_out_of_range_index_without_panicking() {
        let mut buffer = DiagnosticGridRowBuffer::new();
        buffer.push(test_grid_row("row-one", "42"));
        let mut scratch = DiagnosticDisplayScratch::new();
        assert!(diagnostic_grid_cell_into_scratch(&mut scratch, &buffer, 1, 0).is_none());
        assert!(diagnostic_grid_cell_into_scratch(&mut scratch, &buffer, usize::MAX, 0).is_none());
        assert!(
            diagnostic_grid_cell_into_scratch(&mut scratch, &buffer, 0, REPORT_COLUMNS.len())
                .is_none()
        );
        let empty = DiagnosticGridRowBuffer::new();
        assert!(diagnostic_grid_cell_into_scratch(&mut scratch, &empty, 0, 0).is_none());
    }
}
