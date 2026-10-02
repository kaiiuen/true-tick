# Presets Window

Source path: `true-tick/apps/true-tick/src/ui/presets_window.rs`

```mermaid
flowchart TD
    A["presets_window_proc"]
    A --> B{"WM_CREATE"}
    B -- "yes" --> D["fetch create params"]
    B -- "no" --> C{"app is null"}
    D --> E{"params null"}
    E -- "yes" --> F["return diagnostic failure"]
    E -- "no" --> G["store app in GWLP_USERDATA"]
    G --> H["scale layout for DPI"]
    H --> I["create listbox, label, input, 4 buttons"]
    I --> J{"any control null"}
    J -- "yes" --> K["record error, destroy controls"]
    K --> F
    J -- "no" --> L["set fonts, store handles"]
    L --> M["populate presets list"]
    M --> N["return 0"]
    C -- "yes" --> O["DefWindowProcW"]
    C -- "no" --> P{"WM_ERASEBKGND"}
    P -- "yes" --> Q["fill window brush, return 1"]
    P -- "no" --> R{"WM_PAINT"}
    R -- "yes" --> S["begin paint, fill, end paint, return 0"]
    R -- "no" --> T{"CTLCOLOR edit or static"}
    T -- "yes" --> U["set colors, return brush"]
    T -- "no" --> V{"WM_COMMAND"}
    V -- "yes" --> W{"BN_CLICKED"}
    V -- "no" --> X{"WM_DPICHANGED"}
    W -- "yes" --> Y{"command id"}
    W -- "no" --> Z{"listbox selchange"}
    Y -- "Add" --> AA["presets_add"]
    Y -- "Delete" --> AB["presets_delete_selected"]
    Y -- "Reset" --> AC["presets_reset"]
    Y -- "Close" --> AD["DestroyWindow"]
    Z -- "yes" --> AE["record selection, return 0"]
    Z -- "no" --> AF{"input EN_CHANGE"}
    AF -- "yes" --> AG["return 0"]
    AA --> AH{"parse duration ok"}
    AH -- "no" --> AI["record invalid"]
    AH -- "yes" --> AJ{"manager add ok"}
    AJ -- "no" --> AK["record rejected"]
    AJ -- "yes" --> AL["persist_presets"]
    AL --> AM["populate list, clear input"]
    AM --> AN["return 0"]
    AB --> AO{"selection valid"}
    AO -- "no" --> AP["record no selection"]
    AO -- "yes" --> AQ["manager remove"]
    AQ --> AR["persist_presets"]
    AR --> AS["populate list, return 0"]
    AC --> AT["reset defaults"]
    AT --> AU["persist_presets"]
    AU --> AV["populate list, return 0"]
    X -- "yes" --> AW["set window pos, return 0"]
    X -- "no" --> AX{"WM_CLOSE"}
    AX -- "yes" --> AD
    AX -- "no" --> AY{"WM_DESTROY or WM_NCDESTROY"}
    AY -- "yes" --> AZ["clear handles and userdata"]
    AY -- "no" --> BC["DefWindowProcW"]
    AZ --> BA{"WM_NCDESTROY"}
    BA -- "yes" --> BB["return 0"]
    BA -- "no" --> BC
```

## Notes

- Window class is `TrueTickPresetsClass`, sized 380 by 360, created at 140, 140 with caption, system menu, clip children and clip siblings styles.
- `open_presets_window` focuses an existing live window, restores it if iconic, and clears stale handles when `IsWindow` fails.
- Children are created in `WM_CREATE`: a listbox, a static label, an edit input, and Add, Delete, Reset, Close buttons.
- The listbox uses `LBS_NOTIFY` and `WS_VSCROLL`. The edit input uses `ES_AUTOHSCROLL` with `EM_LIMITTEXT` of 64 characters.
- Layout values are scaled per DPI with `scale_logical`, and the button row width is split into four equal columns.
- The `App` pointer is stored in `GWLP_USERDATA` during `WM_CREATE` and read at the top of every later message.
- Add reads the input text, parses it with `DurationPreset::parse`, accepts formats like 15m or 1h 30m, then adds, persists, repopulates, and clears the input.
- Delete reads the current index with `LB_GETCURSEL` and rejects the request when the selection is `LB_ERR` or negative.
- Reset restores factory defaults through `presets_manager.reset_defaults`.
- Every successful add, delete, or reset calls `persist_presets`, which saves `schedule_presets_seconds` atomically and records a failed diagnostic without changing config on error.
- Listbox selection changes record `presets.selection` with the index, and edit input changes are ignored.
- `WM_DESTROY` clears the stored handles, and `WM_NCDESTROY` clears handles and userdata and returns 0 to end the window proc.
