# Tray native FFI layer and helpers

Source path: `true-tick/apps/true-tick/src/tray/native.rs`

```mermaid
flowchart TD
    subgraph STRUCTS["repr C Structs and Handles"]
        S1["Point x y"]
        S2["Rect left top right bottom"]
        S3["ToolInfo cb_size flags hwnd id rect text"]
        S4["CreateStruct create_params and window fields"]
        S5["PaintStruct hdc erase paint rect"]
        S6["AppBarData cb_size hwnd edge rect"]
        S7["PopupMenuHandles root schedule settings status"]
        S8["NativeResult Succeeded or Failed raw_error"]
    end

    subgraph HELPERS["Pure Helpers"]
        H0["app_create_params create"] --> H0A{"create null"}
        H0A -->|yes| H0B["return null_mut"]
        H0A -->|no| H0C["deref create_params"]
        H1["native_bool_result result raw_error"] --> H1A{"result equals 0"}
        H1A -->|yes| H1B["Failed raw_error"]
        H1A -->|no| H1C["Succeeded"]
        H2["native_handle_result is_null raw_error"] --> H2A{"is_null"}
        H2A -->|yes| H2B["Failed raw_error"]
        H2A -->|no| H2C["Succeeded"]
        H3["wide value"] --> H3A["encode_utf16 plus nul terminator Vec u16"]
        H4["scale_logical value dpi"] --> H4A["saturating_mul dpi plus 95 then div 96"]
        H5["diagnostic_create_failure_result"] --> H5A["const -1"]
        H6["popup_track_flags edge horizontal vertical"] --> H6A{"edge ABE_RIGHT"}
        H6A -->|yes| H6B["horizontal = TPM_RIGHTALIGN"]
        H6A -->|no| H6C["keep horizontal"]
        H6B --> H6D{"edge ABE_BOTTOM"}
        H6C --> H6D
        H6D -->|yes| H6E["vertical = TPM_BOTTOMALIGN"]
        H6D -->|no| H6F["keep vertical"]
        H6E --> H6G["horizontal or vertical"]
        H6F --> H6G
    end

    subgraph SAFEWRAPS["Unsafe Convenience Wrappers"]
        W0["set_diagnostic_control_font control"] --> W0A{"control null"}
        W0A -->|yes| W0B["return"]
        W0A -->|no| W0C["GetStockObject DEFAULT_GUI_FONT"]
        W0C --> W0D{"font null"}
        W0D -->|no| W0E["SendMessageW WM_SETFONT font redraw 1"]
        W1["destroy_created_diagnostic_controls controls"] --> W1A["for each non-null control DestroyWindow"]
        W2["show_shutdown_warning hwnd message"] --> W2A["MessageBoxW MB_ICONWARNING"]
        W3["show_unclean_shutdown_warning"] --> W3A["spawn thread then MessageBoxW MB_OK or MB_ICONWARNING or MB_SETFOREGROUND owner null"]
        W4["taskbar_edge"] --> W4A["stack AppBarData cb_size set"]
        W4A --> W4B["SHAppBarMessage ABM_GETTASKBARPOS"]
        W4B --> W4C{"result 0"}
        W4C -->|yes| W4D["None"]
        W4C -->|no| W4E["Some data.edge"]
    end

    subgraph FFI["extern system Imports"]
        F1["user32 CreateWindowExW DefWindowProcW CallWindowProcW GetWindowLongPtrW SetWindowLongPtrW IsWindow IsIconic PostQuitMessage SetTimer KillTimer MessageBoxW"]
        F2["user32 menu CreatePopupMenu GetMenuItemCount GetMenuStringW AppendMenuW ModifyMenuW SetForegroundWindow TrackPopupMenu DestroyMenu"]
        F3["user32 window and paint DestroyWindow ShowWindow UpdateWindow BeginPaint EndPaint GetSysColor GetSysColorBrush SetTextColor SetBkColor SetWindowTextW GetWindowTextLengthW GetWindowTextW"]
        F4["user32 misc GetKeyState GetClientRect SendMessageW SetWindowPos InvalidateRect GetCursorPos GetModuleHandleW GetDpiForSystem GetSystemMetrics FillRect DrawFocusRect GetDC ReleaseDC SetCapture ReleaseCapture GetCapture"]
        F5["shell32 ShellExecuteW SHAppBarMessage"]
        F6["gdi32 GetStockObject"]
        F7["kernel32 GetLastError ExitProcess"]
    end
```

## Notes

- `wide` always appends one nul terminator and never interior nuls, matching the Win32 `LPCWSTR` contract for every call site in the tray.
- `scale_logical` rounds to nearest with the plus 95 half-step on a 96 DPI baseline and saturates on multiply so extreme DPI values cannot wrap.
- `popup_track_flags` only overrides horizontal alignment on a right-edge taskbar and vertical alignment on a bottom-edge taskbar, leaving caller flags intact otherwise.
- `show_unclean_shutdown_warning` spawns a thread because it runs before the message loop exists, so the modal box cannot block startup.
- `app_create_params` treats a null `CREATESTRUCT` pointer as a null create-params answer, keeping `WM_CREATE` handling panic free on synthetic messages.
- `destroy_created_diagnostic_controls` tolerates null entries and partial creation lists, matching the diagnostic window rollback path that passes only the controls built so far.
- `taskbar_edge` returns `None` when `SHAppBarMessage` fails so menu placement falls back to caller flags instead of guessing an edge.
