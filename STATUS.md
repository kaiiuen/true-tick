# Status

**Phase:** internal v1 implementation, no release

## Current lifecycle contract

The timer lifecycle controls the user-visible state. An active release handoff is always yellow `Stopping, handoff` with concise text such as `True™ Tick: Stopping, handoff · 0.4966 ms`. Startup registration, configuration persistence, logging, and other diagnostics are separate evidence and cannot publish red lifecycle failure over a live handoff. The icon color is derived from the same lifecycle state as the text. Handoff completion clears the tracker and publishes red `Stopped` with actual current effective timing in the standard `True™ Tick: Stopped · <timing>` tooltip format. A bounded timeout clears the tracker and publishes red `Stopped` with actual current effective timing while ownership remains released. The tooltip always uses `True™ Tick: <state> · <timing>`, and parenthetical forms such as `Stopped (current: X ms)` or `Stopped (external: X ms)` do not exist anywhere in the application.

Desired intent is latest-wins `Acquire` or `Release`. Start, Stop, Auto-time, and power changes use this queue and the same serialized ownership path. A request during `Starting` or `Stopping` is retained until the transition completes or times out. Repeated requests do not issue duplicate native calls. A panic path must attempt an explicit release, and startup must presume zero prior ownership without writing a guessed default. Quit uses ownership, handoff, and uncertain state together, so it cannot exit while cleanup remains unresolved.

One synchronized timing snapshot supplies the tray tooltip, menu status, diagnostic header, handoff classification, and logs. Requested, selected, effective, bounds, raw status, and validity are distinct fields. A failed current query clears effective timing to unknown. Handoff polling observes the stored release boundary without selecting a new interval. Live interval selection remains query-driven. Zero is the automatic sentinel. The named legacy migration value is not a live request default. Resolution examples and fixtures are restricted to tests and migration material. Handoff interval and poll budget are separate handoff policy values.

The workspace builds an internal tray-only v1 with focused deterministic tests. PE inspection confirmed the pre-main loader failure: `target/debug/true-tick.exe` imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, without an embedded application manifest. Windows therefore loaded legacy Common Controls without version 6 activation and failed with `STATUS_ORDINAL_NOT_FOUND`. The tray package now embeds a checked-in manifest through its package-local `build.rs`. Before diagnostic or tooltip controls are created, the tray calls the explicit `InitCommonControlsEx` FFI from `comctl32.dll` once with list-view and the standard bar-class flags that include tooltips. Initialization failure records the raw Win32 error and aborts with a clear diagnostic window failure. The embedded manifest remains required because it activates Common Controls v6 for the `SysListView32` Logs grid. The Launcher does not import `TaskDialogIndirect` or common controls version 6 APIs, so it does not receive this target-specific integration.

The timer adapter explicitly links `ntdll`, passes Windows BOOLEAN as `u8` values `1` or `0`, and preserves signed `i32` NTSTATUS values. It retains the raw `minimum` and `maximum` output labels and values from `NtQueryTimerResolution` for diagnostics, then selects the numerically smallest supported current boundary in automatic mode before inclusive validation. The labels are API field names, not an ordering guarantee. The captured values `minimum_hns=156250`, `maximum_hns=5000`, `current_hns=9966`, and old `requested_hns=10000` explain the observed effective value of `0.9966 ms`. Legacy `request_interval_hns=10000` is migrated to automatic selection. In that captured case, automatic selection would record `selected_hns=5000`, or `0.5000 ms`, if the current system reports that boundary. This is not a universal fixed value. Manual kernel32 declarations for file replacement, power observation, last-error retrieval, and module-path lookup explicitly link `kernel32`. Static PE inspection is deterministic and non-running. Runtime Windows resolution of the loader failure remains unverified until the user runs the rebuilt tray binary.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior. A postcondition mismatch preserves
uncertain ownership and suppresses duplicate acquisition until controlled cleanup
recovers or reports the state. Hardware motherboard crystal divisor and phase-locked loop quantization can cause the Windows kernel to return a resolution slightly above the nominal requested value, such as 5033 HNS when requesting 5000 HNS (representing 3.3 microseconds of hardware jitter). True Tick defines HARDWARE_TIMER_TOLERANCE_HNS = 100 (10 microseconds) so small physical overshoots within tolerance are verified as satisfied rather than rejected as unverified errors. Normal message-loop shutdown attempts centralized
cleanup once and keeps an unverified warning if release is not confirmed. Tray Quit now logs the request, active-state decision, dialog result, cleanup result, and exit permission. An owned request uses `True™ Tick is currently controlling timer resolution. Stop timing and quit?`. Uncertain ownership uses `True™ Tick could not verify that timing is fully released. Keep the app open and retry cleanup?`. `Yes` maps to Stop and Quit. `No`, close, zero, unknown, and MessageBox failure map to Cancel. A released post-release handoff is observational only, records that external timing remains, stops its watcher, and allows normal exit. Failed owned cleanup keeps the app alive, updates the icon and diagnostic log, and allows retry. A definitely stopped state with no pending ownership exits without a warning. Opt-in current-user boot startup registration is isolated behind its own Windows
adapter. Portable A/B mode targets the buildable `Launcher.exe` entry point. The
launcher requires valid active-slot metadata, validates the named A or B
executable, and launches that slot before activation. A normal debug run uses a
separate fallback that registers the actual `target/debug/true-tick.exe` tray
executable when no portable launcher path is available. Registration is distinct
from automatic timer activation after the application has launched. Secure
signatures and rollback are not implemented.

The tray starts with a clickable `True™ Tick v<version>` header sourced from Cargo
package metadata. Root menu order is `True™ Tick v<version>`, a separator,
`Start`, `Stop`, `Schedule >`, a separator, `Auto-start: On/Off`,
`Auto-time: On/Off`, a separator, a read-only `Status >` submenu, `Logs`, a
separator, and `Quit`. Start and stop are primary and stay at the top. Scheduling
and pausing are secondary and now share one submenu instead of two, so there is
a single duration surface. `Schedule >` contents, in order, are `Start in >`,
`Stop in >`, `Pause for >`, `Interval presets...`, a separator, `Cancel scheduled
action`, and `Resume now`. The declared menu vector in `tray_surface.rs` and the
rendered menu both use `Pause for >`. Pause presets are 5 minutes, 15 minutes,
30 minutes, and 1 hour by default. Schedule has Start in, Stop in, Pause for,
`Interval presets...`, Cancel scheduled action, and `Resume now` when a pause is
active. Each duration submenu is generated dynamically from the active preset
list rather than a fixed command table, so custom intervals appear in Start in,
Stop in, and Pause for without rebuilding the root menu. Dynamic preset command
identifiers occupy bounded per-action ranges, so preset identity is stable while
the menu is open. `Interval presets...` opens a compact native window registered
under the `TrueTickPresetsClass` class and titled `True™ Tick Interval Presets`.
The window provides a ListBox showing the active presets in ascending order, a
hinted input line for adding or editing an interval, and Add, Delete, Reset, and
Close buttons. The input accepts natural unit text such as `45m`, `2h`, `10`,
`30s`, and `1h30m`, where a bare number is interpreted as minutes. Every preset
is a whole-second value from 10 seconds to 24 hours. The preset list is capped
at 12 entries, deduplicated, and kept sorted. Delete requires a selection and
always leaves at least one preset. Reset restores the factory list of 1 minute,
5 minutes, 15 minutes, 30 minutes, and 1 hour. Add, Delete, and Reset persist
the resulting preset list to `schedule_presets_seconds` in `true-tick.toml`
through the same atomic temporary-file replacement used by other settings, so
custom presets survive restarts. A `ScheduledAction` stores its
`DurationPreset` directly, so custom intervals participate in scheduling with
their exact seconds value and no clamping to a fixed choice table. Pause
stacking, an indefinite pause, and overlapping scheduled actions remain absent.
Start is explicitly disabled
while a pause is active. Auto-start launches the app at Windows login. Auto-time
controls automatic timing acquisition and defaults to off. Both tray release
notifications open the same compact menu. Button-down and double-click
notifications are ignored to avoid duplicate menus.

Status contains disabled State, Timing, Running for, Next action, and Ownership rows.
Each row has a stable read-only menu identity so `WM_MENUSELECT` can show a concise
hover description without enabling the row. The descriptions are `Current True Tick
lifecycle state`, `Latest verified effective timing observation`, `Elapsed time since
verified running`, `Scheduled action and remaining time`, and `True Tick ownership
versus external timing`. Disabled rows cannot dispatch actions or open Logs.
`Logs` is a separate top-level item below `Status >`, not a row inside Status. Status and Logs do not change timer state.
The Status Timing row uses the current authoritative effective snapshot and shows `Timing unknown`
when evidence is invalid or stale. A valid row uses the detailed four-decimal form with raw HNS units, for example `0.4966 ms (4966 HNS)`. Running for uses monotonic time from the last
verified Running transition. The `Logs` command opens a normal taskbar diagnostic
window titled `True™ Tick Status and Diagnostics` without changing timer state. It
uses a normal overlapped style, `WS_EX_APPWINDOW`, no `WS_EX_TOOLWINDOW`, no child
style, no owner, standard title-bar controls, a resizable read-only status and
session log view, and a fresh snapshot each time it is reopened. The native menu
keeps Start, Stop, and both setting toggles open after successful or failed handling.
Reopening after any persistent command reuses the original popup anchor POINT, and
the returned `TPM_RETURNCMD` ID is dispatched once. Highlighting a command uses
`WM_MENUSELECT` tooltips for Duration, Start in, Stop in, Pause for, Cancel scheduled
action, Status, Logs, Start, Stop, settings, and Quit. The tooltip is destroyed
when the popup closes and does not change timer state. Cancel keeps the Quit command
loop available, while successful Quit closes it.

Start and Stop use the same guarded policy and ownership lifecycle as automatic
activation. Start remains yellow through query, request, and verification, then
turns green only after a verified request. The tray tooltip uses actual concise
values such as `True™ Tick: Running · 0.4966 ms`, `True™ Tick: Stopped · 0.9966 ms`,
`True™ Tick: Starting · 0.9966 ms`, `True™ Tick: Stopping · 0.4966 ms`,
`True™ Tick: Warning`, and `True™ Tick: Error`. Scheduled and paused states use
`True™ Tick: Starting in 5m 0s · 0.9966 ms`, `True™ Tick: Stopping in 5m 0s · 0.4966 ms`, and
`True™ Tick: Paused for 5m 0s · 0.9966 ms`. Countdowns explicitly include seconds. Invalid evidence uses `Timing unknown`. The diagnostic HUD state row shows a plain state word without a parenthetical timing value. After a successful request, the returned verified effective
observation replaces the preflight current value in the controller and app snapshot.
Release observations update that same snapshot. A remaining finer or different value
is displayed as external effective state and does not claim Tick ownership. Query,
request, release, and power reconciliation observations are carried through
controller and app state. Full system reports, raw HNS values, power explanations,
and full errors remain in the diagnostic window. Its local session log records automatic selection, raw native
boundaries, selected HNS, requested HNS, effective HNS, and an `equal`, `finer`,
or `unverified` effective relation. The window shows a local in-memory session log
from process start through the current moment. It uses monotonic sequence numbers
and elapsed process time, has a default 512 event bound, retains newest events with
a truncation marker, and defers disk persistence. The grid shows a one-based retained
row index separately from the monotonic sequence identity, resolving selection confusion
after truncation. The diagnostic header uses the
latest verified effective observation and keeps requested HNS, selected HNS,
effective HNS, raw boundaries, raw status, and relation distinct. The report rows
use typed `DiagnosticEvent` values across eleven columns through the native
`SysListView32` control. A fourteen-variant phase enum replaces arbitrary lifecycle strings,
an eleven-variant source enum identifies exact event ingress origins, and an outcome enum
separates normal policy suppression and no-op states from failures. A six-variant
`EventCategory` enum classifies every event by name into Startup, Timer, Power, UI,
Schedule, or System for filtering and review. Total event count and distinct
operation context count are tracked separately and must not be conflated in the interface.
A status change event is recorded only when derived status or displayed timing actually changes,
preventing event churn and premature buffer eviction.
Every stored event carries a SHA-256 integrity chain. The first event links from a
deterministic genesis seed (`TrueTick-Genesis-v1`), and each subsequent event stores
its predecessor entry hash in `prev_hash` plus its own `entry_hash` computed over
the link, sequence, timestamp, name, and details. `verify_event_chain` replays the
chain over a snapshot and reports the first index whose content hash or link hash
mismatches, so any modification to a recorded event is detected. The store tracks
the running tail hash internally so recording stays O(1).
Fields are sanitized and bounded so raw pointers, credentials, private tokens, arbitrary
secrets, and unbounded sensitive paths are not recorded. Ownership state is
logged separately from effective system state. After release, a remaining finer
value is labeled external without claiming Tick ownership, and a later query
may show the effective value returning to baseline. A successful owned release that leaves a remaining external effective value enters yellow `Stopping, handoff`.
The active-only watcher queries every 250 ms for at most 12 observations. It turns
red with `Stopped` and current effective timing when the value is no longer finer. On timeout it
turns red with `Stopped` and external timing, logging released ownership with another
or unknown finer client. Battery-policy and other owned-request releases use the same
handoff classification.

Startup always queries current timing after the tray surface is ready, even when
Tick is stopped and Auto-time is off. A successful query displays stopped current
timing without claiming ownership. A failed query displays `Stopped · Timing unknown` and records the failure. The selected or requested boundary remains
separate from the current effective observation. A missing configuration file falls
back to defaults with a red status, explaining the red startup state on a fresh extraction.

Power broadcasts refresh timing and recalculate the visible state whether
Auto-time is on or off. With Auto-time off, AC and no owned request show stopped
current timing and wait for manual Start. Returning to AC with auto-time off transitions
to stopped with current timing and waits for a manual start rather than staying blocked.
Battery, Battery Saver, and unknown power release owned timing conservatively and retain the policy reason. AC with
Auto-time on attempts acquisition. Battery-to-AC with Auto-time off shows
stopped current timing rather than remaining blocked.
Configuration writes use a flushed temporary file replacement under a unique
temporary name in the target directory, so two instances cannot collide. Parse and write
errors remain visible. Portable startup registration validates the existing
`Launcher.exe` file before registry writes. The debug fallback validates the
current executable shape and existence. A failed configuration save after a
registry change never deletes a value this process did not create. After an enable
it leaves the written value and reports repair required. After a disable it
restores the registration that the persisted configuration still describes. An
unavailable nonportable target leaves the preference
persistent with a visible warning.
Initial power observation records success or the native failure reason, and failed
observation remains unknown and blocks acquisition. Manual and automatic activation share the same policy.
Battery, Battery Saver, and unknown power states remain non-acquiring. A
restrictive power transition attempts to release owned state. Normal Quit requires
a safe stop when timing is active or uncertain and reports failed cleanup as an
unverified warning rather than exiting.
Quit warnings use only the built-in warning-style `MessageBoxW` with the concise
owned and uncertain messages above. `Yes` maps to Stop and Quit. `No`, close,
unknown or zero results, and MessageBox failure map to Cancel and keep the application
open. A released post-release handoff is not a Tick-owned request and does not keep
the process alive merely because another client retains finer timing. The exact
MessageBox result, external timing record, and fail-closed decision are logged.

The reliability safeguards now include a non-reentrant popup guard, current-state
command validation, exactly-once returned-command dispatch, and deterministic
burst and repeated-command tests. Normal message-loop exit and `GetMessageW`
failure are modeled separately. The cleanup gate attempts owned-request release
once per successful path, keeps a usable loop open when cleanup is unresolved,
and destroys tray, diagnostic, and callback resources before dropping `App`. Separator
handles created during window creation must be stored, otherwise required-child validation
destroys the window. Summary and state text updates check and log native errors,
trapping silently failed control updates. Repeated identical layout failures coalesce
with a bounded count and never report completed.

Process launch is guarded by a named `Local\TrueTickSingleInstance` mutex created
before window, registry, or timer work. A second instance in the same session
records `result=existing_instance` and exits with code 0 after the shared handle
is closed, while a failed guard records `result=guard_failed` with the raw status
and exits with code 1. The acquired handle is held by a `SingleInstanceGuard`
whose `Drop` records the `native.CloseHandle` result. The tray registers the
shell `TaskbarCreated` window message once at startup and re-adds the
notification icon through `Shell_NotifyIconW(NIM_ADD)` when the registered ID
arrives, so the tray icon is restored after an Explorer restart. Both guards have
deterministic unit coverage.

Power query failure is fail closed and verified in code and unit tests. A failed
`GetSystemPowerStatus` clears stale AC or battery state to `Unknown`, clears the
battery-saver flag, records the structured `ObservationFailed` raw status, blocks
acquisition, and follows conservative release policy until a successful query
restores a definitive state.
Configuration files, parser fields, diagnostic fields, startup status, and native
edit text are bounded. Duplicate keys, invalid UTF-8, malformed values, and
oversized inputs are rejected. Native class, window, menu, tray, icon, bitmap,
text, and layout results are checked with raw failure codes recorded.
Running and verified ownership is green. Starting, stopping, pending, degraded,
or unverified behavior is yellow. Stopped, blocked, unsupported, or error behavior
is red. Unsupported is produced when the native capability is unavailable. Error
is reserved for failed operations. Status icon canvases use current DPI where
available. The policy maps 100, 125, 150, 200, and 400 percent to 16, 20, 24, 32,
and 64 pixels, using the nearest supported canvas for intermediate values. The
tray shell may apply its own rendering scale. The calibration crate remains
future-only and is not a disconnected production path. Profiles, application detection, foreground hooks, and profile hysteresis are intentionally absent from internal v1.

Intentionally absent:

- profiles, application detection, foreground hooks, or child process management beyond launcher handoff
- calibration loops, benchmark loops, real-time priority, affinity, QoS, execution-state requests
- NTP, True™ Time integration, network code, installer, secure updater, signing, or release package
- public compatibility, performance, energy, security, or release claims
- runtime registration validation against the development machine
- interactive runtime validation of launcher handoff and slot execution
- interactive runtime validation of the Windows diagnostic window appearance,
  taskbar presence, title-bar controls, restore, and close behavior
- cross-session or multi-user instance coordination beyond the same-session mutex

Duration scheduling is session-only and uses monotonic deadlines. Only one scheduled
action or pause exists. A new selection logs the replacement before changing the
generation and deadline. Cancellation increments the generation and clears the one
bounded coordinator timer, so stale timer events do nothing. Start in queues a future
acquire intent, refreshes current power at the deadline, and rechecks AC, Battery,
Battery Saver, and unknown policy. A blocked start logs suppression and remains safe.
Stop in uses guarded release, records an already released no-op, and uses the existing
yellow handoff watcher when finer external timing remains. Pause for suppresses acquisition and releases through ownership logic. Scheduled Start remains released until its deadline. Scheduled Stop remains owned until its deadline unless policy blocks it. Expiry and cancellation refresh current power and re-evaluate policy through the serialized path. Fixed choices do not
persist, stack, accept custom input, or create an indefinite pause.

The header uses a safe fixed Windows shell URL operation for GitHub. Shell failure is
logged and does not change timer state. Tooltip tracking uses `TTF_ABSOLUTE` and
signed screen coordinates. Failed `GetCursorPos` deactivates the tooltip.

The Windows support matrix, exact native API behavior, and runtime confirmation of the loader fix remain bounded internal validation work. The exact non-running tray development build command is `cargo build -p true-tick --bin true-tick`. It selects the tray binary and is the required build target for this project. Core Windows DLLs such as `kernel32.dll`, `user32.dll`, `ntdll.dll`, `shell32.dll`, `gdi32.dll`, and `comctl32.dll` are OS components and must not be copied or bundled. The embedded Common Controls v6 manifest is the compatibility mechanism. The current MSVC build has a non-system dependency on the Microsoft Visual C++ runtime and Universal CRT. A static binary scan found `VCRUNTIME140.dll` and `api-ms-win-crt-*` imports. The eventual distribution choice is a documented VC++ Redistributable prerequisite or a validated static CRT build. No installer or arbitrary DLL copy is added. The deterministic PE evidence check uses Visual Studio `dumpbin` for `/DEPENDENTS`, `/IMPORTS`, the `.rsrc` section headers, and `.rsrc` raw data. It must show the ordinal 345 import, a non-empty resource directory, and the embedded Common Controls dependency. No runtime Windows success is claimed here. The diagnostic window style contract is source-tested, but its actual appearance, taskbar registration, title-bar controls, restore, and close behavior remain runtime-unverified because the app is not launched. The grid message contract keeps `LVM_INSERTITEMW` at `LVM_FIRST + 77` and corrects `LVM_SETITEMTEXTW` to `LVM_FIRST + 116`. Each refresh logs bounded snapshot row count, inserted row count, item count, insert failures, and set-text failures without cell values or secrets. Session events also retain operation_id, optional parent_operation_id, correlation_id, finite phase, finite source, finite outcome, and typed native outcome fields. Retention is hard-capped at 512 events with a truncation marker. Rendered text and fields are bounded and sanitized. Power observation does not yet provide full Battery Saver,
session, lock, suspend, or resume notification coverage. Unknown observation is
reported as degraded and blocks acquisition. A/B selection is a safe local
scaffold. Missing or invalid active-slot metadata requires repair. It does not
verify a signed package or perform rollback. No installer, signed artifact,
tag, GitHub Release, or public publication is created here. An internal test
package is assembled locally under the untracked `artifacts/` directory for
hand testing, and it is not published.

## Tray tooltip and ownership contract

The tooltip and icon consume the same derived lifecycle state. Verified Running is green. Stopped is red. Scheduled, paused, transitioning, handoff, and unverified states are yellow. Warning, Error, and policy-blocked states retain the existing safety mapping. Timing comes from the authoritative current effective snapshot. Requested and selected values never appear in the compact tooltip. Positive remaining schedule and pause durations round upward from the monotonic deadline, and countdowns always include seconds (`5m 0s`). A plain Paused state has no countdown. The tooltip always uses `True™ Tick: <state> · <timing>`, and parenthetical stopped forms are never rendered. Popup Status rows use the same state and timing source, with more detail in the running duration, next action, and ownership rows. The context-menu status row does not repeat the brand.

Displayed timing uses exact four-decimal millisecond formatting. The shared `Hns` formatter renders `{}.{:04}` over the native unit count, so one HNS is exactly one ten-thousandth of a millisecond and the last decimal digit is the physical 100-nanosecond Windows kernel boundary rather than a rounded approximation. No rounding is applied at any magnitude, and a boundary of 5000 HNS is written `0.5000 ms` instead of a shortened or rounded form. The context menu Status Timing row and the diagnostic HUD Effective field use the detailed form `{}.{:04} ms ({} HNS)`, for example `0.4966 ms (4966 HNS)`. The tray tooltip uses the compact form `{}.{:04} ms`, for example `0.4966 ms`, and keeps raw units out of the shell tooltip. Every surface therefore reports the same unrounded value and only the raw unit suffix distinguishes the compact and detailed views.

## Audited v1 UX and diagnostics update

The current menu hierarchy is `True™ Tick v<version>`, a separator, `Start`, `Stop`, `Schedule >`, a separator, `Auto-start`, `Auto-time`, a separator, `Status >`, `Logs`, a separator, and `Quit`. Start and stop are primary and stay at the top. Scheduling and pausing are secondary and now share one submenu instead of two, so there is a single duration surface. Inside `Schedule >`, contents are, in order, `Start in >`, `Stop in >`, `Pause for >`, `Interval presets...`, a separator, `Cancel scheduled action`, and `Resume now`. The declared label vector in `tray_surface.rs` and the rendered menu both use `Pause for >`. Pause presets are 5 minutes, 15 minutes, 30 minutes, and 1 hour by default and come from the same dynamic preset list as Start in and Stop in. There are no two scheduling surfaces, stacking, or indefinite pause. `Interval presets...` opens the compact native `TrueTickPresetsClass` window titled `True™ Tick Interval Presets` with a sorted ListBox of active presets, an interval input line, and Add, Delete, Reset, and Close buttons. The input accepts natural duration text such as `45m`, `2h`, `10`, `30s`, and `1h30m`. Custom presets update all three duration submenus dynamically and persist across restarts in `schedule_presets_seconds` in `true-tick.toml`. A `ScheduledAction` stores its `DurationPreset` directly, so custom intervals from 10 seconds to 24 hours schedule with their exact seconds value and no clamping to a fixed table. Start is disabled while paused. Status contains only read-only State, Timing, Running for, Next action, and Ownership rows. Logs is a separate clickable action below `Status >`, not an entry inside Status.

An open popup retains its native menu handles and runs a popup-only 500 ms refresh timer for live Status values, ownership, Next action, Start and Stop enabled state, and cancellation state. A separate one-second UI timer runs only while a schedule or pause is active. Its publication key includes the schedule generation and rounded remaining-second bucket, so the shell tooltip receives `Shell_NotifyIconW(NIM_MODIFY)` for each displayed countdown change. These timers never change policy, acquire timing, release timing, or replace the authoritative deadline or handoff timer. A five-minute pause initially displays `5m 0s`. Elapsed durations remain floored.

Logs now shows a concise summary and a read-only native `SysListView32` grid. The list and tooltip classes are initialized once with `InitCommonControlsEx` before controls are created, and the embedded v6 manifest remains required. The columns are `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. `Row` is the current 1-based retained-session row position. `Sequence` remains the event sequence number. Rows come from the existing `DiagnosticStore` snapshot through typed `DiagnosticEvent` conversion. A `CBS_DROPDOWNLIST` category combo box on the toolbar filters the grid by `EventCategory` in real time, offering `All categories`, Startup, Timer, Power, UI, Schedule, and System. `CBN_SELCHANGE` maps the selection to an `EventCategory` or the unfiltered state, records `diagnostic.category_filter.changed`, and refreshes the grid so only matching events display. The Display group accepts a positive `Show rows:` count from 1 through 512 and has a `Show all` override. It shows the newest retained rows in chronological order and reports `Showing X of Y retained rows`, where the visible count reflects the active category filter. It shows the newest retained rows in chronological order and reports `Showing X of Y retained rows`, where the visible count reflects the active category filter. The separate Transfer group accepts empty or `all`, a single retained row, or an inclusive `start-end` retained-row range. Hidden retained rows can be copied or exported intentionally. Invalid input keeps the previous valid display, disables Copy and Export for invalid transfer input, and shows a short bounded validation state. List creation failure, negative `LVM_INSERTITEMW`, failed `LVM_SETITEMTEXTW`, and the optional row or item count after refresh are recorded with bounded details, native return values, and raw Win32 status values. Details preserve raw HNS, operation lineage, pause and schedule generations, timing observations, NTSTATUS, and Win32 error values. Refresh is coalesced through the UI message loop without background polling. The local session retains at most 512 newest events and one truncation marker. It is not disk telemetry. Unknown or stale timing evidence is displayed as `Timing unknown`.

The Logs window has one fixed toolbar row between the summary and grid. It keeps `Show rows:` input, `Show all`, a category filter combo box, `Rows to copy/export:` input, `Selected: N rows`, `Copy`, `Export`, and `Export all` on one horizontal row with compact fixed widths. One shared `DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS` table defines the compact logical widths consumed by both `WM_CREATE` control creation and `WM_SIZE` layout, so every control stays inside the window at any DPI. `WM_SIZE` recalculates positions without wrapping or stacking. `Show rows:` accepts a positive count from 1 through 512. The transfer input accepts empty or `all` for all retained rows, one retained 1-based row, or an inclusive `start-end` range such as `12-24`. Input is always a retained row position, never an event sequence ID. For example, retained rows `353-354` can display event sequences `753-754`. Whitespace is trimmed. Invalid display input preserves the last valid view. Invalid, reversed, zero, negative, and out-of-range transfer values show a short message, create a bounded diagnostic validation event, and keep Copy and Export disabled when no grid selection is available. Retention truncation retains the newest rows and a truncation marker, so earlier row positions may no longer exist. Hidden retained rows remain transferable by intentional range selection. Refresh preserves selected events when possible and visibly resets and logs the transfer selection when it cannot.

The report grid preserves native Windows multi-selection behavior alongside full-row selection, visible selection, and gridlines. A plain click establishes focus and the native selection anchor. Shift+Click selects a contiguous range from the anchor to the clicked row. Ctrl+Click toggles individual row selection. A mouse drag starts rubber-band marquee selection only after the pointer crosses the system drag threshold defined by `SM_CXDRAG` and `SM_CYDRAG`, so plain clicks and small movements never begin a band. A dedicated list subclass draws a focus-rectangle band once the threshold is crossed and selects every displayed row whose bounds intersect the band. Holding Ctrl makes the band additive to the selection that existed when the drag began. Ctrl+A remains available, and a capture change ends an interrupted drag cleanly. Its one-row toolbar summary says `Selected: N rows`. Selected rows are copied or exported in chronological retained order. `Row` remains the current retained position and `Sequence` remains the event identity. Headers and visible content are auto-fitted with native ListView column-width messages only after the initial population or a material data snapshot change. Fixed columns use bounded DPI-scaled minimum and maximum widths, Details absorbs remaining width, and horizontal scrolling remains available when needed. Edit text uses a system GUI font and a minimum control height. System-color erase and paint handling prevents a black widened area and preserves high-contrast behavior where Windows provides it.

Diagnostics now use separate initialization, layout, and data phases. Common Controls and all child HWNDs exist before the diagnostic parent is shown. The compact resizable default opens at a 960x520 client area at 96 DPI, scaled for the current DPI, converted to outer dimensions with DPI-aware frame metrics, and clamped to the primary work area so high-DPI displays no longer open an oversized window. The summary starts with initialization text while hidden, then one full coalescing-compatible refresh takes a bounded snapshot synchronously before the first show. The first visible frame therefore contains the real HUD, status summary, and grid rows rather than `Loading True™ Tick diagnostics...`. Redraw is suspended for the bounded batch and re-enabled by `finish_diagnostic_redraw`, which now explicitly invalidates and updates all 16 child controls in `DIAGNOSTIC_POST_REDRAW_CHILD_TARGETS` before invalidating and updating the parent. The covered children are the bold State label, the HUD summary, both etched separators, the display label, the display input, the Show all button, the category filter combo, the toolbar label, the selection summary, the transfer range input, the Copy, Export, and Export all buttons, the bounded validation message control, and the report list. That exhaustive pass removes the blank-control defect where a control could stay unpainted after `WM_SETREDRAW` until a mouse hover forced a repaint. A deterministic unit test asserts that the target list still covers every diagnostic child. `WM_SIZE` only uses one deferred control reposition pass. It does not read diagnostic data, rebuild rows, or auto-fit columns. Horizontal scrolling and header interaction do not schedule auto-fit. A material snapshot change may schedule one auto-fit for its new generation. Normal refresh restores horizontal scroll when possible. The diagnostic list view implements tail-pinned auto-scrolling: if the viewport is already scrolled to the bottom (the newest event is visible, detected via LVM_GETTOPINDEX and LVM_GETCOUNTPERPAGE), incoming events automatically scroll the newest row into view via LVM_ENSUREVISIBLE. If the user has scrolled up or has a selection of earlier rows, the viewport position is left untouched so their reading position is preserved. Reopening reuses the stable window and posts one refresh.

Ctrl+C is handled by the diagnostic grid when it has focus. Ctrl+A selects all displayed grid rows. Copy and Export first use selected grid rows. If none are selected, they use the separate `Rows to copy/export:` range over the retained snapshot. They never use `Show rows:` as a transfer range. Diagnostics record `source=grid-selection` or `source=range-selection`, selected row count, and result. Refresh preserves valid event identities. Truncation removes only invalid identities and shows and logs a concise selection reset. These actions do not change timer state.

Copy and Export use exactly the selected retained rows from the full bounded snapshot, independent of the displayed slice. The separate `Export all` button bypasses both the grid selection and the transfer range and always exports the entire un-truncated event snapshot. Copy places a sanitized, bounded TSV with the eleven report columns `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details` on the standard Windows clipboard. Export and Export all open the standard Save dialog with `true-tick-log.csv` as the default filename. The dialog filter offers `CSV files (*.csv)` first, then `TSV files (*.tsv)`, then `All files (*.*)`. The default extension is `csv`. Format is selected from the chosen path extension, so `.tsv` produces a sanitized TSV and every other extension produces RFC 4180 CSV. Fields containing commas, double quotes, or line breaks are quoted and internal quotes are doubled. Both formats write UTF-8 through a same-directory temporary replacement and retain the eleven report columns. In parallel, a rolling daily log is appended on a background thread. After each diagnostic refresh and at shutdown, `flush_diagnostic_events_to_disk` writes every newly recorded event as one RFC 4180 CSV row to `Data/logs/true-tick-YYYY-MM-DD.csv` under the portable root (or beside the executable outside portable layout). Each row carries the eleven report columns plus hex-encoded `PrevHash` and `EntryHash` columns, so the on-disk log preserves the same integrity chain as the in-memory store. The app tracks `last_persisted_event_sequence` so only new events are appended. Log directory creation, open, and write failures are reported to stderr and never alter timer state. Display-limit changes, Show all, category filter changes, transfer-range parsing, Copy requests and results, Export requests and results, and schedule menu changes are logged with operation context. Dialog cancellation is logged without error noise. Native clipboard, dialog, and file failures show a short window warning and retain raw native status values in session diagnostics. The toolbar creates explicit HWND controls, follows normal DPI and window resizing, and does not acquire, release, pause, schedule, or otherwise alter timer state. No automatic files, telemetry, credentials, secrets, raw pointers, or uncontrolled logs are produced. The HUD and transfer evidence report actual retained rows and truncation status. Copy and Export preserve the eleven detailed TSV columns, use the actual selected rows from one snapshot, and preserve `Row` as retained position distinct from `Sequence` as event identity.

Each root tray operation records `operation.begin` as `Begin` and `InProgress`, then one terminal `operation.complete` outcome. Logs opening is Failed when native creation or initial layout fails. Nested diagnostic presentation and refresh records keep the active operation context. Sub-events and native calls recorded while a root operation is active are allocated child operation contexts whose `parent_operation_id` is the root operation id, so the `Parent` column now carries real operation lineage. Identical layout failures are coalesced with a bounded count and are never classified Completed.

The diagnostic layout contract is now stable. The summary is a restrained native HUD with a larger bold lifecycle-derived State line and concise key/value fields for Effective timing, Ownership, Power, Startup, Running duration, Next action, retained event history, visible row count, and chain integrity. Native etched separators separate the HUD from the one-row toolbar and the toolbar from the grid. The HUD uses read-only non-scrolling `STATIC` controls with a DPI-scaled 104 logical pixel base height. The larger bold State line occupies 32 of those pixels and the key/value block occupies the remainder. The HUD renders one compact three-line telemetry block with `|` separators instead of an eight-line vertical dump. Line one carries Effective, Ownership, and Power. Line two carries Startup, Running, and Next. Line three carries the History label, the `Showing X of Y retained rows` count, and the chain integrity status. The chain field runs `verify_event_chain` over the current snapshot and shows `Chain: OK` when every entry hash and link verifies, or `Chain: Error at #<index>` naming the first offending row when any event was modified. At 96 DPI the toolbar client minimum is 830 pixels and the minimum client height is 240 pixels, which is the 104 pixel HUD plus a 2 pixel separator plus the 36 pixel toolbar plus a 2 pixel separator plus the 96 pixel grid viewport. The compact default opens at a 960x520 client area at 96 DPI with the outer size clamped to the primary work area. Outer tracking limits are derived from those client minimums with DPI-aware frame metrics, so the controls retain their intended client space.

Deferred positioning logs begin, per-control, and end failures, then falls back to individual positioning for every child. Deferred positioning layout checks IsWindow and positive dimensions on each control before calling DeferWindowPos, eliminating Win32 invalid parameter errors on initial frame layout. Creation failure destroys all partial children and returns `-1`. Successful creation assigns every required child HWND, including both etched separators, before initial layout and showing. Reuse checks the parent and all required children before showing. Parent destruction clears all child handles, selections, refresh flags, and layout state together.

`WM_SIZE` only lays out controls and fits Details to the current viewport. `WM_DPICHANGED` uses the suggested rectangle, reapplies the system GUI font and bold State font, recalculates the layout, and preserves horizontal scroll where possible. `WM_THEMECHANGED`, `WM_SYSCOLORCHANGE`, and `WM_SETTINGCHANGE` refresh diagnostic fonts, system colors, ListView colors, and repainting without changing timer or policy state. High contrast follows system colors. Tray status colors remain separate. Native menus, MessageBox, tooltips, and title-bar behavior remain Windows-owned. The embedded manifest declares Per-Monitor V2 with a `true/pm` fallback for Windows 10 and Windows 11. Runtime DPI compatibility remains unverified until tested. The window does not snapshot data, rebuild rows, or content auto-fit during size changes. Auto-fit is limited to initial population and material snapshot changes. One refresh uses one snapshot and permits one bounded follow-up for refresh-generated records. The session remains capped at 512 events.

The diagnostic `SysListView32` grid applies `LVS_EX_DOUBLEBUFFER` alongside `LVS_EX_GRIDLINES` and `LVS_EX_FULLROWSELECT` through one `LVM_SETEXTENDEDLISTVIEWSTYLE` call during control initialization. The extended-style mask is a compile-time constant with deterministic unit coverage, and double buffering eliminates redraw flicker during coalesced refresh bursts.

Startup diagnostics record the full portable slot layout through `lifecycle.portable_environment`, which carries `is_slot_layout`, the resolved portable root path, the active slot selection result, and the slot executable target. The separate `portable.active_slot.selection` record retains the raw selection outcome including `RepairRequired` reasons and the `not_portable_slot_layout` result for ordinary installations.

Power observation is logged at maximum detail. Every refresh records `power.observation.raw` with the raw `power_state` and `battery_saver` values before any higher-level classification, so the uninterpreted native observation is preserved even when the query fails. This applies at startup, on power broadcast messages, and during schedule deadline rechecks.

Native call sites retain their parameters in bounded diagnostics. Timer request and release records carry `desired_hns` and the `set` boolean, query results carry `minimum_hns`, `maximum_hns`, and `current_hns`, and registry, window, clipboard, and file operations record their stage name and raw status values. Typed `NativeOutcome` fields keep NTSTATUS, Win32 last error, requested, selected, and effective HNS values distinct in every grid row. Every workflow step, including window class registration (`window.class.verify`), tray host window creation (`window.host.verify`), tray icon add (`tray.icon.verify`), diagnostic window creation (`window.diagnostic.verify`), diagnostic window visibility confirmation (`window.diagnostic.visible.verify`), menu command dispatch (`command.dispatch.verify`), and scheduled action execution (`schedule.action.verify`), now emits a paired `*.verify` record in the Verify phase carrying real observed evidence (atom, HWND, IsWindow, IsWindowVisible, command validity, timing match) under the root operation lineage.
