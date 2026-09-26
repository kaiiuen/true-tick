use std::ffi::c_void;

use tick_diagnostics::DiagnosticOutcome;

use crate::config;
use crate::pause::DurationPreset;
use crate::tray::{
    app_create_params, destroy_created_diagnostic_controls, diagnostic_create_failure_result,
    scale_logical, set_diagnostic_control_font, wide, App, BeginPaint, CreateStruct,
    CreateWindowExW, DefWindowProcW, DestroyWindow, EndPaint, FillRect, GetClientRect,
    GetDpiForSystem, GetDpiForWindow, GetLastError, GetModuleHandleW, GetSysColor,
    GetSysColorBrush, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow,
    PaintStruct, Rect, SendMessageW, SetBkColor, SetForegroundWindow, SetTextColor,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, UpdateWindow, BN_CLICKED,
    BS_PUSHBUTTON, COLOR_WINDOW, COLOR_WINDOWTEXT, EM_LIMITTEXT, EN_CHANGE, ES_AUTOHSCROLL,
    GWLP_USERDATA, SS_LEFT, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SW_SHOWNORMAL, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_NCDESTROY, WM_PAINT, WM_SIZE, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

pub const PRESETS_WINDOW_TITLE: &str = "True™ Tick Interval Presets";
pub const PRESETS_WINDOW_WIDTH: i32 = 380;
pub const PRESETS_WINDOW_HEIGHT: i32 = 360;
pub const PRESETS_WINDOW_CLASS: &str = "TrueTickPresetsClass";

const ID_PRESETS_LISTBOX: usize = 1301;
const ID_PRESETS_LABEL: usize = 1302;
const ID_PRESETS_INPUT: usize = 1303;
const ID_PRESETS_ADD: usize = 1304;
const ID_PRESETS_DELETE: usize = 1305;
const ID_PRESETS_RESET: usize = 1306;
const ID_PRESETS_CLOSE: usize = 1307;

const LBS_NOTIFY: u32 = 0x0001;
const LBN_SELCHANGE: usize = 1;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_GETCURSEL: u32 = 0x0188;
const LB_ERR: isize = -1;
const WS_OVERLAPPED: u32 = 0x00000000;
const WS_CAPTION: u32 = 0x00C00000;
const WS_SYSMENU: u32 = 0x00080000;

const PRESETS_MARGIN: i32 = 10;
const PRESETS_LIST_HEIGHT: i32 = 168;
const PRESETS_LABEL_HEIGHT: i32 = 18;
const PRESETS_INPUT_HEIGHT: i32 = 24;
const PRESETS_BUTTON_HEIGHT: i32 = 26;
const PRESETS_ROW_GAP: i32 = 8;
const PRESETS_BUTTON_GAP: i32 = 8;
const PRESETS_INPUT_LIMIT: usize = 64;

fn persist_presets(app: &mut App) -> bool {
    let mut next = app.config.clone();
    next.schedule_presets_seconds = app.presets_manager.to_seconds_list();
    if let Err(error) = config::save_atomic(&app.config_path, &next) {
        app.record_with_outcome(
            "config.save.result",
            format!("result=error setting=schedule_presets_seconds error={error}"),
            DiagnosticOutcome::Failed,
        );
        return false;
    }
    app.record(
        "config.save.result",
        "result=success setting=schedule_presets_seconds",
    );
    app.config = next;
    true
}

unsafe fn populate_presets_list(app: &App) {
    let Some(listbox) = app.presets_listbox else {
        return;
    };
    let _ = SendMessageW(listbox, LB_RESETCONTENT, 0, 0);
    for preset in app.presets_manager.presets() {
        let label = wide(&preset.format_label());
        let _ = SendMessageW(listbox, LB_ADDSTRING, 0, label.as_ptr() as isize);
    }
}

unsafe fn presets_input_text(input: *mut c_void) -> String {
    let length = GetWindowTextLengthW(input);
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(input, buffer.as_mut_ptr(), buffer.len() as i32);
    if copied <= 0 {
        return String::new();
    }
    buffer.truncate(copied as usize);
    String::from_utf16_lossy(&buffer)
}

unsafe fn presets_add(app: &mut App) {
    let Some(input) = app.presets_input else {
        app.record_with_outcome(
            "presets.add",
            "result=failed reason=input_missing",
            DiagnosticOutcome::Failed,
        );
        return;
    };
    let text = presets_input_text(input);
    match DurationPreset::parse(&text) {
        Ok(preset) => match app.presets_manager.add(preset) {
            Ok(()) => {
                app.record(
                    "presets.add",
                    format!("result=added seconds={}", preset.seconds()),
                );
                persist_presets(app);
                populate_presets_list(app);
                let empty = wide("");
                let _ = SetWindowTextW(input, empty.as_ptr());
            }
            Err(error) => {
                app.record_with_outcome(
                    "presets.add",
                    format!("result=rejected error={error}"),
                    DiagnosticOutcome::Failed,
                );
            }
        },
        Err(error) => {
            app.record_with_outcome(
                "presets.add",
                format!("result=invalid error={error}"),
                DiagnosticOutcome::Failed,
            );
        }
    }
}

unsafe fn presets_delete_selected(app: &mut App) {
    let Some(listbox) = app.presets_listbox else {
        app.record_with_outcome(
            "presets.delete",
            "result=failed reason=listbox_missing",
            DiagnosticOutcome::Failed,
        );
        return;
    };
    let selection = SendMessageW(listbox, LB_GETCURSEL, 0, 0);
    if selection == LB_ERR || selection < 0 {
        app.record_with_outcome(
            "presets.delete",
            "result=rejected reason=no_selection",
            DiagnosticOutcome::Suppressed,
        );
        return;
    }
    match app.presets_manager.remove(selection as usize) {
        Ok(()) => {
            app.record(
                "presets.delete",
                format!("result=removed index={selection}"),
            );
            persist_presets(app);
            populate_presets_list(app);
        }
        Err(error) => {
            app.record_with_outcome(
                "presets.delete",
                format!("result=rejected error={error}"),
                DiagnosticOutcome::Failed,
            );
        }
    }
}

unsafe fn presets_reset(app: &mut App) {
    app.presets_manager.reset_defaults();
    app.record("presets.reset", "result=restored defaults=factory");
    persist_presets(app);
    populate_presets_list(app);
}

unsafe fn presets_children_ready(app: &App) -> bool {
    [app.presets_listbox, app.presets_input]
        .iter()
        .all(|control| control.is_some_and(|hwnd| !hwnd.is_null() && IsWindow(hwnd) != 0))
}

pub unsafe fn open_presets_window(app: &mut App) -> bool {
    if let Some(window) = app.presets_window {
        if IsWindow(window) != 0 {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            } else {
                ShowWindow(window, SW_SHOWNORMAL);
            }
            UpdateWindow(window);
            SetForegroundWindow(window);
            app.record("presets.window.result", "result=focused_existing");
            return true;
        } else {
            app.presets_window = None;
            app.presets_listbox = None;
            app.presets_input = None;
        }
    }
    let class_name = wide(PRESETS_WINDOW_CLASS);
    let title = wide(PRESETS_WINDOW_TITLE);
    let dpi = GetDpiForSystem().max(96);
    let window = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        140,
        140,
        scale_logical(PRESETS_WINDOW_WIDTH, dpi),
        scale_logical(PRESETS_WINDOW_HEIGHT, dpi),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        app as *mut App as *mut c_void,
    );
    if window.is_null() {
        app.record_with_outcome(
            "presets.window.result",
            format!("result=create_failed raw_status={}", GetLastError()),
            DiagnosticOutcome::Failed,
        );
        return false;
    }
    app.presets_window = Some(window);
    if !presets_children_ready(app) {
        app.record(
            "presets.window.invalid",
            "result=destroyed reason=create_returned_without_required_children",
        );
        let _ = DestroyWindow(window);
        app.presets_window = None;
        app.presets_listbox = None;
        app.presets_input = None;
        return false;
    }
    ShowWindow(window, SW_SHOWNORMAL);
    UpdateWindow(window);
    SetForegroundWindow(window);
    app.record("presets.window.result", "result=opened");
    true
}

pub unsafe extern "system" fn presets_window_proc(
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if message == WM_CREATE {
        let create = l_param as *const CreateStruct;
        let app_ptr = app_create_params(create);
        if app_ptr.is_null() {
            return diagnostic_create_failure_result();
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        let app = app_ptr as *mut App;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let margin = scale_logical(PRESETS_MARGIN, dpi);
        let row_gap = scale_logical(PRESETS_ROW_GAP, dpi);
        let list_height = scale_logical(PRESETS_LIST_HEIGHT, dpi);
        let label_height = scale_logical(PRESETS_LABEL_HEIGHT, dpi);
        let input_height = scale_logical(PRESETS_INPUT_HEIGHT, dpi);
        let button_height = scale_logical(PRESETS_BUTTON_HEIGHT, dpi);
        let button_gap = scale_logical(PRESETS_BUTTON_GAP, dpi);
        let mut client = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let client_width = if GetClientRect(hwnd, &mut client) != 0 {
            (client.right - client.left).max(margin * 2 + button_gap * 3 + 4)
        } else {
            scale_logical(PRESETS_WINDOW_WIDTH - 16, dpi)
        };
        let content_width = client_width - margin * 2;
        let instance = GetModuleHandleW(std::ptr::null());
        let mut top = margin;
        let listbox = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("LISTBOX").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_TABSTOP | LBS_NOTIFY,
            margin,
            top,
            content_width,
            list_height,
            hwnd,
            ID_PRESETS_LISTBOX as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        top += list_height + row_gap;
        let label = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("New interval, for example 15m or 1h 30m").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            margin,
            top,
            content_width,
            label_height,
            hwnd,
            ID_PRESETS_LABEL as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        top += label_height + 2;
        let input = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
            margin,
            top,
            content_width,
            input_height,
            hwnd,
            ID_PRESETS_INPUT as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        if !input.is_null() {
            SendMessageW(input, EM_LIMITTEXT, PRESETS_INPUT_LIMIT, 0);
        }
        top += input_height + row_gap;
        let button_width = (content_width - button_gap * 3).max(4) / 4;
        let button_class = wide("BUTTON");
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON;
        let add = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Add").as_ptr(),
            button_style,
            margin,
            top,
            button_width,
            button_height,
            hwnd,
            ID_PRESETS_ADD as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        let delete = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Delete").as_ptr(),
            button_style,
            margin + button_width + button_gap,
            top,
            button_width,
            button_height,
            hwnd,
            ID_PRESETS_DELETE as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        let reset = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Reset").as_ptr(),
            button_style,
            margin + (button_width + button_gap) * 2,
            top,
            button_width,
            button_height,
            hwnd,
            ID_PRESETS_RESET as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        let close = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Close").as_ptr(),
            button_style,
            margin + (button_width + button_gap) * 3,
            top,
            button_width,
            button_height,
            hwnd,
            ID_PRESETS_CLOSE as *mut c_void,
            instance,
            std::ptr::null_mut(),
        );
        if listbox.is_null()
            || label.is_null()
            || input.is_null()
            || add.is_null()
            || delete.is_null()
            || reset.is_null()
            || close.is_null()
        {
            if !app_ptr.is_null() {
                (*app).diagnostics.record_with_outcome(
                    "native.CreateWindowExW.presets_control.error",
                    format!(
                        "listbox_null={} label_null={} input_null={} add_null={} delete_null={} reset_null={} close_null={} raw_status={}",
                        listbox.is_null(),
                        label.is_null(),
                        input.is_null(),
                        add.is_null(),
                        delete.is_null(),
                        reset.is_null(),
                        close.is_null(),
                        GetLastError()
                    )
                , DiagnosticOutcome::Failed);
            }
            destroy_created_diagnostic_controls(&[
                listbox,
                label,
                input,
                add,
                delete,
                reset,
                close,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ]);
            return diagnostic_create_failure_result();
        }
        for control in [listbox, label, input, add, delete, reset, close] {
            set_diagnostic_control_font(control);
        }
        (*app).presets_listbox = Some(listbox);
        (*app).presets_input = Some(input);
        populate_presets_list(&*app);
        return 0;
    }
    if app.is_null() {
        return DefWindowProcW(hwnd, message, w_param, l_param);
    }
    if message == WM_ERASEBKGND {
        let brush = GetSysColorBrush(COLOR_WINDOW);
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if !brush.is_null() && GetClientRect(hwnd, &mut rect) != 0 {
            let _ = FillRect(w_param as *mut c_void, &rect, brush);
        }
        return 1;
    }
    if message == WM_PAINT {
        let mut paint = PaintStruct {
            hdc: std::ptr::null_mut(),
            erase: 0,
            paint: Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            restore: 0,
            inc_update: 0,
            reserved: [0; 32],
        };
        let hdc = BeginPaint(hwnd, &mut paint);
        if !hdc.is_null() {
            let brush = GetSysColorBrush(COLOR_WINDOW);
            let _ = FillRect(hdc, &paint.paint, brush);
        }
        let _ = EndPaint(hwnd, &paint);
        return 0;
    }
    if message == WM_CTLCOLOREDIT || message == WM_CTLCOLORSTATIC {
        let hdc = w_param as *mut c_void;
        let brush = GetSysColorBrush(COLOR_WINDOW);
        let _ = SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
        let _ = SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
        return brush as isize;
    }
    if message == WM_COMMAND {
        let command = w_param & 0xffff;
        let notification = (w_param >> 16) & 0xffff;
        if notification == BN_CLICKED {
            match command {
                ID_PRESETS_ADD => {
                    presets_add(&mut *app);
                    return 0;
                }
                ID_PRESETS_DELETE => {
                    presets_delete_selected(&mut *app);
                    return 0;
                }
                ID_PRESETS_RESET => {
                    presets_reset(&mut *app);
                    return 0;
                }
                ID_PRESETS_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return 0;
                }
                _ => {}
            }
        }
        if command == ID_PRESETS_LISTBOX && notification == LBN_SELCHANGE {
            let selection = app
                .as_ref()
                .and_then(|app| app.presets_listbox)
                .map_or(LB_ERR, |listbox| SendMessageW(listbox, LB_GETCURSEL, 0, 0));
            (*app).record("presets.selection", format!("index={selection}"));
            return 0;
        }
        if command == ID_PRESETS_INPUT && notification == EN_CHANGE {
            return 0;
        }
    }
    if message == WM_SIZE {
        return 0;
    }
    if message == WM_DPICHANGED {
        let suggested = l_param as *const Rect;
        if !suggested.is_null() {
            let width = ((*suggested).right - (*suggested).left).max(0);
            let height = ((*suggested).bottom - (*suggested).top).max(0);
            let _ = SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                (*suggested).left,
                (*suggested).top,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        return 0;
    }
    if message == WM_CLOSE {
        DestroyWindow(hwnd);
        return 0;
    }
    if message == WM_DESTROY {
        (*app).presets_window = None;
        (*app).presets_listbox = None;
        (*app).presets_input = None;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    if message == WM_NCDESTROY {
        (*app).presets_window = None;
        (*app).presets_listbox = None;
        (*app).presets_input = None;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::tests::test_app;
    use std::sync::Arc;
    use tick_diagnostics::DiagnosticStore;

    #[test]
    fn open_presets_window_recovers_from_invalid_handle() {
        let store = Arc::new(DiagnosticStore::new(16));
        let mut app = test_app(store.clone());
        let invalid = 0xdead_beef as *mut c_void;
        app.presets_window = Some(invalid);
        app.presets_listbox = Some(invalid);
        app.presets_input = Some(invalid);
        let result = unsafe { open_presets_window(&mut app) };
        if let Some(window) = app.presets_window {
            assert_ne!(window, invalid);
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        if result {
            assert!(app.presets_window.is_some());
        }
        assert_ne!(app.presets_listbox, Some(invalid));
        assert_ne!(app.presets_input, Some(invalid));
    }
}
