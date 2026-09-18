# True™ Tick

True™ Tick is a high-assurance, portable Windows notification area application for managing and explaining user-mode timer resolution. Engineered with mission-critical systems rigor (adapted from DO-178C avionics and IEC 62304 medical software standards), True Tick acquires, verifies, and releases exclusively its own tracked timer resolution request without interfering with external system clients or injecting invasive kernel drivers.

---

## 1. High-Assurance Architecture Principles

1. **Closed-Loop Causality and Verification**:
   Every state-changing operation executes through a four-stage transactional cycle: **preflight -> apply -> verify -> postcondition**. True Tick never assumes a kernel request succeeded based on API return values alone, instead performing an independent postcondition query to verify hardware clock state.

2. **Strict Own-Request Ownership**:
   True Tick tracks its own requested contribution independently. Upon exit or policy release, it releases only its own request and never attempts to force a guessed global default (such as 15.625 ms), preserving the resolution required by other active processes.

3. **Conservative Power Governance**:
   Operating under strict battery conservation invariants, True Tick automatically releases high-resolution requests when transitioning to DC battery power. Battery Saver mode acts as an unconditional, non-overridable veto against high-frequency timer requests. When returning to AC power, active sessions cleanly reacquire timing via debounced power reconciliation.

4. **Forensic Auditability**:
   Every operational step, window message dispatch, and kernel return is recorded in a 13-column RFC 4180 CSV log protected by continuous SHA-256 cryptographic hash chaining, ensuring complete post-mortem auditability and tamper evidence.

---

## 2. Notification Area Interface

True Tick operates as a native, lightweight notification area icon designed for maximum efficiency and zero UI latency:

```
True™ Tick v<version>          [Clickable package identity header]
-----------------------------
Start                         [Direct manual timing acquisition]
Stop                          [Guarded timing release]
Schedule >                    [Duration scheduling and pause controls]
Settings >                    [Atomic configuration toggles]
Status >                      [Read-only telemetry rows]
-----------------------------
Logs                          [Opens diagnostic report window]
-----------------------------
Quit                          [Guarded exit with release confirmation]
```

### 2.1 Persistent Settings Submenu
The `Settings >` submenu aggregates configuration toggles:
- **`Auto-start: On/Off`**: Manages Windows login startup via the CurrentUser Run key.
- **`Auto-time: On/Off`**: Enables automatic timing acquisition when AC power is detected.
- **`Auto-resume on AC: On/Off`**: Restores active manual sessions upon reconnecting to AC power.

Toggling any setting inside `Settings >` executes an atomic configuration write to `true-tick.toml` and re-opens the submenu at the exact cursor anchor coordinates, allowing rapid configuration adjustments without closing the menu.

---

## 3. High-Precision Metrology

Windows timer interrupt intervals are represented using integer 100-nanosecond units (HNS):
- **Default Resolution**: Nominal 15.625 ms (156,250 HNS, 64.0 Hz interrupt frequency).
- **Target Resolution**: 0.5000 ms (5,000 HNS, 2,000 Hz interrupt frequency).
- **Four-Decimal Metrology**: Displayed values are rendered to four decimal places (`{}.{:04}`), matching the physical 100-nanosecond kernel boundary without floating-point rounding errors.
- **Hardware PLL Tolerance**: Motherboards with fractional BCLK frequencies or spread-spectrum clocking frequently yield slight divisor variations (e.g. 5033 HNS for a 5000 HNS request). True Tick incorporates a formal `HARDWARE_TIMER_TOLERANCE_HNS = 100` ($10\,\mu\text{s}$) window to accept physical crystal frequency variations as valid satisfied states.

---

## 4. Documentation Suite

True Tick maintains a comprehensive, modular documentation suite organized into dedicated engineering specifications:

| Document | Focus and Contents |
| :--- | :--- |
| [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) | Rust toolchain, building, workspace crates, unit tests, and CI style guards |
| [`docs/PORTABLE_PACKAGING.md`](docs/PORTABLE_PACKAGING.md) | Portable directory topography, A/B partition mechanics, and launcher supervisor |
| [`docs/UI_AND_INTERACTION.md`](docs/UI_AND_INTERACTION.md) | Menu state machine, duration presets dialog, status rows, and tooltip contracts |
| [`docs/WINDOWS_NATIVE_LOADER.md`](docs/WINDOWS_NATIVE_LOADER.md) | Common Controls v6 embedded manifest specification and PE loader verification |
| [`docs/HIGH_ASSURANCE_TELEMETRY.md`](docs/HIGH_ASSURANCE_TELEMETRY.md) | 13-column forensic schema, SHA-256 hash chaining, and synchronous flushes |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | System architecture overview, component boundaries, and runtime invariants |
| [`STATUS.md`](STATUS.md) | Active implementation status, test metrics, and verification results |

For complete formal engineering volumes (including the Software Requirements Specification, Traceability Matrix, Mathematical State-Transition Matrix, and System Hazard FMEA), refer to the workspace planning repository under `docs/projects/true-tick/`.

---

## 5. License and Governance

True Tick is developed under a source-available, non-commercial governance model. Commercial deployment, binary redistribution, and embedding in proprietary software requires explicit authorization from the copyright holder.

All documentation, commit messages, and internal notes adhere strictly to the Kaiwentek style guard: zero em-dashes and zero semicolons.
