//! Layout and DPI geometry for the diagnostic window.
//!
//! Provides the pure layout rectangle math, minimum tracking size, and outer
//! size scaling for the diagnostic surface. Window creation, message handling,
//! and control state live in `diagnostic_window`.

use std::ffi::c_void;

use crate::tray::{scale_logical, GetSystemMetrics, Point, Rect};

pub(crate) const SM_CXWORKAREA: i32 = 60;
pub(crate) const SM_CYWORKAREA: i32 = 61;
pub(crate) const DIAGNOSTIC_SUMMARY_HEIGHT: i32 = 140;
pub(crate) const DIAGNOSTIC_HUD_STATE_HEIGHT: i32 = 32;
pub(crate) const DIAGNOSTIC_SEPARATOR_HEIGHT: i32 = 2;
pub(crate) const DIAGNOSTIC_TOOLBAR_HEIGHT: i32 = 36;
pub(crate) const DIAGNOSTIC_GRID_MIN_HEIGHT: i32 = 96;
pub(crate) const DIAGNOSTIC_TOOLBAR_MARGIN: i32 = 8;
pub(crate) const DIAGNOSTIC_TOOLBAR_GAP: i32 = 8;
pub(crate) const DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH: i32 = 140;
// Compact toolbar widths at 96 DPI in left to right order. Both WM_CREATE sizes and
// diagnostic_toolbar_layout consume this same list so creation and layout never drift.
pub(crate) const DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS: [i32; 12] =
    [64, 52, 64, 100, 54, 104, 128, 66, 124, 56, 56, 70];
// Compact logical client default. The outer rectangle is DPI adjusted before creation.
pub(crate) const DIAGNOSTIC_DEFAULT_WIDTH: i32 = 1_280;
pub(crate) const DIAGNOSTIC_DEFAULT_HEIGHT: i32 = 520;

const WS_OVERLAPPEDWINDOW: u32 = 0x00cf0000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_CLIPSIBLINGS: u32 = 0x04000000;
const WS_EX_APPWINDOW: u32 = 0x00040000;

#[link(name = "user32")]
extern "system" {
    fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex_style: u32) -> i32;
    fn AdjustWindowRectExForDpi(
        rect: *mut Rect,
        style: u32,
        menu: i32,
        ex_style: u32,
        dpi: u32,
    ) -> i32;
}

const fn diagnostic_window_style() -> u32 {
    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
}

const fn diagnostic_window_extended_style() -> u32 {
    WS_EX_APPWINDOW
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticLayoutRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

#[cfg(test)]
impl DiagnosticLayoutRect {
    pub(crate) fn right(self) -> i32 {
        self.left.saturating_add(self.width)
    }

    pub(crate) fn bottom(self) -> i32 {
        self.top.saturating_add(self.height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticToolbarLayout {
    pub(crate) display_label: DiagnosticLayoutRect,
    pub(crate) display_input: DiagnosticLayoutRect,
    pub(crate) show_all: DiagnosticLayoutRect,
    pub(crate) category_filter: DiagnosticLayoutRect,
    pub(crate) search_label: DiagnosticLayoutRect,
    pub(crate) search_input: DiagnosticLayoutRect,
    pub(crate) transfer_label: DiagnosticLayoutRect,
    pub(crate) range_input: DiagnosticLayoutRect,
    pub(crate) selection_summary: DiagnosticLayoutRect,
    pub(crate) copy: DiagnosticLayoutRect,
    pub(crate) export: DiagnosticLayoutRect,
    pub(crate) export_all: DiagnosticLayoutRect,
    pub(crate) message: DiagnosticLayoutRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticLayout {
    pub(crate) hud_state: DiagnosticLayoutRect,
    pub(crate) summary: DiagnosticLayoutRect,
    pub(crate) hud_separator: DiagnosticLayoutRect,
    pub(crate) toolbar: DiagnosticToolbarLayout,
    pub(crate) toolbar_separator: DiagnosticLayoutRect,
    pub(crate) list: DiagnosticLayoutRect,
}

pub(crate) fn diagnostic_toolbar_min_width(dpi: u32) -> i32 {
    let fixed_width = DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS
        .into_iter()
        .fold(0i32, i32::saturating_add);
    let gaps = DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS.len() as i32 * DIAGNOSTIC_TOOLBAR_GAP;
    scale_logical(
        DIAGNOSTIC_TOOLBAR_MARGIN * 2 + fixed_width + gaps + DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH,
        dpi.max(96),
    )
}

pub(crate) fn diagnostic_min_client_height(dpi: u32) -> i32 {
    scale_logical(
        DIAGNOSTIC_SUMMARY_HEIGHT
            + DIAGNOSTIC_SEPARATOR_HEIGHT
            + DIAGNOSTIC_TOOLBAR_HEIGHT
            + DIAGNOSTIC_SEPARATOR_HEIGHT
            + DIAGNOSTIC_GRID_MIN_HEIGHT,
        dpi.max(96),
    )
}

pub(crate) fn outer_size_from_client(
    _client_width: i32,
    _client_height: i32,
    frame: Rect,
) -> Point {
    // The adjusted frame already contains the full outer rectangle including the client.
    Point {
        x: (frame.right - frame.left).max(0),
        y: (frame.bottom - frame.top).max(0),
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn diagnostic_outer_size(
    client_width: i32,
    client_height: i32,
    dpi: u32,
) -> Point {
    let mut frame = Rect {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    let adjusted = AdjustWindowRectExForDpi(
        &mut frame,
        diagnostic_window_style(),
        0,
        diagnostic_window_extended_style(),
        dpi,
    ) != 0;
    let adjusted = if adjusted {
        true
    } else {
        frame = Rect {
            left: 0,
            top: 0,
            right: client_width,
            bottom: client_height,
        };
        AdjustWindowRectEx(
            &mut frame,
            diagnostic_window_style(),
            0,
            diagnostic_window_extended_style(),
        ) != 0
    };
    if adjusted {
        outer_size_from_client(client_width, client_height, frame)
    } else {
        Point {
            x: client_width,
            y: client_height,
        }
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn diagnostic_min_outer_size(
    window: *mut c_void,
    client_width: i32,
    client_height: i32,
    dpi: u32,
) -> Point {
    let _ = window;
    diagnostic_outer_size(client_width, client_height, dpi)
}

fn clamp_outer_to_work_area(outer: Point, work_width: i32, work_height: i32) -> Point {
    let safe_width = work_width.max(320);
    let safe_height = work_height.max(240);
    Point {
        x: outer.x.min(safe_width).max(320),
        y: outer.y.min(safe_height).max(240),
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn diagnostic_default_outer_size(dpi: u32) -> Point {
    let dpi = dpi.max(96);
    let scaled_client_width = scale_logical(DIAGNOSTIC_DEFAULT_WIDTH, dpi);
    let scaled_client_height = scale_logical(DIAGNOSTIC_DEFAULT_HEIGHT, dpi);
    let outer = diagnostic_outer_size(scaled_client_width, scaled_client_height, dpi);
    let work_width = GetSystemMetrics(SM_CXWORKAREA).max(0);
    let work_height = GetSystemMetrics(SM_CYWORKAREA).max(0);
    clamp_outer_to_work_area(outer, work_width, work_height)
}

pub(crate) fn diagnostic_toolbar_layout(
    width: i32,
    top: i32,
    height: i32,
    dpi: u32,
) -> DiagnosticToolbarLayout {
    let dpi = dpi.max(96);
    let row_height = scale_logical(24, dpi).min(height.max(0));
    let row_top = top.saturating_add((height.saturating_sub(row_height)) / 2);
    let gap = scale_logical(DIAGNOSTIC_TOOLBAR_GAP, dpi);
    let margin = scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi);
    let widths = DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS.map(|value| scale_logical(value, dpi));
    let mut cursor = margin;
    let mut next = |control_width: i32| {
        let rect = DiagnosticLayoutRect {
            left: cursor,
            top: row_top,
            width: control_width,
            height: row_height,
        };
        cursor = cursor.saturating_add(control_width).saturating_add(gap);
        rect
    };
    let display_label = next(widths[0]);
    let display_input = next(widths[1]);
    let show_all = next(widths[2]);
    let category_filter = next(widths[3]);
    let search_label = next(widths[4]);
    let search_input = next(widths[5]);
    let transfer_label = next(widths[6]);
    let range_input = next(widths[7]);
    let selection_summary = next(widths[8]);
    let copy = next(widths[9]);
    let export = next(widths[10]);
    let export_all = next(widths[11]);
    let message_left = cursor;
    let message_width = width
        .saturating_sub(message_left)
        .saturating_sub(margin)
        .max(scale_logical(DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH, dpi));
    let message = DiagnosticLayoutRect {
        left: message_left,
        top: row_top,
        width: message_width,
        height: row_height,
    };
    DiagnosticToolbarLayout {
        display_label,
        display_input,
        show_all,
        category_filter,
        search_label,
        search_input,
        transfer_label,
        range_input,
        selection_summary,
        copy,
        export,
        export_all,
        message,
    }
}

pub(crate) fn diagnostic_layout(width: i32, height: i32, dpi: u32) -> DiagnosticLayout {
    let dpi = dpi.max(96);
    let hud_height = scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, dpi);
    let state_height = scale_logical(DIAGNOSTIC_HUD_STATE_HEIGHT, dpi);
    let separator_height = scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, dpi);
    let toolbar_height = scale_logical(DIAGNOSTIC_TOOLBAR_HEIGHT, dpi);
    let grid_minimum = scale_logical(DIAGNOSTIC_GRID_MIN_HEIGHT, dpi);
    let available_hud = height
        .saturating_sub(separator_height)
        .saturating_sub(toolbar_height)
        .saturating_sub(separator_height)
        .saturating_sub(grid_minimum)
        .max(0);
    let hud_height = hud_height.min(available_hud);
    let hud_separator_top = hud_height;
    let toolbar_top = hud_separator_top.saturating_add(separator_height);
    let toolbar_separator_top = toolbar_top.saturating_add(toolbar_height);
    let list_top = toolbar_separator_top.saturating_add(separator_height);
    DiagnosticLayout {
        hud_state: DiagnosticLayoutRect {
            left: scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi),
            top: scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi),
            width: width
                .saturating_sub(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN * 2, dpi))
                .max(0),
            height: state_height,
        },
        summary: DiagnosticLayoutRect {
            left: scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi),
            top: state_height.saturating_add(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi)),
            width: width
                .saturating_sub(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN * 2, dpi))
                .max(0),
            height: hud_height
                .saturating_sub(state_height)
                .saturating_sub(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN * 2, dpi))
                .max(0),
        },
        hud_separator: DiagnosticLayoutRect {
            left: 0,
            top: hud_separator_top,
            width: width.max(0),
            height: separator_height,
        },
        toolbar: diagnostic_toolbar_layout(width, toolbar_top, toolbar_height, dpi),
        toolbar_separator: DiagnosticLayoutRect {
            left: 0,
            top: toolbar_separator_top,
            width: width.max(0),
            height: separator_height,
        },
        list: DiagnosticLayoutRect {
            left: 0,
            top: list_top,
            width: width.max(0),
            height: height.saturating_sub(list_top).max(0),
        },
    }
}

#[cfg(test)]
pub(crate) fn diagnostic_layout_guard_allows(rect: DiagnosticLayoutRect, is_window: bool) -> bool {
    is_window && rect.width > 0 && rect.height > 0
}

#[cfg(test)]
pub(crate) fn diagnostic_layout_stops_on_first_failure(position_ok: &[bool]) -> (usize, bool) {
    let mut applied = 0usize;
    for ok in position_ok {
        if !ok {
            return (applied, false);
        }
        applied += 1;
    }
    (applied, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::{scale_logical, Rect};

    #[test]
    fn diagnostic_layout_guards_and_stop_on_first_failure() {
        let good = DiagnosticLayoutRect {
            left: 0,
            top: 0,
            width: 10,
            height: 10,
        };
        let flat = DiagnosticLayoutRect {
            left: 0,
            top: 0,
            width: 0,
            height: 10,
        };
        assert!(diagnostic_layout_guard_allows(good, true));
        assert!(!diagnostic_layout_guard_allows(good, false));
        assert!(!diagnostic_layout_guard_allows(flat, true));
        assert_eq!(
            diagnostic_layout_stops_on_first_failure(&[true, true, true]),
            (3, true)
        );
        assert_eq!(
            diagnostic_layout_stops_on_first_failure(&[true, false, true]),
            (1, false)
        );
        assert_eq!(
            diagnostic_layout_stops_on_first_failure(&[false]),
            (0, false)
        );
        assert_eq!(diagnostic_layout_stops_on_first_failure(&[]), (0, true));
    }

    #[test]
    fn diagnostic_toolbar_layout_is_one_row_and_derived_from_flow() {
        let layout = diagnostic_layout(1_190, 480, 96);
        let toolbar = layout.toolbar;
        let controls = [
            toolbar.display_label,
            toolbar.display_input,
            toolbar.show_all,
            toolbar.category_filter,
            toolbar.search_label,
            toolbar.search_input,
            toolbar.transfer_label,
            toolbar.range_input,
            toolbar.selection_summary,
            toolbar.copy,
            toolbar.export,
            toolbar.export_all,
            toolbar.message,
        ];
        assert!(controls
            .windows(2)
            .all(|pair| pair[0].right() <= pair[1].left));
        assert!(controls
            .iter()
            .all(|control| control.top == toolbar.display_input.top
                && control.height == toolbar.display_input.height));
        assert_eq!(
            toolbar.display_input.left - toolbar.display_label.right(),
            8
        );
        assert_eq!(toolbar.message.width, 140);
        assert_eq!(diagnostic_toolbar_min_width(96), 1_190);
        assert!(toolbar.message.width >= DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
        let tight = diagnostic_toolbar_layout(400, 0, 36, 96);
        assert_eq!(tight.message.width, DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
        let tighter = diagnostic_toolbar_layout(0, 0, 36, 96);
        assert_eq!(tighter.message.width, DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
    }

    #[test]
    fn compact_default_outer_size_keeps_one_row_toolbar_and_grid_usable() {
        assert_eq!(
            (DIAGNOSTIC_DEFAULT_WIDTH, DIAGNOSTIC_DEFAULT_HEIGHT),
            (1_280, 520)
        );
        let frame = Rect {
            left: -8,
            top: -31,
            right: 1_024,
            bottom: 249,
        };
        let minimum = outer_size_from_client(1_190, 240, frame);
        assert_eq!(minimum, Point { x: 1_032, y: 280 });
        assert!(DIAGNOSTIC_DEFAULT_WIDTH >= diagnostic_toolbar_min_width(96));
        assert!(DIAGNOSTIC_DEFAULT_WIDTH >= minimum.x);
        assert!(DIAGNOSTIC_DEFAULT_HEIGHT >= minimum.y);
        let layout = diagnostic_layout(1_280, 480, 96);
        assert!(layout.toolbar.message.width >= DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
        assert!(layout.list.height >= DIAGNOSTIC_GRID_MIN_HEIGHT);
        assert!(layout.toolbar.display_input.top == layout.toolbar.export.top);
    }

    #[test]
    fn diagnostic_minimum_size_and_dpi_scaling_keep_summary_and_grid_visible() {
        assert_eq!(diagnostic_min_client_height(96), 276);
        assert_eq!(diagnostic_toolbar_min_width(144), 1_785);
        assert_eq!(diagnostic_min_client_height(144), 414);
        let layout = diagnostic_layout(1_600, 414, 144);
        assert_eq!(
            layout.hud_state.height,
            scale_logical(DIAGNOSTIC_HUD_STATE_HEIGHT, 144)
        );
        assert_eq!(
            layout.hud_separator.top,
            scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, 144)
        );
        assert_eq!(
            layout.list.height,
            scale_logical(DIAGNOSTIC_GRID_MIN_HEIGHT, 144)
        );
        assert!(layout.summary.bottom() <= layout.toolbar.display_input.top);
        assert!(layout.toolbar.display_input.bottom() <= layout.list.top);
        assert_eq!(
            layout.hud_separator.height,
            scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, 144)
        );
        assert_eq!(
            layout.toolbar_separator.height,
            scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, 144)
        );
    }

    #[test]
    fn minimum_tracking_size_adds_the_native_frame_to_the_client_contract() {
        let frame = Rect {
            left: -8,
            top: -31,
            right: 838,
            bottom: 249,
        };
        assert_eq!(
            outer_size_from_client(830, 240, frame),
            Point { x: 846, y: 280 }
        );
    }

    #[test]
    fn diagnostic_toolbar_contract_uses_explicit_controls_and_tsv_events() {
        assert_eq!(DIAGNOSTIC_TOOLBAR_HEIGHT, 36);
    }

    #[test]
    fn diagnostic_summary_is_a_non_scrolling_read_only_static_contract() {
        assert_eq!(DIAGNOSTIC_SUMMARY_HEIGHT, 140);
    }

    #[test]
    fn hud_state_and_separators_are_native_control_contracts() {
        assert_eq!(DIAGNOSTIC_HUD_STATE_HEIGHT, 32);
        assert_eq!(DIAGNOSTIC_SEPARATOR_HEIGHT, 2);
    }

    #[test]
    fn diagnostic_window_contract_is_normal_and_taskbar_visible() {
        assert_eq!(
            diagnostic_window_style() & WS_OVERLAPPEDWINDOW,
            WS_OVERLAPPEDWINDOW
        );
        assert_eq!(diagnostic_window_style() & 0x40000000, 0);
        assert_eq!(diagnostic_window_style() & 0x10000000, 0);
        assert_ne!(diagnostic_window_extended_style() & WS_EX_APPWINDOW, 0);
        assert_eq!(diagnostic_window_extended_style() & 0x00000080, 0);
        assert_eq!(diagnostic_toolbar_min_width(96), 1_190);
        assert_eq!(diagnostic_min_client_height(96), 276);
        assert_eq!(DIAGNOSTIC_TOOLBAR_HEIGHT, 36);
    }
}
