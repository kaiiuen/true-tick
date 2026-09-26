use std::ffi::c_void;
use std::mem::size_of;
use std::sync::{Mutex, OnceLock};

use crate::tray_surface::{
    dpi_to_icon_canvas, icon_pixel_color, tooltip_at, TimingValues, TrayStatus,
};
use tick_policy::PolicyReason;

pub const NIF_MESSAGE: u32 = 0x0001;
pub const NIF_ICON: u32 = 0x0002;
pub const NIF_TIP: u32 = 0x0004;
#[allow(dead_code)]
pub const NIF_INFO: u32 = 0x0010;
pub const NIM_ADD: u32 = 0x0000;
pub const NIM_DELETE: u32 = 0x0002;
pub const NIM_MODIFY: u32 = 0x0001;

#[allow(dead_code)]
pub const NIIF_NONE: u32 = 0x0000;
#[allow(dead_code)]
pub const NIIF_INFO: u32 = 0x0001;
#[allow(dead_code)]
pub const NIIF_WARNING: u32 = 0x0002;
#[allow(dead_code)]
pub const NIIF_ERROR: u32 = 0x0003;

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

/// Cache key for one reusable tray icon: the rendered pixel color plus the
/// DPI derived canvas size. Icons that share a key render identical pixels,
/// so every state update that maps to the same key reuses one `HICON`
/// instead of rebuilding bitmaps through `CreateIconIndirect`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct IconCacheKey {
    color: u32,
    canvas: i32,
}

/// One cached icon entry. The cache owns the handle and hands out shared
/// references to `NotifyIconData` so repeated state updates never call
/// `CreateIconIndirect` again for an unchanged visual.
struct CachedIcon {
    key: IconCacheKey,
    handle: *mut c_void,
}

// `*mut c_void` handles are only touched on the tray thread behind the
// cache mutex, so sharing them across the process boundary is sound here.
unsafe impl Send for CachedIcon {}

/// Process wide tray icon cache. Entries are destroyed through
/// `DestroyIcon` by `clear_icon_cache` during shutdown so no GDI handle
/// leaks survive the tray lifetime.
static ICON_CACHE: OnceLock<Mutex<Vec<CachedIcon>>> = OnceLock::new();

fn icon_cache() -> &'static Mutex<Vec<CachedIcon>> {
    ICON_CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Looks up a cached icon for `key` without creating anything.
fn cached_icon_handle(key: IconCacheKey) -> Option<*mut c_void> {
    let cache = match icon_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    cache
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.handle)
}

/// Stores a freshly created icon handle for later reuse.
fn cache_icon_handle(key: IconCacheKey, handle: *mut c_void) {
    let mut cache = match icon_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !cache.iter().any(|entry| entry.key == key) {
        cache.push(CachedIcon { key, handle });
    }
}

/// Destroys every cached icon handle. Called once during tray shutdown so
/// no cached `HICON` leaks as a GDI object.
///
/// # Safety
///
/// The caller must guarantee no `NotifyIconData` still references a cached
/// handle, which holds because the tray icon is always removed with
/// `NIM_DELETE` before `clear_icon_cache` runs at shutdown.
pub(crate) unsafe fn clear_icon_cache() {
    let entries = {
        let mut cache = match icon_cache().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::mem::take(&mut *cache)
    };
    for entry in entries {
        destroy_icon(entry.handle);
    }
}

/// Returns a shared cached icon for the status and DPI, creating and
/// caching it on first use. The returned handle stays owned by the cache
/// and must never be destroyed by the caller.
pub(crate) unsafe fn shared_status_icon(status: TrayStatus, dpi: u32) -> Result<*mut c_void, u32> {
    let key = IconCacheKey {
        color: icon_pixel_color(status),
        canvas: dpi_to_canvas_size(dpi) as i32,
    };
    if let Some(handle) = cached_icon_handle(key) {
        return Ok(handle);
    }
    let handle = status_icon(status, dpi)?;
    cache_icon_handle(key, handle);
    Ok(handle)
}

impl Drop for NotifyIconData {
    fn drop(&mut self) {
        // SAFETY: dropping only clears our alias. Cached handles stay owned
        // by the icon cache and are destroyed once by `clear_icon_cache`.
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
            h_icon: unsafe { shared_status_icon(status, dpi_for_window(hwnd)) }?,
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

// A 1bpp bitmap scan line is padded to a 4 byte boundary per Windows
fn mask_buffer_len(canvas: i32) -> usize {
    (((canvas + 31) / 32) * 4) as usize * canvas as usize
}

pub(crate) unsafe fn status_icon(status: TrayStatus, dpi: u32) -> Result<*mut c_void, u32> {
    let color = icon_pixel_color(status);
    let canvas = dpi_to_canvas_size(dpi) as i32;
    let pixel_count = (canvas * canvas) as usize;
    let pixels = vec![color; pixel_count];
    let mask = vec![0u8; mask_buffer_len(canvas)];
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

/// Refreshes the tooltip text on a live tray registration without swapping
/// the icon handle. Used when the visual is unchanged so no `NIM_MODIFY`
/// icon work is needed beyond the text update.
fn refresh_tooltip_only(
    icon: &mut NotifyIconData,
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<crate::pause::ScheduledAction>,
    block_reason: Option<PolicyReason>,
) -> Result<(), u32> {
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
    // SAFETY: `icon` is a live tray registration and `Shell_NotifyIconW`
    // only reads the struct for the duration of the call.
    let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, icon) };
    let raw_error = if result == 0 {
        // SAFETY: called immediately on the same thread after a failed Win32
        // call, so the thread local last error is valid.
        unsafe { GetLastError() }
    } else {
        0
    };
    if matches!(
        super::native_bool_result(result, raw_error),
        super::NativeResult::Failed { .. }
    ) {
        Err(raw_error)
    } else {
        Ok(())
    }
}

pub(crate) fn update_icon(
    icon: &mut NotifyIconData,
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<crate::pause::ScheduledAction>,
    block_reason: Option<PolicyReason>,
) -> Result<(), u32> {
    // SAFETY: the cache owns the returned handle until `clear_icon_cache`
    // runs at shutdown, so aliasing it here never frees a live icon.
    let replacement = unsafe { shared_status_icon(status, dpi_for_window(icon.h_wnd)) }?;
    if replacement == icon.h_icon {
        return refresh_tooltip_only(icon, status, timing, scheduled, block_reason);
    }
    // Both handles are cache owned, so no per transition destroy runs here.
    // The retired handle stays alive in the cache for the next swap back.
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
    // SAFETY: `icon` is a live tray registration and `Shell_NotifyIconW`
    // only reads the struct for the duration of the call.
    let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, icon) };
    let raw_error = if result == 0 {
        // SAFETY: called immediately on the same thread after a failed Win32
        // call, so the thread local last error is valid.
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
        Err(raw_error)
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub(crate) fn truncate_utf16_into_buffer(buffer: &mut [u16], text: &str) {
    buffer.fill(0);
    if buffer.is_empty() {
        return;
    }
    let max_len = buffer.len() - 1;
    let mut written = 0;
    let mut encode_buf = [0u16; 2];
    for ch in text.chars() {
        let encoded = ch.encode_utf16(&mut encode_buf);
        if written + encoded.len() > max_len {
            break;
        }
        for &code_unit in encoded.iter() {
            buffer[written] = code_unit;
            written += 1;
        }
    }
}

#[allow(dead_code)]
pub(crate) fn show_balloon(
    icon: &mut NotifyIconData,
    title: &str,
    body: &str,
    warning: bool,
) -> Result<(), u32> {
    truncate_utf16_into_buffer(&mut icon.sz_info_title, title);
    truncate_utf16_into_buffer(&mut icon.sz_info, body);
    icon.dw_info_flags = if warning { NIIF_WARNING } else { NIIF_INFO };

    let previous_flags = icon.u_flags;
    icon.u_flags |= NIF_INFO;

    let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, icon) };
    let raw_error = if result == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };

    icon.u_flags = previous_flags;

    if matches!(
        super::native_bool_result(result, raw_error),
        super::NativeResult::Failed { .. }
    ) {
        Err(raw_error)
    } else {
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

    #[test]
    fn mask_buffer_len_pads_each_scan_line_to_four_bytes() {
        for (canvas, expected) in [(16, 64), (20, 80), (24, 96), (32, 128), (64, 512)] {
            assert_eq!(mask_buffer_len(canvas), expected);
        }
    }

    #[test]
    fn icon_cache_key_distinguishes_color_and_canvas() {
        let green_small = IconCacheKey {
            color: 0xFF00B000,
            canvas: 16,
        };
        assert_eq!(green_small, green_small);
        assert_ne!(
            green_small,
            IconCacheKey {
                color: 0xFFD00000,
                canvas: 16,
            }
        );
        assert_ne!(
            green_small,
            IconCacheKey {
                color: 0xFF00B000,
                canvas: 32,
            }
        );
    }

    #[test]
    fn icon_cache_key_covers_all_status_colors() {
        let running = IconCacheKey {
            color: icon_pixel_color(TrayStatus::Running),
            canvas: 16,
        };
        let stopped = IconCacheKey {
            color: icon_pixel_color(TrayStatus::Stopped),
            canvas: 16,
        };
        let degraded = IconCacheKey {
            color: icon_pixel_color(TrayStatus::Degraded),
            canvas: 16,
        };
        let error = IconCacheKey {
            color: icon_pixel_color(TrayStatus::Error),
            canvas: 16,
        };
        assert_ne!(running, stopped);
        assert_ne!(running, degraded);
        assert_ne!(running, error);
        assert_eq!(stopped, error);
    }

    #[test]
    fn title_truncation_respects_capacity_and_stays_valid_utf16() {
        let mut buffer = [0u16; 64];
        let input = "A".repeat(100);
        truncate_utf16_into_buffer(&mut buffer, &input);
        assert_eq!(buffer[63], 0);
        for &code_unit in &buffer[..63] {
            assert_eq!(code_unit, 'A' as u16);
        }
        let decoded = String::from_utf16(&buffer[..63]).expect("valid utf16");
        assert_eq!(decoded.len(), 63);
    }

    #[test]
    fn body_truncation_respects_capacity_and_stays_valid_utf16() {
        let mut buffer = [0u16; 256];
        let input = "B".repeat(300);
        truncate_utf16_into_buffer(&mut buffer, &input);
        assert_eq!(buffer[255], 0);
        for &code_unit in &buffer[..255] {
            assert_eq!(code_unit, 'B' as u16);
        }
        let decoded = String::from_utf16(&buffer[..255]).expect("valid utf16");
        assert_eq!(decoded.len(), 255);
    }

    #[test]
    fn non_bmp_character_at_truncation_boundary_is_dropped_whole() {
        let mut buffer = [0u16; 4];
        let input = "ab\u{1F600}";
        truncate_utf16_into_buffer(&mut buffer, input);
        assert_eq!(buffer[0], 'a' as u16);
        assert_eq!(buffer[1], 'b' as u16);
        assert_eq!(buffer[2], 0);
        assert_eq!(buffer[3], 0);

        let mut title_buffer = [0u16; 64];
        let mut title_input = "a".repeat(62);
        title_input.push('\u{1F600}');
        truncate_utf16_into_buffer(&mut title_buffer, &title_input);
        assert_eq!(title_buffer[62], 0);
        assert_eq!(title_buffer[63], 0);
        let decoded = String::from_utf16(&title_buffer[..62]).expect("valid utf16");
        assert_eq!(decoded, "a".repeat(62));
    }

    #[test]
    fn info_flag_is_cleared_from_u_flags_after_call_path() {
        let mut data = NotifyIconData {
            cb_size: size_of::<NotifyIconData>() as u32,
            h_wnd: std::ptr::null_mut(),
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: 0x8001,
            h_icon: std::ptr::null_mut(),
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

        let initial_flags = data.u_flags;
        assert_eq!(initial_flags & NIF_INFO, 0);

        let _ = show_balloon(&mut data, "Test Title", "Test Body", false);
        assert_eq!(data.u_flags, initial_flags);
        assert_eq!(data.u_flags & NIF_INFO, 0);
        assert_eq!(data.dw_info_flags, NIIF_INFO);

        let _ = show_balloon(&mut data, "Warning Title", "Warning Body", true);
        assert_eq!(data.u_flags, initial_flags);
        assert_eq!(data.u_flags & NIF_INFO, 0);
        assert_eq!(data.dw_info_flags, NIIF_WARNING);
    }
}
