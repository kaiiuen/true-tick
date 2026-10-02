# tick-observation-windows (lib)

Source: `true-tick/crates/tick-observation-windows/src/lib.rs`

```mermaid
flowchart TD
    A["Power broadcast event from caller"] --> B["refresh_power"]
    A --> C["next_event stub"]
    C --> D["Err(CoreError::Unsupported) for events"]
    B --> E{"cfg target windows?"}
    E -->|"no"| F["Err(CoreError::Unsupported)"]
    E -->|"yes"| G["query_power"]
    G --> H["Seed SystemPowerStatus with 255 sentinels"]
    H --> I["GetSystemPowerStatus with pointer to status"]
    I --> J{"call return value not zero?"}
    J -->|"no"| K["Err(ObservationFailed with GetLastError)"]
    J -->|"yes"| L["battery_saver = Some(flag bit 0)"]
    L --> M{"ac_line_status"}
    M -->|"1"| N["base_state = Ac"]
    M -->|"0"| O["base_state = Battery"]
    M -->|"other"| P["base_state = Unknown"]
    N --> Q{"battery_saver is Some(true)?"}
    O --> Q
    P --> Q
    Q -->|"yes"| R["state = BatterySaver"]
    Q -->|"no"| S["state = base_state"]
    R --> T["Ok(PowerSnapshot)"]
    S --> T
    T --> U["apply_power_query_result"]
    K --> U
    F --> U
    U --> V{"result is Ok?"}
    V -->|"yes"| W["Cache snapshot and clear power_error"]
    V -->|"no"| X["Cache Unknown and None and store power_error"]
    W --> Y["power returns cached snapshot"]
    X --> Z["power_error returns recorded error"]
```

## Notes

- The adapter is event driven, doing no polling and no background timers, so a refresh only happens when the caller asks.
- `PowerSnapshot` holds a `PowerState` plus an optional `battery_saver` flag, and it is `Clone`, `Copy`, `Debug`, `Eq`, and `PartialEq`.
- `ObservationSource` declares `next_event`, `power`, and `refresh_power`, while `next_event` stays a stub returning `CoreError::Unsupported`.
- `WindowsObservation::default` starts at `PowerState::Unknown` with `battery_saver` set to `None` and no recorded error.
- `refresh_power` compiles to `query_power` under `cfg(windows)` and returns `CoreError::Unsupported` on every other target.
- `query_power` calls `GetSystemPowerStatus` through a raw `extern "system"` binding in `kernel32`.
- The `SystemPowerStatus` struct is pre-seeded with 255 sentinel values so an unfilled field reads as unknown rather than as a real reading.
- A zero return from the call becomes `CoreError::ObservationFailed` carrying the raw value from `GetLastError`.
- `ac_line_status` maps 1 to `Ac`, 0 to `Battery`, and any other value to `Unknown`.
- The battery saver flag is read only from bit 0 of `system_status_flag` and, when true, overrides the base state with `PowerState::BatterySaver`.
- `apply_power_query_result` fails closed, clearing the cached snapshot to `Unknown` and `None` and storing the error before returning it.
- A successful result replaces the cached snapshot and clears any earlier `power_error`, and the tests cover stale power clearing, error clearing, and the non-windows unsupported path.
