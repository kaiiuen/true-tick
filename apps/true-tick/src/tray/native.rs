pub(crate) use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub(crate) struct Rect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

#[repr(C)]
pub(crate) struct ToolInfo {
    pub(crate) cb_size: u32,
    pub(crate) flags: u32,
    pub(crate) hwnd: *mut c_void,
    pub(crate) id: usize,
    pub(crate) rect: Rect,
    pub(crate) instance: *mut c_void,
    pub(crate) text: *const u16,
    pub(crate) l_param: isize,
    pub(crate) reserved: *mut c_void,
}

#[repr(C)]
pub(crate) struct CreateStruct {
    pub(crate) create_params: *mut c_void,
    pub(crate) instance: *mut c_void,
    pub(crate) menu: *mut c_void,
    pub(crate) parent: *mut c_void,
    pub(crate) height: i32,
    pub(crate) width: i32,
    pub(crate) y: i32,
    pub(crate) x: i32,
    pub(crate) style: u32,
    pub(crate) name: *const u16,
    pub(crate) class_name: *const u16,
    pub(crate) extended_style: u32,
}

pub(crate) fn app_create_params(create: *const CreateStruct) -> *mut c_void {
    if create.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe {
            // SAFETY: create is the non-null CREATESTRUCT pointer the OS passes with window creation
            (*create).create_params
        }
    }
}

#[repr(C)]
pub(crate) struct PaintStruct {
    pub(crate) hdc: *mut c_void,
    pub(crate) erase: i32,
    pub(crate) paint: Rect,
    pub(crate) restore: i32,
    pub(crate) inc_update: i32,
    pub(crate) reserved: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PopupMenuHandles {
    pub(crate) root: *mut c_void,
    pub(crate) schedule: Option<*mut c_void>,
    pub(crate) settings: Option<*mut c_void>,
    pub(crate) status: Option<*mut c_void>,
}

#[repr(C)]
pub(crate) struct AppBarData {
    pub(crate) cb_size: u32,
    pub(crate) hwnd: *mut c_void,
    pub(crate) callback_message: u32,
    pub(crate) edge: u32,
    pub(crate) rect: Rect,
    pub(crate) l_param: isize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeResult {
    Succeeded,
    Failed { raw_error: u32 },
}

pub(crate) fn native_bool_result(result: i32, raw_error: u32) -> NativeResult {
    if result == 0 {
        NativeResult::Failed { raw_error }
    } else {
        NativeResult::Succeeded
    }
}

pub(crate) fn native_handle_result(is_null: bool, raw_error: u32) -> NativeResult {
    if is_null {
        NativeResult::Failed { raw_error }
    } else {
        NativeResult::Succeeded
    }
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
    let mut buffer = Vec::with_capacity(value.len() + 1);
    buffer.extend(value.encode_utf16());
    buffer.push(0);
    buffer
}

pub(crate) fn scale_logical(value: i32, dpi: u32) -> i32 {
    value.saturating_mul(dpi as i32).saturating_add(95) / 96
}

pub(crate) const fn diagnostic_create_failure_result() -> isize {
    -1
}

pub(crate) unsafe fn set_diagnostic_control_font(control: *mut c_void) {
    if control.is_null() {
        return;
    }
    let font = GetStockObject(super::DEFAULT_GUI_FONT);
    if !font.is_null() {
        let _ = SendMessageW(control, super::WM_SETFONT, font as usize, 1);
    }
}

pub(crate) unsafe fn destroy_created_diagnostic_controls(controls: &[*mut c_void]) {
    for &control in controls {
        if !control.is_null() {
            let _ = DestroyWindow(control);
        }
    }
}

pub(crate) unsafe fn show_shutdown_warning(hwnd: *mut c_void, message: &str) {
    let text = wide(message);
    let title = wide("True™ Tick shutdown warning");
    MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), super::MB_ICONWARNING);
}

pub(crate) fn show_unclean_shutdown_warning() {
    let text = wide(
        "True Tick did not shut down cleanly last time. The previous session may have ended from a force quit or a system crash.",
    );
    let title = wide("True Tick");
    std::thread::spawn(move || unsafe {
        // SAFETY: all string pointers reference live nul-terminated buffers and the owner is null or a live window
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            super::MB_OK | super::MB_ICONWARNING | super::MB_SETFOREGROUND,
        );
    });
}

pub(crate) fn taskbar_edge() -> Option<u32> {
    let mut data = AppBarData {
        cb_size: std::mem::size_of::<AppBarData>() as u32,
        hwnd: std::ptr::null_mut(),
        callback_message: 0,
        edge: 0,
        rect: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        l_param: 0,
    };
    let result = unsafe {
        // SAFETY: data is a valid stack AppBarData with cb_size set
        SHAppBarMessage(super::ABM_GETTASKBARPOS, &mut data)
    };
    if result == 0 {
        None
    } else {
        Some(data.edge)
    }
}

pub(crate) const fn popup_track_flags(edge: Option<u32>, horizontal: u32, vertical: u32) -> u32 {
    let horizontal = match edge {
        Some(super::ABE_RIGHT) => super::menu::TPM_RIGHTALIGN,
        _ => horizontal,
    };
    let vertical = match edge {
        Some(super::ABE_BOTTOM) => super::menu::TPM_BOTTOMALIGN,
        _ => vertical,
    };
    horizontal | vertical
}

#[link(name = "user32")]
extern "system" {
    pub(crate) fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> *mut c_void;
    pub(crate) fn DefWindowProcW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    pub(crate) fn CallWindowProcW(
        prev_wnd_proc: unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize,
        hwnd: *mut c_void,
        message: u32,
        w: usize,
        l: isize,
    ) -> isize;
    pub(crate) fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
    pub(crate) fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    pub(crate) fn IsWindow(window: *mut c_void) -> i32;
    pub(crate) fn IsIconic(window: *mut c_void) -> i32;
    pub(crate) fn PostQuitMessage(code: i32);
    pub(crate) fn SetTimer(
        hwnd: *mut c_void,
        event_id: usize,
        interval_ms: u32,
        callback: *mut c_void,
    ) -> usize;
    pub(crate) fn KillTimer(hwnd: *mut c_void, event_id: usize) -> i32;
    pub(crate) fn MessageBoxW(
        hwnd: *mut c_void,
        text: *const u16,
        title: *const u16,
        flags: u32,
    ) -> i32;
    pub(crate) fn CreatePopupMenu() -> *mut c_void;
    pub(crate) fn GetMenuItemCount(menu: *mut c_void) -> i32;
    pub(crate) fn GetMenuStringW(
        menu: *mut c_void,
        item: usize,
        text: *mut u16,
        maximum: i32,
        flags: u32,
    ) -> i32;
    pub(crate) fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
    pub(crate) fn ModifyMenuW(
        menu: *mut c_void,
        item: usize,
        flags: u32,
        new_item: usize,
        text: *const u16,
    ) -> i32;
    pub(crate) fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    pub(crate) fn TrackPopupMenu(
        menu: *mut c_void,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: *mut c_void,
        rect: *const c_void,
    ) -> i32;
    pub(crate) fn DestroyMenu(menu: *mut c_void) -> i32;
    pub(crate) fn DestroyWindow(window: *mut c_void) -> i32;
    pub(crate) fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    pub(crate) fn UpdateWindow(window: *mut c_void) -> i32;
    pub(crate) fn BeginPaint(window: *mut c_void, paint: *mut PaintStruct) -> *mut c_void;
    pub(crate) fn EndPaint(window: *mut c_void, paint: *const PaintStruct) -> i32;
    pub(crate) fn GetSysColor(index: i32) -> u32;
    pub(crate) fn GetSysColorBrush(index: i32) -> *mut c_void;
    pub(crate) fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
    pub(crate) fn SetBkColor(hdc: *mut c_void, color: u32) -> u32;

    pub(crate) fn SetWindowTextW(window: *mut c_void, text: *const u16) -> i32;
    pub(crate) fn GetWindowTextLengthW(window: *mut c_void) -> i32;
    pub(crate) fn GetWindowTextW(window: *mut c_void, text: *mut u16, maximum: i32) -> i32;
    pub(crate) fn GetKeyState(key: i32) -> i16;
    pub(crate) fn GetClientRect(window: *mut c_void, rect: *mut Rect) -> i32;
    pub(crate) fn SendMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    pub(crate) fn SetWindowPos(
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    pub(crate) fn InvalidateRect(window: *mut c_void, rect: *const Rect, erase: i32) -> i32;
    pub(crate) fn GetCursorPos(point: *mut Point) -> i32;
    pub(crate) fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    pub(crate) fn GetDpiForSystem() -> u32;
    pub(crate) fn GetSystemMetrics(index: i32) -> i32;
    pub(crate) fn FillRect(hdc: *mut c_void, rect: *const Rect, brush: *mut c_void) -> i32;
    pub(crate) fn DrawFocusRect(hdc: *mut c_void, rect: *const Rect) -> i32;
    pub(crate) fn GetDC(hwnd: *mut c_void) -> *mut c_void;
    pub(crate) fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> i32;
    pub(crate) fn SetCapture(hwnd: *mut c_void) -> *mut c_void;
    pub(crate) fn ReleaseCapture() -> i32;
    pub(crate) fn GetCapture() -> *mut c_void;
}

#[link(name = "shell32")]
extern "system" {
    pub(crate) fn ShellExecuteW(
        hwnd: *mut c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show_command: i32,
    ) -> *mut c_void;
    pub(crate) fn SHAppBarMessage(message: usize, data: *mut AppBarData) -> usize;
}

#[link(name = "gdi32")]
extern "system" {
    pub(crate) fn GetStockObject(index: i32) -> *mut c_void;
}

#[link(name = "kernel32")]
extern "system" {
    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn ExitProcess(exit_code: u32) -> !;
}
