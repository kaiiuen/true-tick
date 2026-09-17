use std::ffi::c_void;

use crate::pause::DurationAction;
use crate::tray_surface::{
    duration_command, duration_menu_items, menu_action_keeps_open,
    menu_command_is_enabled_with_pause, menu_description, menu_items_with_duration, preset_command,
    preset_command_base, preset_menu_items, settings_menu_items, status_menu_items,
    CANCEL_PAUSE_COMMAND_ID, CANCEL_SCHEDULED_COMMAND_ID, GITHUB_COMMAND_ID, LOGS_COMMAND_ID,
    PAUSE_FOR_15_COMMAND_ID, PAUSE_FOR_30_COMMAND_ID, PAUSE_FOR_5_COMMAND_ID,
    PAUSE_FOR_60_COMMAND_ID, SCHEDULE_PRESETS_COMMAND_ID, START_IN_15_COMMAND_ID,
    START_IN_1_COMMAND_ID, START_IN_30_COMMAND_ID, START_IN_5_COMMAND_ID, START_IN_60_COMMAND_ID,
    STOP_IN_15_COMMAND_ID, STOP_IN_1_COMMAND_ID, STOP_IN_30_COMMAND_ID, STOP_IN_5_COMMAND_ID,
    STOP_IN_60_COMMAND_ID,
};
use crate::win32::DestroyWindow;

use super::{
    cancel_scheduled_action, manual_start, manual_stop, native_bool_result, open_github_page,
    quit_decision, schedule_duration_action, schedule_preset_action, set_auto_resume,
    set_automatic, set_startup, show_quit_warning, show_shutdown_warning, wide, App, AppendMenuW,
    CreatePopupMenu, CreateWindowExW, DestroyMenu, DiagnosticOutcome, DiagnosticSource,
    GetCursorPos, GetLastError, GetMenuItemCount, GetMenuStringW, GetModuleHandleW,
    GetSystemMetrics, KillTimer, MessageBoxW, ModifyMenuW, NativeResult, Point, PopupMenuHandles,
    PostQuitMessage, QuitDecision, QuitDialogDecision, Rect, SendMessageW, SetForegroundWindow,
    SetTimer, ToolInfo, TrackPopupMenu, ID_AUTOMATIC_OFF, ID_AUTOMATIC_ON, ID_AUTO_RESUME_OFF,
    ID_AUTO_RESUME_ON, ID_QUIT, ID_START, ID_STARTUP_OFF, ID_STARTUP_ON, ID_STOP, MB_ICONWARNING,
    POPUP_REFRESH_INTERVAL_MS, POPUP_REFRESH_TIMER_ID, TTF_ABSOLUTE, TTF_IDISHWND, TTF_TRACK,
    TTM_ADDTOOLW, TTM_TRACKACTIVATE, TTM_TRACKPOSITION, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTS_NOPREFIX, WS_EX_TOPMOST, WS_POPUP,
};

pub(crate) const TPM_LEFTALIGN: u32 = 0x0000;
pub(crate) const TPM_TOPALIGN: u32 = 0x0000;
pub(crate) const TPM_RIGHTBUTTON: u32 = 0x0002;
pub(crate) const TPM_RETURNCMD: u32 = 0x0100;
pub(crate) const SM_CXWORKAREA: i32 = 60;
pub(crate) const SM_CYWORKAREA: i32 = 61;
pub(crate) const MF_STRING: u32 = 0x0000;
pub(crate) const MF_SEPARATOR: u32 = 0x0800;
pub(crate) const MF_GRAYED: u32 = 0x0001;
pub(crate) const MF_POPUP: u32 = 0x0010;
pub(crate) const MF_BYPOSITION: u32 = 0x0400;

pub(crate) const ROOT_MENU_START_POSITION: usize = 2;
pub(crate) const ROOT_MENU_STOP_POSITION: usize = 3;
#[allow(dead_code)]
pub(crate) const ROOT_MENU_SETTINGS_POSITION: usize = 5;
#[allow(dead_code)]
pub(crate) const SCHEDULE_MENU_PRESETS_POSITION: usize = 4;
pub(crate) const SCHEDULE_MENU_CANCEL_POSITION: usize = 6;
pub(crate) const SCHEDULE_MENU_RESUME_POSITION: usize = 7;
pub(crate) const SETTINGS_MENU_AUTO_START_POSITION: usize = 0;
pub(crate) const SETTINGS_MENU_AUTO_TIME_POSITION: usize = 1;
pub(crate) const SETTINGS_MENU_AUTO_RESUME_POSITION: usize = 2;

pub(crate) unsafe fn show_menu(hwnd: *mut c_void, app: &mut App) {
    if app.menu_active {
        return;
    }
    app.menu_active = true;
    app.settings_submenu_open = false;
    let mut anchor = Point { x: 0, y: 0 };
    if GetCursorPos(&mut anchor) == 0 {
        app.record(
            "native.GetCursorPos.error",
            format!("raw_status={}", GetLastError()),
        );
        app.menu_active = false;
        return;
    }
    let work_width = GetSystemMetrics(SM_CXWORKAREA).max(0);
    let work_height = GetSystemMetrics(SM_CYWORKAREA).max(0);
    if work_width > 0 && work_height > 0 {
        anchor = clamp_menu_anchor(
            anchor,
            Rect {
                left: 0,
                top: 0,
                right: work_width,
                bottom: work_height,
            },
        );
    }
    loop {
        let (menu, track_target) = if app.settings_submenu_open {
            let submenu = CreatePopupMenu();
            if submenu.is_null() {
                app.record(
                    "native.CreatePopupMenu.settings_root.error",
                    format!("raw_status={}", GetLastError()),
                );
                break;
            }
            let items = settings_menu_items(
                app.config.startup_enabled,
                app.config.automatic,
                app.config.auto_resume_on_ac,
            );
            let startup_id = if app.config.startup_enabled {
                ID_STARTUP_OFF
            } else {
                ID_STARTUP_ON
            };
            let automatic_id = if app.config.automatic {
                ID_AUTOMATIC_OFF
            } else {
                ID_AUTOMATIC_ON
            };
            let auto_resume_id = if app.config.auto_resume_on_ac {
                ID_AUTO_RESUME_OFF
            } else {
                ID_AUTO_RESUME_ON
            };
            let mut ok = append_menu_checked(
                app,
                submenu,
                MF_STRING,
                startup_id,
                wide(&items[0].label).as_ptr(),
            );
            ok &= append_menu_checked(
                app,
                submenu,
                MF_STRING,
                automatic_id,
                wide(&items[1].label).as_ptr(),
            );
            ok &= append_menu_checked(
                app,
                submenu,
                MF_STRING,
                auto_resume_id,
                wide(&items[2].label).as_ptr(),
            );
            if !ok {
                DestroyMenu(submenu);
                break;
            }
            app.popup_menus = Some(PopupMenuHandles {
                root: submenu,
                schedule: None,
                settings: Some(submenu),
                status: None,
            });
            (submenu, submenu)
        } else {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                app.record(
                    "native.CreatePopupMenu.error",
                    format!("raw_status={}", GetLastError()),
                );
                break;
            }
            app.popup_menus = Some(PopupMenuHandles {
                root: menu,
                schedule: None,
                settings: None,
                status: None,
            });
            let items = menu_items_with_duration(
                app.lifecycle_status(),
                app.config.startup_enabled,
                app.config.automatic,
                app.timing_values(),
                app.pause.pause_active(),
            );
            let header_flags = if items[0].enabled {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let start_flags = if items[1].enabled {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let stop_flags = if items[2].enabled {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let menu_ok = append_menu_checked(
                app,
                menu,
                header_flags,
                GITHUB_COMMAND_ID,
                wide(&items[0].label).as_ptr(),
            ) && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
                && append_menu_checked(
                    app,
                    menu,
                    start_flags,
                    ID_START,
                    wide(&items[1].label).as_ptr(),
                )
                && append_menu_checked(
                    app,
                    menu,
                    stop_flags,
                    ID_STOP,
                    wide(&items[2].label).as_ptr(),
                )
                && append_schedule_submenu(app, menu, &items[3].label)
                && append_settings_submenu(app, menu, &items[4].label)
                && append_status_submenu(app, menu)
                && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
                && append_menu_checked(
                    app,
                    menu,
                    MF_STRING,
                    LOGS_COMMAND_ID,
                    wide(&items[6].label).as_ptr(),
                )
                && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
                && append_menu_checked(
                    app,
                    menu,
                    MF_STRING,
                    ID_QUIT,
                    wide(&items[7].label).as_ptr(),
                );
            if !menu_ok {
                app.popup_menus = None;
                if DestroyMenu(menu) == 0 {
                    app.record(
                        "native.DestroyMenu.error",
                        format!("raw_status={}", GetLastError()),
                    );
                }
                break;
            }
            (menu, menu)
        };
        let _ = create_menu_help(hwnd, app);
        begin_popup_refresh_timer(app);
        if SetForegroundWindow(hwnd) == 0 {
            app.record(
                "native.SetForegroundWindow.error",
                format!("raw_status={}", GetLastError()),
            );
        }
        let command = TrackPopupMenu(
            track_target,
            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            anchor.x,
            anchor.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        destroy_menu_help(app);
        kill_popup_refresh_timer(app);
        app.popup_menus = None;
        if DestroyMenu(menu) == 0 {
            app.record(
                "native.DestroyMenu.error",
                format!("raw_status={}", GetLastError()),
            );
        }
        if command == 0 {
            app.record(
                "native.TrackPopupMenu.result",
                format!("result=empty raw_status={}", GetLastError()),
            );
        }
        let Some(command) = returned_menu_command(command) else {
            break;
        };
        app.record(
            "tray.command.dispatch",
            format!("source=TPM_RETURNCMD id={command}"),
        );
        let allowed = crate::tray_surface::menu_command_dispatch_allowed(app.menu_active, true);
        if !allowed {
            break;
        }
        let is_settings_cmd = matches!(
            command,
            ID_STARTUP_ON
                | ID_STARTUP_OFF
                | ID_AUTOMATIC_ON
                | ID_AUTOMATIC_OFF
                | ID_AUTO_RESUME_ON
                | ID_AUTO_RESUME_OFF
        );
        let continues = handle_menu_command(hwnd, app, command);
        if !continues {
            break;
        }
        app.settings_submenu_open = is_settings_cmd;
    }
    app.settings_submenu_open = false;
    destroy_menu_help(app);
    kill_popup_refresh_timer(app);
    app.popup_menus = None;
    app.menu_active = false;
}

pub(crate) fn begin_popup_refresh_timer(app: &mut App) {
    if app.popup_refresh_timer_active {
        return;
    }
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        return;
    };
    let result = unsafe {
        SetTimer(
            hwnd,
            POPUP_REFRESH_TIMER_ID,
            POPUP_REFRESH_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record(
            "native.SetTimer.popup_refresh.error",
            format!("raw_status={}", unsafe { GetLastError() }),
        );
    } else {
        app.popup_refresh_timer_active = true;
    }
}

pub(crate) fn kill_popup_refresh_timer(app: &mut App) {
    if !app.popup_refresh_timer_active {
        return;
    }
    app.popup_refresh_timer_active = false;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            let _ = KillTimer(hwnd, POPUP_REFRESH_TIMER_ID);
        }
    }
}

pub(crate) unsafe fn refresh_popup_menu(app: &mut App) {
    let Some(handles) = app.popup_menus else {
        return;
    };
    let timing = app.timing_values();
    let start_enabled = menu_command_is_enabled_with_pause(
        ID_START,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.config.auto_resume_on_ac,
        app.pause.active(),
    );
    let stop_enabled = menu_command_is_enabled_with_pause(
        ID_STOP,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.config.auto_resume_on_ac,
        app.pause.active(),
    );
    let _ = ModifyMenuW(
        handles.root,
        ROOT_MENU_START_POSITION,
        MF_BYPOSITION | MF_STRING | if start_enabled { 0 } else { MF_GRAYED },
        ID_START,
        wide("Start").as_ptr(),
    );
    let _ = ModifyMenuW(
        handles.root,
        ROOT_MENU_STOP_POSITION,
        MF_BYPOSITION | MF_STRING | if stop_enabled { 0 } else { MF_GRAYED },
        ID_STOP,
        wide("Stop").as_ptr(),
    );
    let startup_id = if app.config.startup_enabled {
        ID_STARTUP_OFF
    } else {
        ID_STARTUP_ON
    };
    let automatic_id = if app.config.automatic {
        ID_AUTOMATIC_OFF
    } else {
        ID_AUTOMATIC_ON
    };
    let auto_resume_id = if app.config.auto_resume_on_ac {
        ID_AUTO_RESUME_OFF
    } else {
        ID_AUTO_RESUME_ON
    };
    if let Some(settings_menu) = handles.settings {
        let items = settings_menu_items(
            app.config.startup_enabled,
            app.config.automatic,
            app.config.auto_resume_on_ac,
        );
        let _ = ModifyMenuW(
            settings_menu,
            SETTINGS_MENU_AUTO_START_POSITION,
            MF_BYPOSITION | MF_STRING,
            startup_id,
            wide(&items[0].label).as_ptr(),
        );
        let _ = ModifyMenuW(
            settings_menu,
            SETTINGS_MENU_AUTO_TIME_POSITION,
            MF_BYPOSITION | MF_STRING,
            automatic_id,
            wide(&items[1].label).as_ptr(),
        );
        let _ = ModifyMenuW(
            settings_menu,
            SETTINGS_MENU_AUTO_RESUME_POSITION,
            MF_BYPOSITION | MF_STRING,
            auto_resume_id,
            wide(&items[2].label).as_ptr(),
        );
    }
    if let Some(schedule) = handles.schedule {
        let cancel_enabled = app
            .pause
            .current()
            .is_some_and(|action| action.action != DurationAction::Pause);
        let _ = ModifyMenuW(
            schedule,
            SCHEDULE_MENU_CANCEL_POSITION,
            MF_BYPOSITION | MF_STRING | if cancel_enabled { 0 } else { MF_GRAYED },
            CANCEL_SCHEDULED_COMMAND_ID,
            wide("Cancel scheduled action").as_ptr(),
        );
        let pause_active = app.pause.pause_active();
        let _ = ModifyMenuW(
            schedule,
            SCHEDULE_MENU_RESUME_POSITION,
            MF_BYPOSITION | MF_STRING | if pause_active { 0 } else { MF_GRAYED },
            CANCEL_PAUSE_COMMAND_ID,
            wide("Resume now").as_ptr(),
        );
    }
    if let Some(status_menu) = handles.status {
        let items = status_menu_items(
            app.lifecycle_status(),
            timing,
            app.controller.ownership(),
            app.pause.current(),
            app.running_duration(),
            std::time::Instant::now(),
        );
        for (index, item) in items.iter().enumerate() {
            let _ = ModifyMenuW(
                status_menu,
                index,
                MF_BYPOSITION | MF_STRING | MF_GRAYED,
                item.command_id.unwrap_or(0),
                wide(&item.label).as_ptr(),
            );
        }
    }
}

pub(crate) unsafe fn append_settings_submenu(
    app: &mut App,
    parent: *mut c_void,
    label: &str,
) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.settings.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    let items = settings_menu_items(
        app.config.startup_enabled,
        app.config.automatic,
        app.config.auto_resume_on_ac,
    );
    let startup_id = if app.config.startup_enabled {
        ID_STARTUP_OFF
    } else {
        ID_STARTUP_ON
    };
    let automatic_id = if app.config.automatic {
        ID_AUTOMATIC_OFF
    } else {
        ID_AUTOMATIC_ON
    };
    let auto_resume_id = if app.config.auto_resume_on_ac {
        ID_AUTO_RESUME_OFF
    } else {
        ID_AUTO_RESUME_ON
    };
    let mut ok = append_menu_checked(
        app,
        submenu,
        MF_STRING,
        startup_id,
        wide(&items[0].label).as_ptr(),
    );
    ok &= append_menu_checked(
        app,
        submenu,
        MF_STRING,
        automatic_id,
        wide(&items[1].label).as_ptr(),
    );
    ok &= append_menu_checked(
        app,
        submenu,
        MF_STRING,
        auto_resume_id,
        wide(&items[2].label).as_ptr(),
    );
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        parent,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide(label).as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    if let Some(handles) = app.popup_menus.as_mut() {
        handles.settings = Some(submenu);
    }
    true
}

pub(crate) unsafe fn append_duration_choice_submenu(
    app: &mut App,
    parent: *mut c_void,
    action: DurationAction,
    label: &str,
) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.duration_choice.error",
            format!("action={} raw_status={}", action.label(), GetLastError()),
        );
        return false;
    }
    let items = preset_menu_items(action, app.presets_manager.presets());
    let base = preset_command_base(action);
    let mut ok = true;
    for item in items.iter() {
        let enabled = match action {
            DurationAction::Start => !app.pause.pause_active(),
            DurationAction::Stop | DurationAction::Pause => true,
        };
        let flags = if enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        ok &= append_menu_checked(
            app,
            submenu,
            flags,
            item.command_id.unwrap_or(base),
            wide(&item.label).as_ptr(),
        );
    }
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        parent,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide(label).as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    true
}

pub(crate) unsafe fn append_schedule_submenu(
    app: &mut App,
    menu: *mut c_void,
    parent_caption: &str,
) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.schedule.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    let items = duration_menu_items(app.pause.current());
    let mut ok =
        append_duration_choice_submenu(app, submenu, DurationAction::Start, &items[0].label);
    ok &= append_duration_choice_submenu(app, submenu, DurationAction::Stop, &items[1].label);
    ok &= append_duration_choice_submenu(app, submenu, DurationAction::Pause, &items[2].label);
    ok &= append_menu_checked(app, submenu, MF_SEPARATOR, 0, std::ptr::null());
    let presets = items[3].clone();
    let presets_flags = if presets.enabled {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    ok &= append_menu_checked(
        app,
        submenu,
        presets_flags,
        SCHEDULE_PRESETS_COMMAND_ID,
        wide(&presets.label).as_ptr(),
    );
    ok &= append_menu_checked(app, submenu, MF_SEPARATOR, 0, std::ptr::null());
    let cancel = items[4].clone();
    let cancel_flags = if cancel.enabled {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    ok &= append_menu_checked(
        app,
        submenu,
        cancel_flags,
        CANCEL_SCHEDULED_COMMAND_ID,
        wide(&cancel.label).as_ptr(),
    );
    let resume = items[5].clone();
    let resume_flags = if resume.enabled {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    ok &= append_menu_checked(
        app,
        submenu,
        resume_flags,
        CANCEL_PAUSE_COMMAND_ID,
        wide(&resume.label).as_ptr(),
    );
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        menu,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide(parent_caption).as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    if let Some(handles) = app.popup_menus.as_mut() {
        handles.schedule = Some(submenu);
    }
    true
}

pub(crate) unsafe fn append_status_submenu(app: &mut App, menu: *mut c_void) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.status.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    let items = status_menu_items(
        app.lifecycle_status(),
        app.timing_values(),
        app.controller.ownership(),
        app.pause.current(),
        app.running_duration(),
        std::time::Instant::now(),
    );
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let flags = if item.enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        ok &= append_menu_checked(
            app,
            submenu,
            flags,
            item.command_id.unwrap_or(0),
            wide(&item.label).as_ptr(),
        );
        if index == 4 {
            app.record(
                "status.submenu.render",
                format!(
                    "rows=state,timing,running_for,next_action,ownership state={:?} ownership={:?} next_action={}",
                    app.lifecycle_status(),
                    app.controller.ownership(),
                    app.pause.current().map_or_else(
                        || "none".to_owned(),
                        |action| format!(
                            "{} remaining_ms={}",
                            action.action.label(),
                            action.remaining(std::time::Instant::now()).as_millis()
                        )
                    )
                ),
            );
            app.record("status.submenu.timing", app.timing_snapshot_details());
        }
    }
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        menu,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide("Status >").as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    if let Some(handles) = app.popup_menus.as_mut() {
        handles.status = Some(submenu);
    }
    true
}

pub(crate) unsafe fn create_menu_help(hwnd: *mut c_void, app: &mut App) -> bool {
    let class_name = wide("tooltips_class32");
    let tooltip = CreateWindowExW(
        WS_EX_TOPMOST,
        class_name.as_ptr(),
        std::ptr::null(),
        WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
        0,
        0,
        0,
        0,
        hwnd,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    if tooltip.is_null() {
        app.record(
            "native.CreateWindowExW.menu_help.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    app.menu_help_text = wide("");
    let tool = menu_tool_info(hwnd, app);
    if SendMessageW(
        tooltip,
        TTM_ADDTOOLW,
        0,
        (&tool as *const ToolInfo).cast::<c_void>() as isize,
    ) == 0
    {
        app.record(
            "native.SendMessageW.menu_help_add.error",
            format!("raw_status={}", GetLastError()),
        );
        DestroyWindow(tooltip);
        return false;
    }
    app.menu_help = Some(tooltip);
    true
}

pub(crate) fn menu_tool_info(hwnd: *mut c_void, app: &App) -> ToolInfo {
    ToolInfo {
        cb_size: size_of::<ToolInfo>() as u32,
        flags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
        hwnd,
        id: hwnd as usize,
        rect: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        instance: std::ptr::null_mut(),
        text: app.menu_help_text.as_ptr(),
        l_param: 0,
        reserved: std::ptr::null_mut(),
    }
}

pub(crate) fn track_position_lparam(x: i32, y: i32) -> isize {
    let packed = (x as u32 & 0xffff) | ((y as u32 & 0xffff) << 16);
    packed as usize as isize
}

pub(crate) fn submenu_description(menu: *mut c_void) -> Option<&'static str> {
    if menu.is_null() {
        return None;
    }
    let count = unsafe { GetMenuItemCount(menu) };
    if count < 0 {
        return None;
    }
    for position in 0..count {
        let mut buffer = [0u16; 64];
        let length = unsafe {
            GetMenuStringW(
                menu,
                position as usize,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                MF_BYPOSITION,
            )
        };
        if length <= 0 {
            continue;
        }
        let label = String::from_utf16_lossy(&buffer[..length as usize]);
        let label = label.trim_end_matches('&');
        let description = match label {
            "Schedule >" => Some("Schedule a bounded timing action"),
            "Start in >" => Some("Schedule a future guarded acquire"),
            "Stop in >" => Some("Schedule a future guarded release"),
            "Pause for >" => Some("Suppress acquisition for a fixed duration"),
            "Resume now" => Some("Resume timing and cancel the pause"),
            "Settings >" => Some("Configure startup and automation settings"),
            "Status >" => Some("View read-only lifecycle details"),
            _ => None,
        };
        if description.is_some() {
            return description;
        }
    }
    None
}

pub(crate) fn update_menu_help(app: &mut App, command: usize, flags: u32, menu: *mut c_void) {
    let Some(tooltip) = app.menu_help else {
        return;
    };
    let description = menu_description(command).or_else(|| {
        if flags & MF_POPUP != 0 {
            submenu_description(menu)
        } else {
            None
        }
    });
    let Some(description) = description else {
        unsafe {
            let tool = menu_tool_info(
                app.tray_icon
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |icon| icon.h_wnd),
                app,
            );
            SendMessageW(
                tooltip,
                TTM_TRACKACTIVATE,
                0,
                (&tool as *const ToolInfo).cast::<c_void>() as isize,
            );
        }
        return;
    };
    app.menu_help_text = wide(description);
    let tool = menu_tool_info(
        app.tray_icon
            .as_ref()
            .map_or(std::ptr::null_mut(), |icon| icon.h_wnd),
        app,
    );
    unsafe {
        SendMessageW(
            tooltip,
            TTM_UPDATETIPTEXTW,
            0,
            (&tool as *const ToolInfo).cast::<c_void>() as isize,
        );
        let mut cursor = Point::default();
        if GetCursorPos(&mut cursor) == 0 {
            app.record(
                "native.GetCursorPos.menu_help.error",
                format!("raw_status={}", GetLastError()),
            );
            SendMessageW(
                tooltip,
                TTM_TRACKACTIVATE,
                0,
                (&tool as *const ToolInfo).cast::<c_void>() as isize,
            );
            return;
        }
        SendMessageW(
            tooltip,
            TTM_TRACKPOSITION,
            0,
            track_position_lparam(cursor.x, cursor.y),
        );
        SendMessageW(
            tooltip,
            TTM_TRACKACTIVATE,
            1,
            (&tool as *const ToolInfo).cast::<c_void>() as isize,
        );
    }
}

pub(crate) fn destroy_menu_help(app: &mut App) {
    if let Some(tooltip) = app.menu_help.take() {
        unsafe {
            let tool = menu_tool_info(
                app.tray_icon
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |icon| icon.h_wnd),
                app,
            );
            SendMessageW(
                tooltip,
                TTM_TRACKACTIVATE,
                0,
                (&tool as *const ToolInfo).cast::<c_void>() as isize,
            );
            if DestroyWindow(tooltip) == 0 {
                app.record(
                    "native.DestroyWindow.menu_help.error",
                    format!("raw_status={}", GetLastError()),
                );
            }
        }
    }
    app.menu_help_text.clear();
}

pub(crate) unsafe fn append_menu_checked(
    app: &mut App,
    menu: *mut c_void,
    flags: u32,
    command: usize,
    text: *const u16,
) -> bool {
    let result = AppendMenuW(menu, flags, command, text);
    let raw_error = if result == 0 { GetLastError() } else { 0 };
    if matches!(
        native_bool_result(result, raw_error),
        NativeResult::Succeeded
    ) {
        true
    } else {
        app.record(
            "native.AppendMenuW.error",
            format!("command={command} raw_status={raw_error}"),
        );
        false
    }
}

pub(crate) unsafe fn handle_menu_command(hwnd: *mut c_void, app: &mut App, command: usize) -> bool {
    if !menu_command_is_enabled_with_pause(
        command,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.config.auto_resume_on_ac,
        app.pause.active(),
    ) {
        let reason = if command == ID_START && app.pause.pause_active() {
            "start_disabled_while_paused"
        } else {
            "disabled_or_unknown"
        };
        app.record(
            "tray.command.rejected",
            format!("id={command} reason={reason}"),
        );
        return false;
    }
    app.begin_operation(DiagnosticSource::TrayCommand);
    app.record("tray.command.id", format!("id={command}"));
    let action_name = match command {
        ID_START => "start",
        ID_STOP => "stop",
        LOGS_COMMAND_ID => "logs",
        GITHUB_COMMAND_ID => "github",
        SCHEDULE_PRESETS_COMMAND_ID => "schedule_presets",
        CANCEL_SCHEDULED_COMMAND_ID => "cancel_scheduled",
        CANCEL_PAUSE_COMMAND_ID => "cancel_pause",
        START_IN_1_COMMAND_ID => "start_in_1m",
        START_IN_5_COMMAND_ID => "start_in_5m",
        START_IN_15_COMMAND_ID => "start_in_15m",
        START_IN_30_COMMAND_ID => "start_in_30m",
        START_IN_60_COMMAND_ID => "start_in_60m",
        STOP_IN_1_COMMAND_ID => "stop_in_1m",
        STOP_IN_5_COMMAND_ID => "stop_in_5m",
        STOP_IN_15_COMMAND_ID => "stop_in_15m",
        STOP_IN_30_COMMAND_ID => "stop_in_30m",
        STOP_IN_60_COMMAND_ID => "stop_in_60m",
        PAUSE_FOR_5_COMMAND_ID => "pause_for_5m",
        PAUSE_FOR_15_COMMAND_ID => "pause_for_15m",
        PAUSE_FOR_30_COMMAND_ID => "pause_for_30m",
        PAUSE_FOR_60_COMMAND_ID => "pause_for_60m",
        ID_STARTUP_ON => "startup_on",
        ID_STARTUP_OFF => "startup_off",
        ID_AUTOMATIC_ON => "automatic_on",
        ID_AUTOMATIC_OFF => "automatic_off",
        ID_AUTO_RESUME_ON => "auto_resume_on",
        ID_AUTO_RESUME_OFF => "auto_resume_off",
        ID_QUIT => "quit",
        _ => {
            if preset_command(command).is_some() {
                "schedule_preset"
            } else {
                "unknown"
            }
        }
    };
    let command_valid = action_name != "unknown";
    let dispatch_outcome = if command_valid {
        DiagnosticOutcome::Completed
    } else {
        DiagnosticOutcome::Failed
    };
    let dispatch_context = app.operation.map_or_else(
        || {
            app.diagnostics
                .begin_operation(DiagnosticSource::TrayCommand)
        },
        |root| {
            app.diagnostics
                .child_operation(root, DiagnosticSource::TrayCommand)
        },
    );
    app.diagnostics.record_verification(
        dispatch_context,
        DiagnosticSource::TrayCommand,
        dispatch_outcome,
        "command.dispatch.verify",
        format!("command_id={command} action={action_name} valid={command_valid}"),
    );
    let mut operation_outcome = DiagnosticOutcome::Completed;
    match command {
        ID_START => manual_start(app),
        ID_STOP => manual_stop(app),
        LOGS_COMMAND_ID => {
            app.record("tray.command", "command=logs");
            operation_outcome = crate::ui::diagnostic_window::diagnostic_open_operation_outcome(
                crate::ui::open_diagnostic_window(app),
            );
        }
        GITHUB_COMMAND_ID => open_github_page(hwnd, app),
        SCHEDULE_PRESETS_COMMAND_ID => {
            crate::ui::open_presets_window(app);
        }
        CANCEL_SCHEDULED_COMMAND_ID | CANCEL_PAUSE_COMMAND_ID => cancel_scheduled_action(app),
        START_IN_1_COMMAND_ID
        | START_IN_5_COMMAND_ID
        | START_IN_15_COMMAND_ID
        | START_IN_30_COMMAND_ID
        | START_IN_60_COMMAND_ID
        | STOP_IN_1_COMMAND_ID
        | STOP_IN_5_COMMAND_ID
        | STOP_IN_15_COMMAND_ID
        | STOP_IN_30_COMMAND_ID
        | STOP_IN_60_COMMAND_ID
        | PAUSE_FOR_5_COMMAND_ID
        | PAUSE_FOR_15_COMMAND_ID
        | PAUSE_FOR_30_COMMAND_ID
        | PAUSE_FOR_60_COMMAND_ID => {
            if let Some((action, duration)) = duration_command(command) {
                schedule_duration_action(app, action, duration.to_preset());
            }
        }
        ID_STARTUP_ON => set_startup(app, true),
        ID_STARTUP_OFF => set_startup(app, false),
        ID_AUTOMATIC_ON => set_automatic(app, true),
        ID_AUTOMATIC_OFF => set_automatic(app, false),
        ID_AUTO_RESUME_ON => set_auto_resume(app, true),
        ID_AUTO_RESUME_OFF => set_auto_resume(app, false),
        ID_QUIT => {
            app.record("tray.command", "command=quit");
            app.record("lifecycle.shutdown_request", "source=tray");
            app.record("quit.requested", "source=tray");
            let decision = quit_decision(app);
            app.record(
                "quit.active_state",
                format!(
                    "decision={decision:?} status={:?} ownership={:?} verification={:?}",
                    app.tray_status,
                    app.controller.ownership(),
                    app.controller.verification()
                ),
            );
            match decision {
                QuitDecision::ExitNormally => {
                    app.record("quit.dialog.result", "result=not_shown");
                    match app.cleanup_normal_shutdown() {
                        Ok(()) => {
                            app.record("quit.cleanup.result", "result=verified");
                            app.record(
                                "quit.exit.allowed",
                                "result=allowed reason=already_stopped",
                            );
                            PostQuitMessage(0);
                            return false;
                        }
                        Err(error) => {
                            app.record(
                                "quit.cleanup.result",
                                format!("result=unverified error={error}"),
                            );
                            app.record("quit.blocked.uncertain_cleanup", format!("error={error}"));
                            app.record("quit.exit.allowed", "result=denied");
                            show_shutdown_warning(
                                hwnd,
                                &format!(
                                    "True™ Tick could not verify a safe stop. The app remains open.\n\n{error}"
                                ),
                            );
                            return true;
                        }
                    }
                }
                QuitDecision::RequireSafetyDialog { reason } => {
                    app.record("quit.warning.shown", format!("reason={reason:?}"));
                    let warning = show_quit_warning(hwnd, reason);
                    app.record(
                        "quit.dialog.result",
                        format!(
                            "dialog_path=message_box reason={reason:?} message_box_result={:?} final_decision={:?} dialog_shown={}",
                            warning.message_box_result,
                            warning.decision,
                            warning.dialog_shown
                        ),
                    );
                    match warning.decision {
                        QuitDialogDecision::StopAndQuit => {
                            app.record("quit.dialog.result", "result=stop_and_quit");
                            app.record("quit.stop_and_quit.selected", "result=selected");
                            match app.cleanup_normal_shutdown() {
                                Ok(()) => {
                                    app.record("quit.cleanup.result", "result=verified");
                                    app.record("quit.release.result", "result=verified");
                                    app.record("quit.exit.allowed", "result=allowed");
                                    PostQuitMessage(0);
                                    return false;
                                }
                                Err(error) => {
                                    app.record(
                                        "quit.cleanup.result",
                                        format!("result=unverified error={error}"),
                                    );
                                    app.record(
                                        "quit.blocked.uncertain_cleanup",
                                        format!("error={error}"),
                                    );
                                    app.record("quit.exit.allowed", "result=denied");
                                    let message = format!(
                                        "True™ Tick could not verify a safe stop. The app remains open.\n\n{error}"
                                    );
                                    MessageBoxW(
                                        hwnd,
                                        wide(&message).as_ptr(),
                                        wide("True™ Tick quit warning").as_ptr(),
                                        MB_ICONWARNING,
                                    );
                                    return true;
                                }
                            }
                        }
                        QuitDialogDecision::Cancel => {
                            app.record(
                                "quit.cancel.selected",
                                format!("result=cancelled dialog_shown={}", warning.dialog_shown),
                            );
                            app.record("quit.cleanup.result", "result=not_attempted");
                            app.record("quit.exit.allowed", "result=denied");
                            return warning.dialog_shown;
                        }
                    }
                }
            }
        }
        _ => {
            if let Some((action, preset_index)) = preset_command(command) {
                schedule_preset_action(app, action, preset_index);
            }
        }
    }
    let keeps_open = menu_action_keeps_open(command);
    if command != ID_QUIT {
        app.finish_operation(operation_outcome);
    }
    keeps_open
}

pub(crate) fn returned_menu_command(result: i32) -> Option<usize> {
    (result > 0).then_some(result as usize)
}

pub(crate) fn clamp_menu_anchor(point: Point, work: Rect) -> Point {
    let max_x = work.right.saturating_sub(1).max(work.left);
    let max_y = work.bottom.saturating_sub(1).max(work.top);
    Point {
        x: point.x.clamp(work.left, max_x),
        y: point.y.clamp(work.top, max_y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_menu_positions_match_the_single_label_source() {
        let items = crate::tray_surface::duration_menu_items(None);
        assert_eq!(items.len(), 6);
        for item in &items[..3] {
            assert_eq!(item.command_id, None);
        }
        assert_eq!(SCHEDULE_MENU_PRESETS_POSITION, 4);
        assert_eq!(SCHEDULE_MENU_CANCEL_POSITION, 6);
        assert_eq!(SCHEDULE_MENU_RESUME_POSITION, 7);
        assert_eq!(
            items[SCHEDULE_MENU_PRESETS_POSITION - 1].command_id,
            Some(crate::tray_surface::SCHEDULE_PRESETS_COMMAND_ID)
        );
        assert_eq!(
            items[SCHEDULE_MENU_CANCEL_POSITION - 2].command_id,
            Some(crate::tray_surface::CANCEL_SCHEDULED_COMMAND_ID)
        );
        assert_eq!(
            items[SCHEDULE_MENU_RESUME_POSITION - 2].command_id,
            Some(crate::tray_surface::CANCEL_PAUSE_COMMAND_ID)
        );
    }

    #[test]
    fn signed_screen_coordinates_keep_their_low_32_bits() {
        assert_eq!(track_position_lparam(-1, -2) as u32, 0xfffe_ffff);
        assert_eq!(track_position_lparam(-1920, 1080) as u32, 0x0438_f880);
    }

    #[test]
    fn clamp_menu_anchor_keeps_point_inside_work_area() {
        let work = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(
            clamp_menu_anchor(Point { x: 500, y: 400 }, work),
            Point { x: 500, y: 400 }
        );
        let work = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(
            clamp_menu_anchor(Point { x: 2500, y: 3000 }, work),
            Point { x: 1919, y: 1039 }
        );
        let work = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(
            clamp_menu_anchor(Point { x: -10, y: -20 }, work),
            Point { x: 0, y: 0 }
        );
    }

    #[test]
    fn settings_menu_positions_and_commands_match_settings_contract() {
        assert_eq!(ROOT_MENU_SETTINGS_POSITION, 5);
        assert_eq!(SETTINGS_MENU_AUTO_START_POSITION, 0);
        assert_eq!(SETTINGS_MENU_AUTO_TIME_POSITION, 1);
        assert_eq!(SETTINGS_MENU_AUTO_RESUME_POSITION, 2);
        assert_eq!(ID_AUTO_RESUME_ON, 1009);
        assert_eq!(ID_AUTO_RESUME_OFF, 1010);
    }

    #[test]
    fn returned_popup_command_is_dispatched_once_as_an_optional_id() {
        assert_eq!(returned_menu_command(0), None);
        assert_eq!(returned_menu_command(-1), None);
        assert_eq!(returned_menu_command(ID_START as i32), Some(ID_START));
        assert_eq!(
            returned_menu_command(ID_AUTOMATIC_OFF as i32),
            Some(ID_AUTOMATIC_OFF)
        );
    }
}
