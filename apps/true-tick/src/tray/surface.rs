use std::ffi::c_void;

use tick_diagnostics::DiagnosticOutcome;

use crate::tray::controller::App;
use crate::tray::icon::{clear_icon_cache, NotifyIconData, Shell_NotifyIconW, NIM_ADD, NIM_DELETE};
use crate::tray::native::{GetLastError, NativeResult};

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn remove_tray_icon(app: &mut App) {
    if let Some(mut icon) = app.tray_icon.take() {
        // SAFETY: `icon` is the live tray registration owned by the app, so
        // passing it to `NIM_DELETE` unregisters exactly this icon.
        let result = Shell_NotifyIconW(NIM_DELETE, &mut icon);
        app.record(
            "native.Shell_NotifyIconW.delete",
            format!(
                "result={} raw_status={}",
                result != 0,
                // SAFETY: `GetLastError` runs immediately on the same thread
                // after the delete call, so the value belongs to this call.
                if result == 0 { GetLastError() } else { 0 }
            ),
        );
    }
    // SAFETY: the tray registration above is gone, so no `NotifyIconData`
    // aliases a cached handle and every cached `HICON` can be destroyed.
    clear_icon_cache();
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn restore_tray_icon(hwnd: *mut c_void, app: &mut App) {
    app.record("tray.taskbar_created", "action=restore");
    let mut icon = match NotifyIconData::new(
        hwnd,
        app.lifecycle_status(),
        app.timing_values(),
        app.pause.current(),
        app.last_block_reason,
    ) {
        Ok(icon) => icon,
        Err(raw_error) => {
            app.record_with_outcome(
                "native.tray_icon.create.error",
                format!("raw_status={raw_error}"),
                DiagnosticOutcome::Failed,
            );
            // The surface is gone, so the tier machine must see the icon as
            // absent instead of trusting a stale registration.
            app.tray_icon = None;
            publish_tray_icon(app);
            return;
        }
    };
    let add_result = Shell_NotifyIconW(NIM_ADD, &mut icon);
    let add_error = if add_result == 0 { GetLastError() } else { 0 };
    if matches!(
        crate::tray::native::native_bool_result(add_result, add_error),
        NativeResult::Failed { .. }
    ) {
        app.record_with_outcome(
            "native.Shell_NotifyIconW.add.error",
            format!("raw_status={add_error}"),
            DiagnosticOutcome::Failed,
        );
        app.tray_icon = None;
        publish_tray_icon(app);
        return;
    }
    app.tray_icon = Some(icon);
    app.last_publication = None;
    publish_tray_icon(app);
}

pub(crate) fn publish_tray_icon(app: &mut App) {
    app.publish();
}
