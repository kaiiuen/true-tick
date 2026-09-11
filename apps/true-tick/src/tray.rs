use crate::config;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;

use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{TimerController, Verification};
use tick_platform_windows::WindowsTimerPlatform;
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
const WM_COMMAND: u32 = 0x0111;
const WM_DESTROY: u32 = 0x0002;
const WM_RBUTTONUP: usize = 0x0205;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
const ID_START: usize = 1001;
const ID_STOP: usize = 1002;
const ID_AUTOMATIC: usize = 1003;
const ID_QUIT: usize = 1004;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;
const NIF_MESSAGE: u32 = 0x0001;
const NIF_ICON: u32 = 0x0002;
const NIF_TIP: u32 = 0x0004;
const NIM_ADD: u32 = 0x0000;
const NIM_DELETE: u32 = 0x0002;
const NIM_MODIFY: u32 = 0x0001;
const GWLP_USERDATA: i32 = -21;
const IDI_APPLICATION: usize = 32512;

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
    automatic: bool,
    status_text: String,
    config_path: PathBuf,
    startup_status: String,
}

pub fn run() {
    unsafe {
        let executable = get_module_file_name_w_path();
        let config_path = config::path_from_executable(&executable);
        let loaded = config::load(&config_path).unwrap_or_default();
        let startup_status = match startup_operation(loaded.startup_enabled) {
            StartupOperation::Register => {
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
            StartupOperation::Remove => match WindowsUserStartup::default().remove() {
                Ok(()) => "boot startup registration disabled by config".to_owned(),
                Err(error) => format!("Red: boot startup removal error {error:?}"),
            },
        };
        let portable_status = match crate::portable::select(
            executable
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
        ) {
            crate::portable::Selection::Selected { slot, path } => {
                format!("slot {:?}: {}", slot, path.display())
            }
            crate::portable::Selection::RepairRequired(reason) => {
                format!("A/B scaffold: repair required, {reason}")
            }
        };
        let mut observation = WindowsObservation::default();
        let _ = observation.refresh_power();
        let app = Box::new(App {
            controller: TimerController::new(
                WindowsTimerPlatform::default(),
                loaded.request_interval,
            ),
            observation,
            automatic: loaded.automatic,
            status_text: format!("Yellow: waiting for a verified request, {portable_status}"),
            config_path,
            startup_status,
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
        let mut icon = NotifyIconData::new(hwnd, status(&app));
        Shell_NotifyIconW(NIM_ADD, &mut icon);
        if app.automatic {
            reconcile(app);
            update_icon(&mut icon, status(&app));
        }
        let mut message = Message::default();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Shell_NotifyIconW(NIM_DELETE, &mut icon);
        drop(Box::from_raw(app_ptr));
    }
}

fn reconcile(app: &mut App) {
    if app.observation.power().state == tick_policy::PowerState::Ac {
        match app.controller.start() {
            Ok(Verification::Verified) => app.status_text = "Green: active and verified".into(),
            Ok(Verification::Unverified | Verification::NotCollected) => {
                app.status_text = "Yellow: request accepted but unverified".into()
            }
            Err(error) => app.status_text = format!("Red: request error {error:?}"),
        }
    } else {
        let _ = app.controller.stop();
        app.status_text = match app.observation.power().state {
            tick_policy::PowerState::Battery => "Yellow: released by battery policy".into(),
            tick_policy::PowerState::BatterySaver => {
                "Red: automatic activation blocked by Battery Saver".into()
            }
            tick_policy::PowerState::Unknown => {
                "Red: automatic activation blocked by unknown power state".into()
            }
            tick_policy::PowerState::Ac => "Yellow: released".into(),
        };
    }
}

fn status(app: &App) -> String {
    format!(
        "{} | {} | config: {}",
        app.status_text,
        app.startup_status,
        app.config_path.display()
    )
}

fn update_icon(icon: &mut NotifyIconData, text: String) {
    let old_icon = icon.h_icon;
    icon.h_icon = unsafe { status_icon(&text) };
    icon.sz_tip = [0; 128];
    for (target, source) in icon.sz_tip.iter_mut().zip(text.encode_utf16()) {
        *target = source;
    }
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, icon);
        DestroyIcon(old_icon);
    }
}

unsafe fn status_icon(text: &str) -> *mut c_void {
    let color = if text.starts_with("Green") {
        0x0000b000u32
    } else if text.starts_with("Red") {
        0x0000d000u32
    } else {
        0x0000d0d0u32
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
    if message == 1 {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, l_param);
        return 0;
    }
    if !app.is_null() {
        let app = &mut *app;
        match message {
            WM_TRAY if l_param as usize == WM_RBUTTONUP => show_menu(hwnd, app),
            WM_COMMAND => match w_param & 0xffff {
                ID_START => {
                    app.automatic = false;
                    app.status_text = match app.controller.start() {
                        Ok(Verification::Verified) => "Green: active and verified".into(),
                        Ok(_) => "Yellow: request accepted but unverified".into(),
                        Err(error) => format!("Red: request error {error:?}"),
                    };
                }
                ID_STOP => {
                    app.automatic = false;
                    match app.controller.stop() {
                        Ok(_) => app.status_text = "Yellow: stopped and released".into(),
                        Err(error) => app.status_text = format!("Red: release error {error:?}"),
                    }
                }
                ID_AUTOMATIC => {
                    app.automatic = true;
                    reconcile(app);
                }
                ID_QUIT => PostQuitMessage(0),
                _ => {}
            },
            WM_POWERBROADCAST if w_param == PBT_APMPOWERSTATUSCHANGE => {
                let _ = app.observation.refresh_power();
                if app.automatic {
                    reconcile(app);
                }
            }
            WM_DESTROY => PostQuitMessage(0),
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}

unsafe fn show_menu(hwnd: *mut c_void, app: &App) {
    let menu = CreatePopupMenu();
    AppendMenuW(menu, MF_STRING, ID_START, wide("Start").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_STOP, wide("Stop").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_AUTOMATIC, wide("Automatic").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, MF_STRING, 0, wide(&status(app)).as_ptr());
    AppendMenuW(
        menu,
        MF_STRING,
        0,
        wide("Power and timing are event-driven").as_ptr(),
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

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

impl NotifyIconData {
    fn new(hwnd: *mut c_void, tip: String) -> Self {
        let mut value = Self {
            cb_size: size_of::<Self>() as u32,
            h_wnd: hwnd,
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: WM_TRAY,
            h_icon: unsafe { status_icon(&tip) },
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
        for (target, source) in value.sz_tip.iter_mut().zip(tip.encode_utf16()) {
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
