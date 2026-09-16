use std::ffi::c_void;
use std::mem::size_of;

use crate::tray_surface::{
    dpi_to_icon_canvas, icon_pixel_color, tooltip_at, TimingValues, TrayStatus,
};
use tick_policy::PolicyReason;

pub const NIF_MESSAGE: u32 = 0x0001;
pub const NIF_ICON: u32 = 0x0002;
pub const NIF_TIP: u32 = 0x0004;
pub const NIM_ADD: u32 = 0x0000;
pub const NIM_DELETE: u32 = 0x0002;
pub const NIM_MODIFY: u32 = 0x0001;

pub const IDI_APPLICATION: usize = 32512;

#[repr(C)]
pub struct NotifyIconData {
    pub(crate) cb_size: u32,
    pub(crate) h_wnd: *mut c_void,
    pub(crate) u_id: u32,
    pub(crate) u_flags: u32,
    pub(crate) u_callback_message: u32,
    pub(crate) h_icon: *mut c_void,
    pub(crate) sz_tip: [u16; 128],
    pub(crate) dw_state: u32,
    pub(crate) dw_state_mask: u32,
    pub(crate) sz_info: [u16; 256],
    pub(crate) u_timeout_or_version: u32,
    pub(crate) sz_info_title: [u16; 64],
    pub(crate) dw_info_flags: u32,
    pub(crate) guid: [u8; 16],
    pub(crate) h_balloon_icon: *mut c_void,
}

impl Drop for NotifyIconData {
    fn drop(&mut self) {
        unsafe { destroy_icon(self.h_icon) };
        self.h_icon = std::ptr::null_mut();
    }
}

impl NotifyIconData {
    pub(crate) fn new(
        hwnd: *mut c_void,
        status: TrayStatus,
        timing: TimingValues,
        scheduled: Option<crate::pause::ScheduledAction>,
        block_reason: Option<PolicyReason>,
    ) -> Result<Self, u32> {
        let mut value = Self {
            cb_size: size_of::<Self>() as u32,
            h_wnd: hwnd,
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: super::WM_TRAY,
            h_icon: unsafe { status_icon(status, dpi_for_window(hwnd)) }?,
            sz_tip: [0; 128],
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0; 256],
            u_timeout_or_version: 0,
            sz_info_title: [0; 64],
            dw_info_flags: 0,
            guid: [0; 16],
            h_balloon_icon: std::ptr::null_mut(),
        };
        for (target, source) in value.sz_tip.iter_mut().zip(
            tooltip_at(
                status,
                timing,
                scheduled,
                std::time::Instant::now(),
                block_reason,
            )
            .encode_utf16(),
        ) {
            *target = source;
        }
        Ok(value)
    }
}

pub(crate) const fn dpi_to_canvas_size(dpi: u32) -> u32 {
    dpi_to_icon_canvas(dpi) as u32
}

pub(crate) unsafe fn status_icon(status: TrayStatus, dpi: u32) -> Result<*mut c_void, u32> {
    let color = icon_pixel_color(status);
    let canvas = dpi_to_canvas_size(dpi) as i32;
    let pixel_count = (canvas * canvas) as usize;
    let pixels = vec![color; pixel_count];
    let mask = vec![0u8; pixel_count / 8];
    let bitmap = CreateBitmap(canvas, canvas, 1, 32, pixels.as_ptr() as *const c_void);
    if bitmap.is_null() {
        return Err(GetLastError());
    }
    let mask_bitmap = CreateBitmap(canvas, canvas, 1, 1, mask.as_ptr() as *const c_void);
    if mask_bitmap.is_null() {
        let raw_error = GetLastError();
        DeleteObject(bitmap);
        return Err(raw_error);
    }
    let info = IconInfo {
        f_icon: 1,
        x_hotspot: 0,
        y_hotspot: 0,
        h_bm_mask: mask_bitmap,
        h_bm_color: bitmap,
    };
    let icon = CreateIconIndirect(&info);
    if icon.is_null() {
        let raw_error = GetLastError();
        DeleteObject(bitmap);
        DeleteObject(mask_bitmap);
        return Err(raw_error);
    }
    DeleteObject(bitmap);
    DeleteObject(mask_bitmap);
    Ok(icon)
}

pub(crate) fn dpi_for_window(window: *mut c_void) -> u32 {
    let dpi = unsafe { GetDpiForWindow(window) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

pub(crate) unsafe fn destroy_icon(icon: *mut c_void) {
    if !icon.is_null() {
        DestroyIcon(icon);
    }
}

pub(crate) fn update_icon(
    icon: &mut NotifyIconData,
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<crate::pause::ScheduledAction>,
    block_reason: Option<PolicyReason>,
) -> Result<(), u32> {
    let replacement = unsafe { status_icon(status, dpi_for_window(icon.h_wnd)) }?;
    let old_icon = icon.h_icon;
    let old_tip = icon.sz_tip;
    icon.h_icon = replacement;
    icon.sz_tip = [0; 128];
    for (target, source) in icon.sz_tip.iter_mut().zip(
        tooltip_at(
            status,
            timing,
            scheduled,
            std::time::Instant::now(),
            block_reason,
        )
        .encode_utf16(),
    ) {
        *target = source;
    }
    let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, icon) };
    let raw_error = if result == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    if matches!(
        super::native_bool_result(result, raw_error),
        super::NativeResult::Failed { .. }
    ) {
        icon.h_icon = old_icon;
        icon.sz_tip = old_tip;
        unsafe { destroy_icon(replacement) };
        Err(raw_error)
    } else {
        unsafe { destroy_icon(old_icon) };
        Ok(())
    }
}

#[repr(C)]
struct IconInfo {
    f_icon: i32,
    x_hotspot: u32,
    y_hotspot: u32,
    h_bm_mask: *mut c_void,
    h_bm_color: *mut c_void,
}

#[link(name = "user32")]
extern "system" {
    pub(crate) fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    pub(crate) fn GetDpiForWindow(window: *mut c_void) -> u32;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn DestroyIcon(icon: *mut c_void) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub(crate) fn GetLastError() -> u32;
}

#[link(name = "shell32")]
extern "system" {
    pub(crate) fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconData) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateBitmap(
        width: i32,
        height: i32,
        planes: u32,
        bits: u32,
        bits_data: *const c_void,
    ) -> *mut c_void;
    pub(crate) fn DeleteObject(object: *mut c_void) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_maps_to_requested_canvas_sizes_and_clamps_intermediate_values() {
        for (dpi, canvas) in [(96, 16), (120, 20), (144, 24), (192, 32), (384, 64)] {
            assert_eq!(dpi_to_canvas_size(dpi), canvas);
        }
        assert_eq!(dpi_to_canvas_size(0), 16);
        assert_eq!(dpi_to_canvas_size(108), 20);
        assert_eq!(dpi_to_canvas_size(168), 24);
        assert_eq!(dpi_to_canvas_size(175), 32);
        assert_eq!(dpi_to_canvas_size(288), 32);
        assert_eq!(dpi_to_canvas_size(289), 64);
    }
}
