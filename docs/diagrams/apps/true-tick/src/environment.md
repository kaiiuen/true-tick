# environment.rs Diagram

Source: `apps/true-tick/src/environment.rs`

```mermaid
flowchart TD
    A["collect_environment_snapshot()"] --> B["os_version()"]
    A --> C["power_fields()"]
    A --> D["cpu_arch_code()"]
    A --> E["available_parallelism()"]
    A --> F["process_uptime_ms()"]

    B -->|windows| B1["RtlGetVersion() via ntdll"]
    B -->|not windows| B2["return (0, 0, 0)"]
    B1 -->|status != 0| B3["return (0, 0, 0)"]
    B1 -->|ok| B4["(major, minor, build)"]

    C -->|windows| C1["GetSystemPowerStatus() via kernel32"]
    C -->|not windows| C2["return (0, 0)"]
    C1 -->|fail| C3["return (0, 0)"]
    C1 -->|ok| C4["map ac_line_status 0 or 1"]
    C4 --> C5["map battery_life_percent 0..=100"]
    C5 --> C6["(ac_line, battery)"]

    D --> D1["match std::env::consts::ARCH"]
    D1 --> D2["x86=0x014c x86_64=0x8664 arm=0x01c0 aarch64=0xaa64 else 0"]

    E --> E1["count or 0"]

    F --> F1["PROCESS_EPOCH OnceLock"]
    F1 --> F2["elapsed ms as u64 saturating"]

    B4 --> G["EnvironmentSnapshot"]
    B3 --> G
    B2 --> G
    C6 --> G
    C3 --> G
    C2 --> G
    D2 --> G
    E1 --> G
    F2 --> G
    A -->|os_ubr always 0| G
```

## Notes

- `collect_environment_snapshot` fills every `EnvironmentSnapshot` field except `os_ubr`, which is hardcoded to 0.
- OS version comes from `RtlGetVersion` in `ntdll` on Windows, returning `(0, 0, 0)` on failure.
- On non-Windows platforms `os_version` and `power_fields` both return zeros.
- Power status uses `GetSystemPowerStatus` from `kernel32` on Windows.
- `ac_line_status` is kept only when the API reports 0 or 1, otherwise it falls back to 0.
- `battery_percent` is kept only in the 0..=100 range, otherwise it falls back to 0.
- `cpu_arch` maps `std::env::consts::ARCH` to IMAGE_FILE_MACHINE style codes, unknown arches map to 0.
- `processor_count` uses `std::thread::available_parallelism`, mapping failure to 0.
- `uptime_ms` measures elapsed time from a lazily initialized `Instant` stored in a `OnceLock`.
- Uptime conversion saturates at `u64::MAX` on overflow.
- Two tests verify processor count plus arch code, and monotonic uptime.
