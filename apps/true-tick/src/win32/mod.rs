//! Raw Win32 FFI declarations for the true-tick application.
//!
//! This module mirrors the binding style already used in `tray.rs`
//! and keeps every declaration free of external crate dependencies.

#![allow(dead_code)]

use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Rect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: i32,
    pub(crate) y: i32,
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MinMaxInfo {
    pub(crate) reserved: Point,
    pub(crate) maximum_size: Point,
    pub(crate) maximum_position: Point,
    pub(crate) minimum_track_size: Point,
    pub(crate) maximum_track_size: Point,
}

pub(crate) const WS_OVERLAPPED: u32 = 0x0000_0000;
pub(crate) const WS_POPUP: u32 = 0x8000_0000;
pub(crate) const WS_CHILD: u32 = 0x4000_0000;
pub(crate) const WS_MINIMIZE: u32 = 0x2000_0000;
pub(crate) const WS_VISIBLE: u32 = 0x1000_0000;
pub(crate) const WS_DISABLED: u32 = 0x0800_0000;
pub(crate) const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
pub(crate) const WS_CLIPCHILDREN: u32 = 0x0200_0000;
pub(crate) const WS_MAXIMIZE: u32 = 0x0100_0000;
pub(crate) const WS_CAPTION: u32 = 0x00C0_0000;
pub(crate) const WS_BORDER: u32 = 0x0080_0000;
pub(crate) const WS_DLGFRAME: u32 = 0x0040_0000;
pub(crate) const WS_VSCROLL: u32 = 0x0020_0000;
pub(crate) const WS_HSCROLL: u32 = 0x0010_0000;
pub(crate) const WS_SYSMENU: u32 = 0x0008_0000;
pub(crate) const WS_THICKFRAME: u32 = 0x0004_0000;
pub(crate) const WS_GROUP: u32 = 0x0002_0000;
pub(crate) const WS_TABSTOP: u32 = 0x0001_0000;
pub(crate) const WS_MINIMIZEBOX: u32 = 0x0002_0000;
pub(crate) const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
pub(crate) const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
pub(crate) const WS_POPUPWINDOW: u32 = 0x8088_0000;
pub(crate) const WS_CHILDWINDOW: u32 = 0x4000_0000;
pub(crate) const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub(crate) const WS_EX_APPWINDOW: u32 = 0x0004_0000;
pub(crate) const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub(crate) const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;

pub(crate) const WM_NULL: u32 = 0x0000;
pub(crate) const WM_CREATE: u32 = 0x0001;
pub(crate) const WM_DESTROY: u32 = 0x0002;
pub(crate) const WM_MOVE: u32 = 0x0003;
pub(crate) const WM_SIZE: u32 = 0x0005;
pub(crate) const WM_ACTIVATE: u32 = 0x0006;
pub(crate) const WM_SETFOCUS: u32 = 0x0007;
pub(crate) const WM_KILLFOCUS: u32 = 0x0008;
pub(crate) const WM_ENABLE: u32 = 0x000A;
pub(crate) const WM_SETREDRAW: u32 = 0x000B;
pub(crate) const WM_SETTEXT: u32 = 0x000C;
pub(crate) const WM_GETTEXT: u32 = 0x000D;
pub(crate) const WM_GETTEXTLENGTH: u32 = 0x000E;
pub(crate) const WM_PAINT: u32 = 0x000F;
pub(crate) const WM_CLOSE: u32 = 0x0010;
pub(crate) const WM_QUIT: u32 = 0x0012;
pub(crate) const WM_ERASEBKGND: u32 = 0x0014;
pub(crate) const WM_SYSCOLORCHANGE: u32 = 0x0015;
pub(crate) const WM_SHOWWINDOW: u32 = 0x0018;
pub(crate) const WM_SETTINGCHANGE: u32 = 0x001A;
pub(crate) const WM_GETMINMAXINFO: u32 = 0x0024;
pub(crate) const WM_SETFONT: u32 = 0x0030;
pub(crate) const WM_NCDESTROY: u32 = 0x0082;
pub(crate) const WM_COMMAND: u32 = 0x0111;
pub(crate) const WM_TIMER: u32 = 0x0113;
pub(crate) const WM_MENUSELECT: u32 = 0x011F;
pub(crate) const WM_CTLCOLOREDIT: u32 = 0x0133;
pub(crate) const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub(crate) const WM_MOUSEMOVE: u32 = 0x0200;
pub(crate) const WM_LBUTTONDOWN: u32 = 0x0201;
pub(crate) const WM_LBUTTONUP: u32 = 0x0202;
pub(crate) const WM_CAPTURECHANGED: u32 = 0x0215;
pub(crate) const WM_POWERBROADCAST: u32 = 0x0218;
pub(crate) const WM_DPICHANGED: u32 = 0x02E0;
pub(crate) const WM_THEMECHANGED: u32 = 0x031A;
pub(crate) const WM_USER: u32 = 0x0400;
pub(crate) const WM_APP: u32 = 0x8000;

pub(crate) const SW_HIDE: i32 = 0;
pub(crate) const SW_SHOWNORMAL: i32 = 1;
pub(crate) const SW_NORMAL: i32 = 1;
pub(crate) const SW_SHOWMINIMIZED: i32 = 2;
pub(crate) const SW_SHOWMAXIMIZED: i32 = 3;
pub(crate) const SW_MAXIMIZE: i32 = 3;
pub(crate) const SW_SHOWNOACTIVATE: i32 = 4;
pub(crate) const SW_SHOW: i32 = 5;
pub(crate) const SW_MINIMIZE: i32 = 6;
pub(crate) const SW_SHOWMINNOACTIVE: i32 = 7;
pub(crate) const SW_SHOWNA: i32 = 8;
pub(crate) const SW_RESTORE: i32 = 9;
pub(crate) const SW_SHOWDEFAULT: i32 = 10;
pub(crate) const SW_FORCEMINIMIZE: i32 = 11;

pub(crate) const SWP_NOSIZE: u32 = 0x0001;
pub(crate) const SWP_NOMOVE: u32 = 0x0002;
pub(crate) const SWP_NOZORDER: u32 = 0x0004;
pub(crate) const SWP_NOREDRAW: u32 = 0x0008;
pub(crate) const SWP_NOACTIVATE: u32 = 0x0010;
pub(crate) const SWP_FRAMECHANGED: u32 = 0x0020;
pub(crate) const SWP_SHOWWINDOW: u32 = 0x0040;
pub(crate) const SWP_HIDEWINDOW: u32 = 0x0080;
pub(crate) const SWP_NOCOPYBITS: u32 = 0x0100;
pub(crate) const SWP_NOOWNERZORDER: u32 = 0x0200;
pub(crate) const SWP_NOSENDCHANGING: u32 = 0x0400;
pub(crate) const SWP_DEFERERASE: u32 = 0x2000;
pub(crate) const SWP_ASYNCWINDOWPOS: u32 = 0x4000;

pub(crate) const COLOR_SCROLLBAR: i32 = 0;
pub(crate) const COLOR_BACKGROUND: i32 = 1;
pub(crate) const COLOR_ACTIVECAPTION: i32 = 2;
pub(crate) const COLOR_INACTIVECAPTION: i32 = 3;
pub(crate) const COLOR_MENU: i32 = 4;
pub(crate) const COLOR_WINDOW: i32 = 5;
pub(crate) const COLOR_WINDOWFRAME: i32 = 6;
pub(crate) const COLOR_MENUTEXT: i32 = 7;
pub(crate) const COLOR_WINDOWTEXT: i32 = 8;
pub(crate) const COLOR_CAPTIONTEXT: i32 = 9;
pub(crate) const COLOR_ACTIVEBORDER: i32 = 10;
pub(crate) const COLOR_INACTIVEBORDER: i32 = 11;
pub(crate) const COLOR_APPWORKSPACE: i32 = 12;
pub(crate) const COLOR_HIGHLIGHT: i32 = 13;
pub(crate) const COLOR_HIGHLIGHTTEXT: i32 = 14;
pub(crate) const COLOR_BTNFACE: i32 = 15;
pub(crate) const COLOR_BTNSHADOW: i32 = 16;
pub(crate) const COLOR_GRAYTEXT: i32 = 17;
pub(crate) const COLOR_BTNTEXT: i32 = 18;
pub(crate) const COLOR_INACTIVECAPTIONTEXT: i32 = 19;
pub(crate) const COLOR_BTNHIGHLIGHT: i32 = 20;

pub(crate) const GWL_WNDPROC: i32 = -4;
pub(crate) const GWL_HINSTANCE: i32 = -6;
pub(crate) const GWL_HWNDPARENT: i32 = -8;
pub(crate) const GWL_STYLE: i32 = -16;
pub(crate) const GWL_EXSTYLE: i32 = -20;
pub(crate) const GWL_USERDATA: i32 = -21;
pub(crate) const GWL_ID: i32 = -12;

pub(crate) const GWLP_WNDPROC: i32 = -4;
pub(crate) const GWLP_HINSTANCE: i32 = -6;
pub(crate) const GWLP_HWNDPARENT: i32 = -8;
pub(crate) const GWLP_USERDATA: i32 = -21;
pub(crate) const GWLP_ID: i32 = -12;

#[link(name = "user32")]
extern "system" {
    pub(crate) fn GetSystemMetrics(index: i32) -> i32;
    pub(crate) fn GetDpiForSystem() -> u32;
    pub(crate) fn GetDpiForWindow(window: *mut c_void) -> u32;
    pub(crate) fn AdjustWindowRectExForDpi(
        rect: *mut Rect,
        style: u32,
        menu: i32,
        ex_style: u32,
        dpi: u32,
    ) -> i32;
    pub(crate) fn SetWindowPos(
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    pub(crate) fn DeferWindowPos(
        defer: *mut c_void,
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> *mut c_void;
    pub(crate) fn BeginDeferWindowPos(number: i32) -> *mut c_void;
    pub(crate) fn EndDeferWindowPos(defer: *mut c_void) -> i32;
    pub(crate) fn InvalidateRect(window: *mut c_void, rect: *const Rect, erase: i32) -> i32;
    pub(crate) fn UpdateWindow(window: *mut c_void) -> i32;
    pub(crate) fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    pub(crate) fn DestroyWindow(window: *mut c_void) -> i32;
    pub(crate) fn IsWindow(window: *mut c_void) -> i32;
    pub(crate) fn IsWindowVisible(hwnd: *mut c_void) -> i32;
}
