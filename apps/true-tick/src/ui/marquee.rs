//! Rubber-band marquee selection for the diagnostic list view.
//!
//! This module subclasses the SysListView32 control inside the diagnostic
//! window so a plain left button drag draws a focus-rect band and selects
//! every row it intersects. Shift and Ctrl clicks keep the native list view
//! behavior for range and toggle selection.

use std::ffi::c_void;

use crate::tray::list_view_native::{
    LVIR_BOUNDS, LVM_GETITEMCOUNT, LVM_GETITEMRECT, VK_CONTROL, VK_SHIFT,
};
use crate::tray::{
    App, CallWindowProcW, DefWindowProcW, DrawFocusRect, GetCapture, GetDC, GetKeyState,
    GetSystemMetrics, GetWindowLongPtrW, InvalidateRect, Point, Rect, ReleaseCapture, ReleaseDC,
    SendMessageW, SetCapture, GWLP_USERDATA,
};
use crate::ui::diagnostic_window::{
    list_selected_item_positions, set_list_view_item_selected, update_diagnostic_grid_selection,
};

const VK_LBUTTON: i32 = 0x01;
const VK_ESCAPE: i32 = 0x1B;
const MK_LBUTTON: usize = 0x0001;

const WM_KEYDOWN: u32 = 0x0100;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_CAPTURECHANGED: u32 = 0x0215;

const SM_CXDRAG: i32 = 68;
const SM_CYDRAG: i32 = 69;

fn point_from_lparam(l_param: isize) -> Point {
    Point {
        x: (l_param as u16) as i16 as i32,
        y: ((l_param >> 16) as u16) as i16 as i32,
    }
}

fn should_use_native_click_selection(shift_held: bool, ctrl_held: bool) -> bool {
    shift_held || ctrl_held
}

fn marquee_requires_left_mouse_button_pressed_flag(w_param: usize) -> bool {
    w_param & MK_LBUTTON != 0
}

fn marquee_drag_exceeded(anchor: Point, current: Point, cx_drag: i32, cy_drag: i32) -> bool {
    (current.x - anchor.x).abs() >= cx_drag || (current.y - anchor.y).abs() >= cy_drag
}

fn points_to_rect(pt1: Point, pt2: Point) -> Rect {
    Rect {
        left: pt1.x.min(pt2.x),
        top: pt1.y.min(pt2.y),
        right: pt1.x.max(pt2.x),
        bottom: pt1.y.max(pt2.y),
    }
}

fn rects_intersect(r1: &Rect, r2: &Rect) -> bool {
    r1.left <= r2.right && r1.right >= r2.left && r1.top <= r2.bottom && r1.bottom >= r2.top
}

unsafe fn draw_marquee_rect(list: *mut c_void, anchor: Point, current: Point) {
    let rect = points_to_rect(anchor, current);
    if rect.left == rect.right || rect.top == rect.bottom {
        return;
    }
    let hdc = GetDC(list);
    if !hdc.is_null() {
        DrawFocusRect(hdc, &rect);
        ReleaseDC(list, hdc);
    }
}

fn marquee_selected_indices(
    band: &Rect,
    row_rects: &[Rect],
    initial_selected: &[usize],
    is_additive: bool,
) -> Vec<usize> {
    let mut selected = Vec::new();
    for (index, row_rect) in row_rects.iter().enumerate() {
        let in_band = rects_intersect(band, row_rect);
        let should_be_selected = if is_additive {
            in_band || initial_selected.contains(&index)
        } else {
            in_band
        };
        if should_be_selected {
            selected.push(index);
        }
    }
    selected
}

unsafe fn update_marquee_selection(app: &mut App, list: *mut c_void, band: &Rect) {
    let count = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0).max(0) as usize;
    let is_additive = GetKeyState(VK_CONTROL) < 0;
    let mut row_rects = Vec::with_capacity(count);
    for index in 0..count {
        let mut row_rect = Rect {
            left: LVIR_BOUNDS,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let rect_result = SendMessageW(
            list,
            LVM_GETITEMRECT,
            index,
            (&mut row_rect as *mut Rect) as isize,
        );
        if rect_result != 0 {
            row_rects.push(row_rect);
        } else {
            row_rects.push(Rect {
                left: 0,
                top: -1,
                right: 0,
                bottom: -1,
            });
        }
    }
    let selected_indices = marquee_selected_indices(
        band,
        &row_rects,
        &app.diagnostic_marquee_initial_selected,
        is_additive,
    );
    for index in 0..count {
        let should_be_selected = selected_indices.contains(&index);
        set_list_view_item_selected(list, index, should_be_selected);
    }
}

unsafe fn handle_marquee_lbuttondown(hwnd: *mut c_void, l_param: isize) {
    let parent = app_from_list(hwnd);
    if parent.is_null() {
        return;
    }
    if GetKeyState(VK_LBUTTON) >= 0 {
        return;
    }
    let app = &mut *parent;
    let pt = point_from_lparam(l_param);
    app.diagnostic_marquee_pending = true;
    app.diagnostic_marquee_anchor = pt;
    app.diagnostic_marquee_current = pt;
}

unsafe fn handle_marquee_pending_mousemove(hwnd: *mut c_void, l_param: isize) {
    let parent = app_from_list(hwnd);
    if parent.is_null() {
        return;
    }
    let app = &mut *parent;
    if !app.diagnostic_marquee_pending || app.diagnostic_marquee_active {
        return;
    }
    let pt = point_from_lparam(l_param);
    let cx_drag = GetSystemMetrics(SM_CXDRAG).max(1);
    let cy_drag = GetSystemMetrics(SM_CYDRAG).max(1);
    if !marquee_drag_exceeded(app.diagnostic_marquee_anchor, pt, cx_drag, cy_drag) {
        return;
    }
    app.diagnostic_marquee_pending = false;
    app.diagnostic_marquee_active = true;
    SetCapture(hwnd);
    app.diagnostic_marquee_current = pt;
    app.diagnostic_marquee_initial_selected = list_selected_item_positions(hwnd);
    let band = points_to_rect(app.diagnostic_marquee_anchor, pt);
    update_marquee_selection(app, hwnd, &band);
    update_diagnostic_grid_selection(app);
}

unsafe fn handle_marquee_mousemove(hwnd: *mut c_void, l_param: isize) {
    let parent = app_from_list(hwnd);
    if parent.is_null() {
        return;
    }
    let app = &mut *parent;
    if !app.diagnostic_marquee_active {
        return;
    }
    let pt = point_from_lparam(l_param);
    draw_marquee_rect(
        hwnd,
        app.diagnostic_marquee_anchor,
        app.diagnostic_marquee_current,
    );
    app.diagnostic_marquee_current = pt;
    draw_marquee_rect(
        hwnd,
        app.diagnostic_marquee_anchor,
        app.diagnostic_marquee_current,
    );
    let band = points_to_rect(app.diagnostic_marquee_anchor, pt);
    update_marquee_selection(app, hwnd, &band);
    update_diagnostic_grid_selection(app);
}

unsafe fn handle_marquee_lbuttonup(hwnd: *mut c_void) {
    let parent = app_from_list(hwnd);
    if parent.is_null() {
        return;
    }
    let app = &mut *parent;
    if !app.diagnostic_marquee_active {
        return;
    }
    draw_marquee_rect(
        hwnd,
        app.diagnostic_marquee_anchor,
        app.diagnostic_marquee_current,
    );
    app.diagnostic_marquee_active = false;
    app.diagnostic_marquee_initial_selected.clear();
    if GetCapture() == hwnd {
        ReleaseCapture();
    }
    InvalidateRect(hwnd, std::ptr::null(), 1);
    update_diagnostic_grid_selection(app);
}

unsafe fn handle_marquee_cancel(hwnd: *mut c_void) {
    let parent = app_from_list(hwnd);
    if parent.is_null() {
        return;
    }
    let app = &mut *parent;
    app.diagnostic_marquee_pending = false;
    if app.diagnostic_marquee_active {
        handle_marquee_lbuttonup(hwnd);
    }
}

unsafe fn handle_marquee_capturechanged(hwnd: *mut c_void) {
    let parent = app_from_list(hwnd);
    if parent.is_null() {
        return;
    }
    let app = &mut *parent;
    app.diagnostic_marquee_pending = false;
    if app.diagnostic_marquee_active {
        draw_marquee_rect(
            hwnd,
            app.diagnostic_marquee_anchor,
            app.diagnostic_marquee_current,
        );
        app.diagnostic_marquee_active = false;
        app.diagnostic_marquee_initial_selected.clear();
        InvalidateRect(hwnd, std::ptr::null(), 1);
        update_diagnostic_grid_selection(app);
    }
}

unsafe fn app_from_list(hwnd: *mut c_void) -> *mut App {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App
}

pub(crate) unsafe extern "system" fn diagnostic_list_proc(
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    let parent = app_from_list(hwnd);
    let prev_proc = if !parent.is_null() {
        (*parent).diagnostic_list_prev_proc
    } else {
        None
    };
    match message {
        WM_LBUTTONDOWN => {
            // Always forward to the native list view so it can establish the
            // focus and selection anchor needed for Shift click range selection
            // Only arm the custom marquee for plain clicks without modifiers
            let native_modifiers = should_use_native_click_selection(
                GetKeyState(VK_SHIFT) < 0,
                GetKeyState(VK_CONTROL) < 0,
            );
            let result = if let Some(prev) = prev_proc {
                CallWindowProcW(prev, hwnd, message, w_param, l_param)
            } else {
                DefWindowProcW(hwnd, message, w_param, l_param)
            };
            if !native_modifiers {
                handle_marquee_lbuttondown(hwnd, l_param);
            }
            result
        }
        WM_MOUSEMOVE => {
            let left_button_held = marquee_requires_left_mouse_button_pressed_flag(w_param)
                && GetKeyState(VK_LBUTTON) < 0;
            if !left_button_held {
                if !parent.is_null()
                    && ((*parent).diagnostic_marquee_pending || (*parent).diagnostic_marquee_active)
                {
                    handle_marquee_cancel(hwnd);
                }
                if let Some(prev) = prev_proc {
                    CallWindowProcW(prev, hwnd, message, w_param, l_param)
                } else {
                    DefWindowProcW(hwnd, message, w_param, l_param)
                }
            } else {
                if !parent.is_null() && (*parent).diagnostic_marquee_pending {
                    handle_marquee_pending_mousemove(hwnd, l_param);
                }
                if !parent.is_null() && (*parent).diagnostic_marquee_active {
                    handle_marquee_mousemove(hwnd, l_param);
                    0
                } else if let Some(prev) = prev_proc {
                    CallWindowProcW(prev, hwnd, message, w_param, l_param)
                } else {
                    DefWindowProcW(hwnd, message, w_param, l_param)
                }
            }
        }
        WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if !parent.is_null()
                && ((*parent).diagnostic_marquee_pending || (*parent).diagnostic_marquee_active)
            {
                handle_marquee_cancel(hwnd);
            }
            if let Some(prev) = prev_proc {
                CallWindowProcW(prev, hwnd, message, w_param, l_param)
            } else {
                DefWindowProcW(hwnd, message, w_param, l_param)
            }
        }
        WM_KEYDOWN => {
            if w_param == VK_ESCAPE as usize
                && !parent.is_null()
                && ((*parent).diagnostic_marquee_pending || (*parent).diagnostic_marquee_active)
            {
                handle_marquee_cancel(hwnd);
                0
            } else if let Some(prev) = prev_proc {
                CallWindowProcW(prev, hwnd, message, w_param, l_param)
            } else {
                DefWindowProcW(hwnd, message, w_param, l_param)
            }
        }
        WM_LBUTTONUP => {
            if !parent.is_null() && (*parent).diagnostic_marquee_active {
                handle_marquee_lbuttonup(hwnd);
                0
            } else {
                if !parent.is_null() {
                    (*parent).diagnostic_marquee_pending = false;
                }
                if let Some(prev) = prev_proc {
                    CallWindowProcW(prev, hwnd, message, w_param, l_param)
                } else {
                    DefWindowProcW(hwnd, message, w_param, l_param)
                }
            }
        }
        WM_CAPTURECHANGED => {
            handle_marquee_capturechanged(hwnd);
            0
        }
        _ => {
            if let Some(prev) = prev_proc {
                CallWindowProcW(prev, hwnd, message, w_param, l_param)
            } else {
                DefWindowProcW(hwnd, message, w_param, l_param)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_selection_and_rect_intersection_pure_logic() {
        let r1 = Rect {
            left: 10,
            top: 10,
            right: 50,
            bottom: 50,
        };
        let r2 = Rect {
            left: 20,
            top: 20,
            right: 30,
            bottom: 30,
        };
        let r3 = Rect {
            left: 60,
            top: 60,
            right: 80,
            bottom: 80,
        };
        assert!(rects_intersect(&r1, &r2));
        assert!(rects_intersect(&r2, &r1));
        assert!(!rects_intersect(&r1, &r3));

        let pt1 = Point { x: 50, y: 10 };
        let pt2 = Point { x: 10, y: 50 };
        let norm = points_to_rect(pt1, pt2);
        assert_eq!(norm.left, 10);
        assert_eq!(norm.top, 10);
        assert_eq!(norm.right, 50);
        assert_eq!(norm.bottom, 50);

        let row_rects = vec![
            Rect {
                left: 0,
                top: 0,
                right: 200,
                bottom: 20,
            },
            Rect {
                left: 0,
                top: 21,
                right: 200,
                bottom: 40,
            },
            Rect {
                left: 0,
                top: 41,
                right: 200,
                bottom: 60,
            },
            Rect {
                left: 0,
                top: 61,
                right: 200,
                bottom: 80,
            },
        ];

        let band = Rect {
            left: 10,
            top: 15,
            right: 50,
            bottom: 35,
        };
        let selected = marquee_selected_indices(&band, &row_rects, &[], false);
        assert_eq!(selected, vec![0, 1]);

        let selected_add = marquee_selected_indices(&band, &row_rects, &[3], true);
        assert_eq!(selected_add, vec![0, 1, 3]);

        let empty_band = Rect {
            left: 10,
            top: 100,
            right: 50,
            bottom: 120,
        };
        let selected_empty = marquee_selected_indices(&empty_band, &row_rects, &[], false);
        assert!(selected_empty.is_empty());
    }

    #[test]
    fn shift_click_bypasses_marquee_for_native_range_selection() {
        assert!(should_use_native_click_selection(true, false));
        assert!(should_use_native_click_selection(false, true));
        assert!(should_use_native_click_selection(true, true));
        assert!(!should_use_native_click_selection(false, false));

        let anchor = Point { x: 100, y: 100 };
        let still_inside = Point { x: 103, y: 102 };
        let crossed_x = Point { x: 104, y: 100 };
        let crossed_y = Point { x: 100, y: 104 };
        assert!(!marquee_drag_exceeded(anchor, still_inside, 4, 4));
        assert!(marquee_drag_exceeded(anchor, crossed_x, 4, 4));
        assert!(marquee_drag_exceeded(anchor, crossed_y, 4, 4));
    }

    #[test]
    fn marquee_requires_left_mouse_button_pressed() {
        assert!(marquee_requires_left_mouse_button_pressed_flag(MK_LBUTTON));
        assert!(marquee_requires_left_mouse_button_pressed_flag(
            MK_LBUTTON | 0x0008 | 0x0010
        ));
        assert!(!marquee_requires_left_mouse_button_pressed_flag(0));
        assert!(!marquee_requires_left_mouse_button_pressed_flag(0x0002));
        assert!(!marquee_requires_left_mouse_button_pressed_flag(
            0x0008 | 0x0010
        ));
    }
}
