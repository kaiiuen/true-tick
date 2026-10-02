# Rubber-Band Marquee Selection in marquee.rs

Source path: `true-tick/apps/true-tick/src/ui/marquee.rs`

```mermaid
flowchart TD
    P["diagnostic_list_proc subclass of SysListView32"] --> M{"message"}
    M -- "WM_LBUTTONDOWN" --> LD["forward to prev proc or DefWindowProcW"]
    LD --> MOD{"shift or ctrl held"}
    MOD -- "yes" --> NAT["native range or toggle selection only"]
    MOD -- "no" --> ARM["handle_marquee_lbuttondown"]
    ARM --> ARMOK{"app present and VK_LBUTTON down"}
    ARMOK -- "no" --> SKIP["return without arming"]
    ARMOK -- "yes" --> PEND["pending true anchor pt current pt"]
    M -- "WM_MOUSEMOVE" --> HELD{"MK_LBUTTON set and VK_LBUTTON down"}
    HELD -- "no" --> CAN1["handle_marquee_cancel then forward"]
    HELD -- "yes" --> PMOVE{"pending"}
    PMOVE -- "yes" --> THRESH["handle_marquee_pending_mousemove"]
    THRESH --> DRAG{"drag exceeds SM_CXDRAG or SM_CYDRAG"}
    DRAG -- "no" --> WAIT["stay pending"]
    DRAG -- "yes" --> ACT["pending false active true SetCapture"]
    ACT --> INIT["store initial_selected row positions"]
    INIT --> UPD1["update_marquee_selection"]
    PMOVE -- "no" --> AMOVE{"active"}
    AMOVE -- "yes" --> MMOVE["handle_marquee_mousemove erase then redraw band"]
    MMOVE --> UPD2["update_marquee_selection"]
    AMOVE -- "no" --> FWD1["forward to prev proc"]
    UPD1 --> SEL["count rows LVM_GETITEMCOUNT then LVM_GETITEMRECT LVIR_BOUNDS"]
    UPD2 --> SEL
    SEL --> PICK{"additive when VK_CONTROL down"}
    PICK -- "yes" --> UNI["select in band plus initial_selected"]
    PICK -- "no" --> BAND["select rows intersecting band"]
    UNI --> SETSEL["set_list_view_item_selected each row"]
    BAND --> SETSEL
    SETSEL --> GRID["update_diagnostic_grid_selection"]
    M -- "WM_LBUTTONUP" --> LUP{"active"}
    LUP -- "yes" --> REL["handle_marquee_lbuttonup"]
    REL --> CLEAN["erase rect active false clear initial ReleaseCapture InvalidateRect"]
    CLEAN --> GRID
    LUP -- "no" --> PENDC["pending false then forward"]
    M -- "WM_CAPTURECHANGED" --> CC["handle_marquee_capturechanged"]
    CC --> CCD{"active"}
    CCD -- "yes" --> CLEAN2["erase rect active false clear InvalidateRect"]
    CCD -- "no" --> CCDN["clear pending only"]
    CLEAN2 --> GRID
    M -- "WM_RBUTTONDOWN or WM_MBUTTONDOWN" --> CAN2["cancel pending or active then forward"]
    M -- "WM_KEYDOWN" --> ESC{"VK_ESCAPE and pending or active"}
    ESC -- "yes" --> CAN3["handle_marquee_cancel"]
    CAN3 --> CAN4{"active"}
    CAN4 -- "yes" --> REL
    CAN4 -- "no" --> JUSTP["clear pending only"]
    ESC -- "no" --> FWD2["forward to prev proc"]
    M -- "any other message" --> FWD2
```

## Notes

- The module subclasses the native SysListView32 diagnostic list so a plain left button drag draws a focus rect band and selects every row it intersects.
- `diagnostic_list_proc` resolves the owning `App` through `app_from_list`, which reads `GWLP_USERDATA` with `GetWindowLongPtrW`.
- Every `WM_LBUTTONDOWN` is forwarded to the previous proc first so the native list can set focus and the selection anchor needed for Shift click range selection.
- The custom marquee only arms when neither Shift nor Ctrl is held, since `should_use_native_click_selection` returns true for either modifier.
- A press only arms `diagnostic_marquee_pending` when `GetKeyState(VK_LBUTTON)` reports the button still down, which filters out already released clicks.
- A pending drag becomes active only after `marquee_drag_exceeded` clears the `SM_CXDRAG` or `SM_CYDRAG` threshold, and each metric is clamped to a minimum of 1.
- Activation calls `SetCapture` and snapshots the current selection via `list_selected_item_positions` into `diagnostic_marquee_initial_selected`.
- `handle_marquee_mousemove` erases the old focus rect with `draw_marquee_rect`, moves `diagnostic_marquee_current`, then redraws the new band.
- `draw_marquee_rect` normalizes the band with `points_to_rect`, skips zero width or zero height rects, and draws with `DrawFocusRect` inside a `GetDC` and `ReleaseDC` pair.
- `update_marquee_selection` reads row counts and row bounds from `LVM_GETITEMCOUNT` and `LVM_GETITEMRECT` with `LVIR_BOUNDS`, failing rows are pushed as an empty offscreen rect.
- `marquee_selected_indices` selects rows intersecting the band and, when Ctrl is held, unions them with the initial selection so additive marquee keeps prior rows.
- Release, capture change, right or middle button press, and Escape all end the drag through the cancel or lbuttonup path, which clears state, releases capture, invalidates the list, and refreshes the grid selection.
