use crate::config;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;

use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{TimerController, Verification};
use tick_platform_windows::WindowsTimerPlatform;
use tick_policy::{decide, PolicyInput, PowerState};
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

use crate::tray_surface::{auto_start_label, automatic_label, tooltip, IconColor, TrayStatus};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
const WM_CREATE: u32 = 0x0001;
const WM_COMMAND: u32 = 0x0111;
const WM_DESTROY: u32 = 0x0002;
const WM_RBUTTONUP: usize = 0x0205;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
const ID_QUIT: usize = 1004;
const ID_STARTUP_ON: usize = 1005;
const ID_STARTUP_OFF: usize = 1006;
const ID_AUTOMATIC_ON: usize = 1007;
const ID_AUTOMATIC_OFF: usize = 1008;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;
const MF_GRAYED: u32 = 0x0001;

const NIF_MESSAGE: u32 = 0x0001;
const NIF_ICON: u32 = 0x0002;
const NIF_TIP: u32 = 0x0004;
const NIM_ADD: u32 = 0x0000;
const NIM_DELETE: u32 = 0x0002;
const NIM_MODIFY: u32 = 0x0001;
const GWLP_USERDATA: i32 = -21;
const IDI_APPLICATION: usize = 32512;
const MB_ICONWARNING: u32 = 0x0000_0030;

#[repr(C)]
struct NotifyIconData {
    cb_size: u32,
    h_wnd: *mut c_void,
    u_id: u32,
    u_flags: u32,
    u_callback_message: u32,
    h_icon: *mut c_void,
    sz_tip: [u16; 128],
    dw_state: u32,
    dw_state_mask: u32,
    sz_info: [u16; 256],
    u_timeout_or_version: u32,
    sz_info_title: [u16; 64],
    dw_info_flags: u32,
    guid: [u8; 16],
    h_balloon_icon: *mut c_void,
}

#[repr(C)]
struct CreateStruct {
    create_params: *mut c_void,
    instance: *mut c_void,
    menu: *mut c_void,
    parent: *mut c_void,
    height: i32,
    width: i32,
    y: i32,
    x: i32,
    style: u32,
    name: *const u16,
    class_name: *const u16,
    extended_style: u32,
}

fn app_create_params(create: *const CreateStruct) -> *mut c_void {
    if create.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*create).create_params }
    }
}

#[repr(C)]
struct WndClass {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

struct App {
    controller: TimerController<WindowsTimerPlatform>,
    observation: WindowsObservation,
    config: config::Config,
    tray_status: TrayStatus,
    config_path: PathBuf,
    executable: PathBuf,
    startup_status: String,
    tray_icon: Option<NotifyIconData>,
}

pub fn run() {
    unsafe {
        let executable = get_module_file_name_w_path();
        let config_path = config::path_from_executable(&executable);
        let (loaded, config_status) = match config::load(&config_path) {
            Ok(config) => (config, None),
            Err(error) => (config::Config::default(), Some(format!("Red: {error}"))),
        };
        let startup_status = match (config_status, startup_operation(loaded.startup_enabled)) {
            (Some(error), _) => error,
            (None, StartupOperation::Register) => {
                match crate::portable::launcher_path_from_slot_executable(&executable) {
                    Ok(launcher) => match WindowsUserStartup::default().register(&launcher) {
                        Ok(()) => {
                            "boot startup registered for the current-user Launcher.exe entry point"
                                .to_owned()
                        }
                        Err(error) => format!("Red: boot startup registration error {error:?}"),
                    },
                    Err(error) => format!("Red: portable launcher path unavailable {error:?}"),
                }
            }
            (None, StartupOperation::Remove) => match WindowsUserStartup::default().remove() {
                Ok(()) => "boot startup registration disabled by config".to_owned(),
                Err(error) => format!("Red: boot startup removal error {error:?}"),
            },
        };

        let mut observation = WindowsObservation::default();
        let _ = observation.refresh_power();
        let app = Box::new(App {
            controller: TimerController::new(
                WindowsTimerPlatform::default(),
                loaded.request_interval,
            ),
            observation,
            config: loaded,
            tray_status: if startup_status.starts_with("Red") {
                TrayStatus::Stopped
            } else {
                TrayStatus::Warning
            },
            config_path,
            executable,
            startup_status,
            tray_icon: None,
        });
        let app_ptr = Box::into_raw(app);
        let app = &mut *app_ptr;
        let class_name = wide("TrueTickTrayClass");
        let wnd_class = WndClass {
            style: 0,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: GetModuleHandleW(std::ptr::null()),
            icon: LoadIconW(std::ptr::null_mut(), IDI_APPLICATION as *const u16),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: class_name.as_ptr(),
        };
        RegisterClassW(&wnd_class);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            wnd_class.instance,
            app_ptr as *mut c_void,
        );
        let mut icon = NotifyIconData::new(hwnd, app.tray_status);
        Shell_NotifyIconW(NIM_ADD, &mut icon);
        app.tray_icon = Some(icon);
        if app.config.automatic {
            reconcile(app);
        }
        app.publish();
        let mut message = Message::default();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if let Some(mut icon) = app.tray_icon.take() {
            Shell_NotifyIconW(NIM_DELETE, &mut icon);
        }
        drop(Box::from_raw(app_ptr));
    }
}

fn reconcile(app: &mut App) {
    apply_policy(app);
}

fn apply_policy(app: &mut App) {
    let decision = decide(PolicyInput {
        enabled: true,
        eligible_profile: true,
        power: app.observation.power().state,
    });
    if decision.status == tick_core::Status::Requested {
        match app.controller.start() {
            Ok(Verification::Verified) => app.tray_status = TrayStatus::Active,
            Ok(Verification::Unverified | Verification::NotCollected) => {
                app.tray_status = TrayStatus::Warning
            }
            Err(_error) => app.tray_status = TrayStatus::Stopped,
        }
        return;
    }

    release_for_policy(app, decision.reason);
}

fn release_for_policy(app: &mut App, _reason: tick_policy::PolicyReason) {
    match app.controller.stop() {
        Ok(_) => {
            app.tray_status = TrayStatus::Warning;
        }
        Err(_error) => app.tray_status = TrayStatus::Stopped,
    }
}

fn release_for_power_change(app: &mut App) {
    if app.observation.power().state != PowerState::Ac {
        release_for_policy(
            app,
            match app.observation.power().state {
                PowerState::Battery => tick_policy::PolicyReason::BatteryRestricted,
                PowerState::BatterySaver => tick_policy::PolicyReason::BatterySaverRestricted,
                PowerState::Unknown => tick_policy::PolicyReason::PowerUnknown,
                PowerState::Ac => unreachable!(),
            },
        );
    }
}

impl App {
    fn publish(&mut self) {
        if let Some(icon) = self.tray_icon.as_mut() {
            update_icon(icon, self.tray_status);
        }
    }
}

fn update_icon(icon: &mut NotifyIconData, status: TrayStatus) {
    let old_icon = icon.h_icon;
    icon.h_icon = unsafe { status_icon(status) };
    icon.sz_tip = [0; 128];
    for (target, source) in icon.sz_tip.iter_mut().zip(tooltip(status).encode_utf16()) {
        *target = source;
    }
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, icon);
        DestroyIcon(old_icon);
    }
}

unsafe fn status_icon(status: TrayStatus) -> *mut c_void {
    let color = match status.icon_color() {
        // CreateBitmap receives the packed 32-bit pixel as 0x00RRGGBB.
        IconColor::Green => 0x0000b000u32,
        IconColor::Yellow => 0x00d0d000u32,
        IconColor::Red => 0x00d00000u32,
    };
    let pixels = [color; 16 * 16];
    let mask = [0u8; 16 * 16 / 8];
    let bitmap = CreateBitmap(16, 16, 1, 32, pixels.as_ptr() as *const c_void);
    let mask_bitmap = CreateBitmap(16, 16, 1, 1, mask.as_ptr() as *const c_void);
    let info = IconInfo {
        f_icon: 1,
        x_hotspot: 0,
        y_hotspot: 0,
        h_bm_mask: mask_bitmap,
        h_bm_color: bitmap,
    };
    let icon = CreateIconIndirect(&info);
    DeleteObject(bitmap);
    DeleteObject(mask_bitmap);
    icon
}

unsafe extern "system" fn window_proc(
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if message == WM_CREATE {
        let create = l_param as *const CreateStruct;
        let app_ptr = app_create_params(create);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        return 0;
    }
    if !app.is_null() {
        let app = &mut *app;
        match message {
            WM_TRAY if l_param as usize == WM_RBUTTONUP => show_menu(hwnd, app),
            WM_COMMAND => match w_param & 0xffff {
                ID_STARTUP_ON => set_startup(app, true),
                ID_STARTUP_OFF => set_startup(app, false),
                ID_AUTOMATIC_ON => set_automatic(app, true),
                ID_AUTOMATIC_OFF => set_automatic(app, false),
                ID_QUIT => {
                    if let Err(error) = app.controller.stop() {
                        let message = format!(
                            "Normal shutdown release failed. Tick ownership is unverified.\n\n{error:?}"
                        );
                        app.tray_status = TrayStatus::Warning;
                        app.publish();
                        MessageBoxW(
                            hwnd,
                            wide(&message).as_ptr(),
                            wide("True Tick shutdown warning").as_ptr(),
                            MB_ICONWARNING,
                        );
                    }
                    PostQuitMessage(0);
                }
                _ => {}
            },
            WM_POWERBROADCAST if w_param == PBT_APMPOWERSTATUSCHANGE => {
                let _ = app.observation.refresh_power();
                if app.config.automatic {
                    reconcile(app);
                } else {
                    release_for_power_change(app);
                }
                app.publish();
            }
            WM_DESTROY => PostQuitMessage(0),
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}

unsafe fn show_menu(hwnd: *mut c_void, app: &App) {
    let menu = CreatePopupMenu();
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
    AppendMenuW(
        menu,
        MF_STRING,
        startup_id,
        wide(auto_start_label(app.config.startup_enabled)).as_ptr(),
    );
    AppendMenuW(
        menu,
        MF_STRING,
        automatic_id,
        wide(automatic_label(app.config.automatic)).as_ptr(),
    );

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(
        menu,
        MF_STRING | MF_GRAYED,
        0,
        wide(&format!("Status: {}", app.tray_status.label())).as_ptr(),
    );
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, MF_STRING, ID_QUIT, wide("Quit").as_ptr());
    let mut point = Point { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    DestroyMenu(menu);
}

fn set_automatic(app: &mut App, enabled: bool) {
    let mut next = app.config.clone();
    next.automatic = enabled;
    if config::save_atomic(&app.config_path, &next).is_err() {
        app.tray_status = TrayStatus::Stopped;
        app.publish();
        return;
    }
    app.config = next;
    if enabled {
        reconcile(app);
    } else {
        match app.controller.stop() {
            Ok(_) => app.tray_status = TrayStatus::Stopped,
            Err(_error) => app.tray_status = TrayStatus::Warning,
        }
    }
    app.publish();
}

fn set_startup(app: &mut App, enabled: bool) {
    let registration = if enabled {
        match crate::portable::launcher_path_from_slot_executable(&app.executable) {
            Ok(launcher) => WindowsUserStartup::default().register(&launcher),
            Err(_error) => Err(tick_startup_windows::StartupError::InvalidExecutablePath),
        }
    } else {
        WindowsUserStartup::default().remove()
    };
    if registration.is_err() {
        app.startup_status = "startup registration error".into();
        app.tray_status = TrayStatus::Stopped;
        app.publish();
        return;
    }
    let mut next = app.config.clone();
    next.startup_enabled = enabled;
    if config::save_atomic(&app.config_path, &next).is_err() {
        app.startup_status = "startup config error".into();
        app.tray_status = TrayStatus::Stopped;
        app.publish();
        return;
    }
    app.config = next;
    app.startup_status = if enabled {
        "boot startup registered for the current-user Launcher.exe entry point".into()
    } else {
        "boot startup registration disabled by config".into()
    };
    app.publish();
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

impl NotifyIconData {
    fn new(hwnd: *mut c_void, status: TrayStatus) -> Self {
        let mut value = Self {
            cb_size: size_of::<Self>() as u32,
            h_wnd: hwnd,
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: WM_TRAY,
            h_icon: unsafe { status_icon(status) },
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
        for (target, source) in value.sz_tip.iter_mut().zip(tooltip(status).encode_utf16()) {
            *target = source;
        }
        value
    }
}

#[repr(C)]
#[derive(Default)]
struct Message {
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    point: Point,
}
#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn CreateWindowExW(
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
    fn GetMessageW(message: *mut Message, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn DefWindowProcW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    fn PostQuitMessage(code: i32);
    fn MessageBoxW(hwnd: *mut c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
    fn CreatePopupMenu() -> *mut c_void;
    fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
    fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    fn TrackPopupMenu(
        menu: *mut c_void,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: *mut c_void,
        rect: *const c_void,
    ) -> i32;
    fn DestroyMenu(menu: *mut c_void) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn DestroyIcon(icon: *mut c_void) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconData) -> i32;
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
    fn DeleteObject(object: *mut c_void) -> i32;
}

#[repr(C)]
struct IconInfo {
    f_icon: i32,
    x_hotspot: u32,
    y_hotspot: u32,
    h_bm_mask: *mut c_void,
    h_bm_color: *mut c_void,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_params_are_taken_from_create_struct() {
        let marker = 7u8;
        let create = CreateStruct {
            create_params: (&marker as *const u8).cast_mut().cast(),
            instance: std::ptr::null_mut(),
            menu: std::ptr::null_mut(),
            parent: std::ptr::null_mut(),
            height: 0,
            width: 0,
            y: 0,
            x: 0,
            style: 0,
            name: std::ptr::null(),
            class_name: std::ptr::null(),
            extended_style: 0,
        };
        assert_eq!(app_create_params(&create), create.create_params);
        assert!(app_create_params(std::ptr::null()).is_null());
    }
}

unsafe fn get_module_file_name_w_path() -> PathBuf {
    let mut buffer = [0u16; 260];
    let length = GetModuleFileNameW(
        std::ptr::null_mut(),
        buffer.as_mut_ptr(),
        buffer.len() as u32,
    );
    String::from_utf16_lossy(&buffer[..length as usize]).into()
}

extern "system" {
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
}
