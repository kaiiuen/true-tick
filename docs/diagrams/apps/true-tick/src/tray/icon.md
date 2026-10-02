# Tray Icon Rendering, Caching, and Notification Transitions

Source path: `true-tick/apps/true-tick/src/tray/icon.rs`

```mermaid
flowchart TD
    subgraph S1["DPI Resolution and Canvas Clamping"]
        W1["Window Handle hwnd"] --> W2["dpi_for_window"]
        W2 --> W3{"GetDpiForWindow"}
        W3 -- "0" --> W4["Default DPI 96"]
        W3 -- "non-zero" --> W5["Active DPI"]
        W4 --> D1["dpi_to_canvas_size"]
        W5 --> D1
        D1 --> D2["dpi_to_icon_canvas"]
        D2 --> C1["clamp_canvas_size"]
        C1 --> C2{"Match canvas"}
        C2 -- "16 20 24 32 64" --> C3["Preserve canvas size"]
        C2 -- "other values" --> C4["Clamp to default 16"]
    end

    subgraph S2["Cache Lookup and LRU Promotion"]
        T1["TrayStatus"] --> K1["icon_pixel_color"]
        C3 --> K2["Build IconCacheKey color canvas"]
        C4 --> K2
        K1 --> K2
        K2 --> L1["cached_icon_handle"]
        L1 --> L2["Acquire ICON_CACHE mutex"]
        L2 --> L3{"Iterate position key match"}
        L3 -- "Hit Some position" --> L4["cache remove position"]
        L4 --> L5["cache push entry to back"]
        L5 --> L6["Return Some handle"]
        L3 -- "Miss None" --> L7["Return None"]
    end

    subgraph S3["Icon Generation with Stride Math"]
        L7 --> G1["status_icon"]
        G1 --> G2["vec color canvas squared 32bpp"]
        G1 --> G3["mask_buffer_len canvas"]
        G3 --> G4["canvas plus 31 div 32 mul 4 mul canvas"]
        G4 --> G5["vec 0u8 1bpp AND mask"]
        G2 --> B1["CreateBitmap canvas canvas 1 32 pixels"]
        B1 --> B2{"bitmap is null"}
        B2 -- "true" --> B3["Err GetLastError"]
        B2 -- "false" --> M1["CreateBitmap canvas canvas 1 1 mask"]
        M1 --> M2{"mask_bitmap is null"}
        M2 -- "true" --> M3["DeleteObject color bitmap, Err GetLastError"]
        M2 -- "false" --> I1["Build IconInfo f_icon 1 hotspots 0"]
        I1 --> I2["CreateIconIndirect info"]
        I2 --> I3{"icon is null"}
        I3 -- "true" --> I4["DeleteObject both bitmaps, Err GetLastError"]
        I3 -- "false" --> I5["DeleteObject both bitmaps, Ok HICON"]
    end

    subgraph S4["Cache Insertion and Eviction"]
        I5 --> A1["cache_icon_handle"]
        A1 --> A2["Acquire ICON_CACHE mutex"]
        A2 --> A3{"Iterate position key match"}
        A3 -- "Found existing key" --> A4["cache remove position, update handle, push back"]
        A3 -- "New key" --> E1{"cache len gte MAX_ICON_CACHE_ENTRIES 32"}
        E1 -- "true" --> E2["cache remove index 0 LRU entry"]
        E2 --> E3["destroy_icon evicted handle DestroyIcon"]
        E3 --> E4["cache push new CachedIcon"]
        E1 -- "false" --> E4
    end

    subgraph S5["Shell_NotifyIcon NIM_MODIFY Guard"]
        L6 --> U1["update_icon"]
        A4 --> U1
        E4 --> U1
        U1 --> U2{"replacement equals icon h_icon"}
        U2 -- "Identical handle" --> R1["refresh_tooltip_only"]
        R1 --> R2["Format sz_tip encode_utf16"]
        R2 --> R3["Shell_NotifyIconW NIM_MODIFY"]
        R3 --> R4{"native_bool_result"}
        R4 -- "Ok" --> R5["Return Ok"]
        R4 -- "Failed" --> R6["Return Err GetLastError"]
        U2 -- "Different handle" --> T2["Stash old_icon and old_tip"]
        T2 --> T3["Assign replacement icon and update sz_tip"]
        T3 --> T4["Shell_NotifyIconW NIM_MODIFY"]
        T4 --> T5{"native_bool_result"}
        T5 -- "Ok" --> T6["Return Ok retaining cached handles"]
        T5 -- "Failed" --> T7["Rollback icon h_icon and sz_tip, Return Err GetLastError"]
    end
```

## Notes

- DPI translation falls back to 96 when `GetDpiForWindow` returns 0 for a window handle.
- Canvas dimensions are clamped strictly to valid buckets 16, 20, 24, 32, and 64 pixels with any unexpected size collapsing to 16.
- The 1bpp AND mask scan lines are padded to 4 byte boundaries using `(((canvas + 31) / 32) * 4) * canvas` bytes.
- The 32bpp color bitmap buffer allocates `canvas * canvas` elements filled with `icon_pixel_color` for opaque status visuals.
- GDI bitmaps are cleaned up via `DeleteObject` unconditionally after `CreateIconIndirect` completes or fails.
- The cache lookup promotes accessed entries to the back of the vector to maintain recency ordering.
- Cache capacity is bounded at 32 entries, evicting index 0 and releasing the Win32 handle with `destroy_icon` when full.
- Updating an existing cache key updates its handle and shifts the entry to the most recently used tail position.
- Unchanged icon handles skip icon handle swapping and call `refresh_tooltip_only` to update only `sz_tip`.
- Failed `Shell_NotifyIconW` calls during icon updates restore the previous `h_icon` and `sz_tip` state.
- `NotifyIconData` drop zeros its `h_icon` field so cached handles remain owned by `ICON_CACHE` without premature destruction.
- Shutdown releases all cached icons in `clear_icon_cache` after tray registration removal via `NIM_DELETE`.
