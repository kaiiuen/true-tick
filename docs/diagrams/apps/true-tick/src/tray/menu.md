# Tray Menu Construction and Tracking Loop

Source path: `true-tick/apps/true-tick/src/tray/menu.rs`

```mermaid
flowchart TD
    A["show_menu entry"] --> B{"app.menu_active already set"}
    B -- "true" --> Z["return without opening"]
    B -- "false" --> C["GetCursorPos cursor point"]
    C -- "fails" --> CERR["record GetCursorPos.error"]
    CERR --> Z
    C -- "ok" --> D["work = monitor_work_area cursor"]
    D -- "None" --> E["work = primary_work_area"]
    D -- "Some" --> F["clamp_menu_anchor cursor to work"]
    E --> F
    F --> G["menu_anchor icon_rect or cursor"]
    G --> G2["icon_rect Some point is rect.top_left flags TPM_BOTTOMALIGN or TPM_LEFTALIGN"]
    G --> G3["icon_rect None point is cursor flags popup_track_flags edge"]
    G2 --> H{"loop over settings_submenu_open"}
    G3 --> H
    H -- "true" --> I["CreatePopupMenu settings root"]
    I --> J["append_menu_checked startup"]
    J --> J2["append_menu_checked automatic"]
    J2 --> J3["append_menu_checked auto_resume"]
    J3 --> J4["append_menu_checked battery_lockout"]
    J4 --> J5["append separator"]
    J5 --> J6["append ID_SETTINGS_RESET"]
    J6 --> K["popup_menus.root and settings set to submenu, track_target is submenu"]
    H -- "false" --> L["CreatePopupMenu root"]
    L --> M["menu_items_with_duration lifecycle config timing pause"]
    M --> N["append GitHub header id GITHUB_COMMAND_ID"]
    N --> N2["append separator"]
    N2 --> N3["append Start id ID_START, flags greying if disabled"]
    N3 --> N4["append Stop id ID_STOP, flags greying if disabled"]
    N4 --> O["append_schedule_submenu caption items 3"]
    O --> P["append_settings_submenu caption items 4"]
    P --> Q["append_status_submenu caption Status"]
    Q --> R["append separator"]
    R --> R2["append Logs id LOGS_COMMAND_ID"]
    R2 --> R3["append separator"]
    R3 --> R4["append Quit id ID_QUIT"]
    R4 --> S["popup_menus.root set, track_target is root"]
    K --> T["create_menu_help plus begin_popup_refresh_timer plus SetForegroundWindow"]
    S --> T
    T --> U["TrackPopupMenu track_target with anchor.flags or TPM_RIGHTBUTTON or TPM_RETURNCMD"]
    U --> V["destroy_menu_help plus kill_popup_refresh_timer plus DestroyMenu"]
    V --> W{"returned_menu_command"}
    W -- "0" --> W0["record TrackPopupMenu.result empty"]
    W0 --> Z
    W -- "None" --> Z
    W -- "Some command" --> Y{"menu_command_dispatch_allowed"}
    Y -- "false" --> Z
    Y -- "true" --> AA["match command to settings id set"]
    AA --> AB["handle_menu_command dispatch"]
    AB --> AB1["ID_START manual_start or ID_STOP manual_stop"]
    AB --> AB2["duration ids via duration_command then schedule_duration_action"]
    AB --> AB3["settings toggles startup automatic auto_resume battery_lockout"]
    AB --> AB4["preset ids via preset_command then schedule_preset_action"]
    AB --> AB5["ID_SETTINGS_RESET execute_reset relaunch"]
    AB --> AB6["ID_QUIT quit_decision then cleanup then PostQuitMessage"]
    AB --> AC{"menu_action_keeps_open"}
    AC -- "false" --> Z
    AC -- "true" --> AD["settings_submenu_open = is_settings_cmd"]
    AD --> H
    O --> Oa["append_duration_choice_submenu Start in"]
    O --> Ob["append_duration_choice_submenu Stop in"]
    O --> Oc["append_duration_choice_submenu Pause for"]
    O --> Od["append Presets id SCHEDULE_PRESETS_COMMAND_ID"]
    O --> Oe["append Cancel id CANCEL_SCHEDULED_COMMAND_ID"]
    O --> Of["append Resume id CANCEL_PAUSE_COMMAND_ID"]
    Oa --> OC1["preset_menu_items mapped to START_IN ids for 1 5 15 30 60"]
    Ob --> OC2["preset_menu_items mapped to STOP_IN ids for 1 5 15 30 60"]
    Oc --> OC3["preset_menu_items mapped to PAUSE_FOR ids for 5 15 30 60"]
    P --> PS1["append_menu_checked four config toggles"]
    PS1 --> PS2["append separator plus ID_SETTINGS_RESET"]
    Q --> QS1["status_menu_items rows state timing running_for next_action ownership"]
    QS1 --> QS2["all rows appended with MF_GRAYED"]
```

## Notes

- Root menu positions used by the refresh timer are `ROOT_MENU_START_POSITION` 2 and `ROOT_MENU_STOP_POSITION` 3.
- `ROOT_MENU_SETTINGS_POSITION` 5, `SCHEDULE_MENU_PRESETS_POSITION` 4 and `SETTINGS_MENU_RESET_POSITION` 5 are declared but marked `allow(dead_code)`.
- The settings submenu positions are `SETTINGS_MENU_AUTO_START_POSITION` 0, `SETTINGS_MENU_AUTO_TIME_POSITION` 1, `SETTINGS_MENU_AUTO_RESUME_POSITION` 2 and `SETTINGS_MENU_BATTERY_LOCKOUT_POSITION` 3.
- The schedule submenu positions are `SCHEDULE_MENU_CANCEL_POSITION` 6 and `SCHEDULE_MENU_RESUME_POSITION` 7.
- Commands that keep the popup open are reported by `menu_action_keeps_open`. Settings toggles reopen the loop with `settings_submenu_open` set to true.
- `handle_menu_command` returns false for the quit, reset and rejected paths, which ends the tracking loop.
- The anchor clamp uses `clamp_menu_anchor` with `max_x = work.right - 1` and `max_y = work.bottom - 1`, both floored at the work area left and top.
- Work area lookup uses `monitor_work_area` with `MONITOR_DEFAULTTONEAREST` and falls back to `primary_work_area` built from `SM_CXWORKAREA` and `SM_CYWORKAREA`.
- When the icon rect is available the menu grows upward and rightward from the icon top left corner. Otherwise it anchors at the clamped cursor with taskbar edge flags.
- `TrackPopupMenu` always adds `TPM_RIGHTBUTTON` and `TPM_RETURNCMD`, so the command id arrives as the return value, not as a `WM_COMMAND`.
- Tooltip handling creates a `tooltips_class32` window in `create_menu_help`, adds it with `TTM_ADDTOOLW`, tracks it with `TTM_TRACKPOSITION` and `TTM_TRACKACTIVATE` in `update_menu_help`, and tears it down in `destroy_menu_help`.
- `refresh_popup_menu` runs on `POPUP_REFRESH_TIMER_ID` and rewrites Start, Stop, the settings toggles, the schedule cancel and resume rows, and all greyed status rows via `ModifyMenuW`.