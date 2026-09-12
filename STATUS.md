# Status

**Phase:** internal v1 implementation, no release

## Current lifecycle contract

The timer lifecycle controls the user-visible state. An active release handoff is always yellow `Stopping, handoff` with concise text such as `True™ Tick: Stopping, handoff · 0.497 ms`. Startup registration, configuration persistence, logging, and other diagnostics are separate evidence and cannot publish red lifecycle failure over a live handoff. The icon color is derived from the same lifecycle state as the text. Handoff completion clears the tracker and publishes red `Stopped` with actual current effective timing. A bounded timeout clears the tracker and publishes red `Stopped` with actual current effective timing while ownership remains released.

Desired intent is latest-wins `Acquire` or `Release`. Start, Stop, Auto-time, and power changes use this queue and the same serialized ownership path. A request during `Starting` or `Stopping` is retained until the transition completes or times out. Repeated requests do not issue duplicate native calls. Quit uses ownership, handoff, and uncertain state together, so it cannot exit while cleanup remains unresolved.

One synchronized timing snapshot supplies the tray tooltip, menu status, diagnostic header, handoff classification, and logs. Requested, selected, effective, bounds, raw status, and validity are distinct fields. A failed current query clears effective timing to unknown. Handoff polling observes the stored release boundary without selecting a new interval. Live interval selection remains query-driven. Zero is the automatic sentinel. The named legacy migration value is not a live request default. Resolution examples and fixtures are restricted to tests and migration material. Handoff interval and poll budget are separate handoff policy values.

The workspace builds an internal tray-only v1 with focused deterministic tests. PE inspection confirmed the pre-main loader failure: `target/debug/true-tick.exe` imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, without an embedded application manifest. Windows therefore loaded legacy Common Controls without version 6 activation and failed with `STATUS_ORDINAL_NOT_FOUND`. The tray package now embeds a checked-in manifest through its package-local `build.rs`. Before diagnostic or tooltip controls are created, the tray calls the explicit `InitCommonControlsEx` FFI from `comctl32.dll` once with list-view and the standard bar-class flags that include tooltips. Initialization failure records the raw Win32 error and aborts with a clear diagnostic window failure. The embedded manifest remains required because it activates Common Controls v6 for the `SysListView32` Logs grid. The Launcher does not import `TaskDialogIndirect` or common controls version 6 APIs, so it does not receive this target-specific integration.

The timer adapter explicitly links `ntdll`, passes Windows BOOLEAN as `u8` values `1` or `0`, and preserves signed `i32` NTSTATUS values. It retains the raw `minimum` and `maximum` output labels and values from `NtQueryTimerResolution` for diagnostics, then selects the numerically smallest supported current boundary in automatic mode before inclusive validation. The labels are API field names, not an ordering guarantee. The captured values `minimum_hns=156250`, `maximum_hns=5000`, `current_hns=9966`, and old `requested_hns=10000` explain the observed effective value of about `0.997 ms`. Legacy `request_interval_hns=10000` is migrated to automatic selection. In that captured case, automatic selection would record `selected_hns=5000`, or `0.500 ms`, if the current system reports that boundary. This is not a universal fixed value. Manual kernel32 declarations for file replacement, power observation, last-error retrieval, and module-path lookup explicitly link `kernel32`. Static PE inspection is deterministic and non-running. Runtime Windows resolution of the loader failure remains unverified until the user runs the rebuilt tray binary.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior. A postcondition mismatch preserves
uncertain ownership and suppresses duplicate acquisition until controlled cleanup
recovers or reports the state. Normal message-loop shutdown attempts centralized
cleanup once and keeps an unverified warning if release is not confirmed. Tray Quit now logs the request, active-state decision, dialog result, cleanup result, and exit permission. An owned request uses `True™ Tick is currently controlling timer resolution. Stop timing and quit?`. Uncertain ownership uses `True™ Tick could not verify that timing is fully released. Keep the app open and retry cleanup?`. `Yes` maps to Stop and Quit. `No`, close, zero, unknown, and MessageBox failure map to Cancel. A released post-release handoff is observational only, records that external timing remains, stops its watcher, and allows normal exit. Failed owned cleanup keeps the app alive, updates the icon and diagnostic log, and allows retry. A definitely stopped state with no pending ownership exits without a warning. Opt-in current-user boot startup registration is isolated behind its own Windows
adapter. Portable A/B mode targets the buildable `Launcher.exe` entry point. The
launcher requires valid active-slot metadata, validates the named A or B
executable, and launches that slot before activation. A normal debug run uses a
separate fallback that registers the actual `target/debug/true-tick.exe` tray
executable when no portable launcher path is available. Registration is distinct
from automatic timer activation after the application has launched. Secure
signatures and rollback are not implemented.

The tray starts with a clickable `True™ Tick v<version>` header sourced from Cargo
package metadata. It has compact `Start`, `Stop`, a primary `Pause >` submenu, a
`Schedule >` submenu, `Auto-start: On/Off`, `Auto-time: On/Off`, a read-only
`Status` submenu, and Quit. Pause has 5 minute, 15 minute, 30 minute, and 1 hour
choices. Schedule has Start in, Stop in, Cancel scheduled action, and `Resume now`
when a pause is active. There is no duplicate Pause action, custom duration,
persistence, stacking, or indefinite pause. Auto-start launches the app at Windows
login. Auto-time controls automatic timing acquisition and defaults to off. Both
tray release notifications open the same compact menu. Button-down and double-click
notifications are ignored to avoid duplicate menus.

Status contains disabled State, Timing, Running for, Next action, and Ownership rows.
Each row has a stable read-only menu identity so `WM_MENUSELECT` can show a concise
hover description without enabling the row. The descriptions are `Current True Tick
lifecycle state`, `Latest verified effective timing observation`, `Elapsed time since
verified running`, `Scheduled action and remaining time`, and `True Tick ownership
versus external timing`. Disabled rows cannot dispatch actions or open Logs.
Logs is the only clickable Status row. Status and Logs do not change timer state.
The Timing row uses the current authoritative effective snapshot and shows `Unknown`
when evidence is invalid or stale. Running for uses monotonic time from the last
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
values such as `True™ Tick: Running · 0.497 ms`, `True™ Tick: Stopped · 0.997 ms`,
`True™ Tick: Starting · 0.997 ms`, `True™ Tick: Stopping · 0.497 ms`,
`True™ Tick: Warning`, and `True™ Tick: Error`. Scheduled and paused states use
`Starting in 5m · 0.997 ms`, `Stopping in 5m · 0.497 ms`, and
`Paused for 5m · 0.997 ms`. Invalid evidence uses `Timing unknown`. After a successful request, the returned verified effective
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
a truncation marker, and defers disk persistence. The diagnostic header uses the
latest verified effective observation and keeps requested HNS, selected HNS,
effective HNS, raw boundaries, raw status, and relation distinct. The report rows
use typed `DiagnosticEvent` values across ten columns through the native
`SysListView32` control. Fields are
sanitized and bounded so raw pointers, credentials, private tokens, arbitrary
secrets, and unbounded sensitive paths are not recorded. Ownership state is
logged separately from effective system state. After release, a remaining finer
value is labeled external without claiming Tick ownership, and a later query
may show the effective value returning to baseline. A successful owned release that leaves a remaining external effective value enters yellow `Stopping, handoff`.
The active-only watcher queries every 250 ms for at most 12 observations. It turns
red with `Stopped (current: X ms)` when the value is no longer finer. On timeout it
turns red with `Stopped (external: X ms)` and logs released ownership with another
or unknown finer client. Battery-policy and other owned-request releases use the same
handoff classification.

Startup always queries current timing after the tray surface is ready, even when
Tick is stopped and Auto-time is off. A successful query displays stopped current
timing without claiming ownership. A failed query displays `Stopped · Timing unknown` and records the failure. The selected or requested boundary remains
separate from the current effective observation.

Power broadcasts refresh timing and recalculate the visible state whether
Auto-time is on or off. With Auto-time off, AC and no owned request show stopped
current timing and wait for manual Start. Battery, Battery Saver, and unknown
power release owned timing conservatively and retain the policy reason. AC with
Auto-time on attempts acquisition. Battery-to-AC with Auto-time off shows
stopped current timing rather than remaining blocked.
Configuration writes use a flushed temporary file replacement. Parse and write
errors remain visible. Portable startup registration validates the existing
`Launcher.exe` file before registry writes. The debug fallback validates the
current executable shape and existence. Real registry changes attempt rollback if
config persistence fails. An unavailable nonportable target leaves the preference
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
and destroys tray, diagnostic, and callback resources before dropping `App`.
Power query failure clears stale AC or battery state to `Unknown`, records the
structured error, blocks acquisition, and follows conservative release policy.
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
verify a signed package or perform rollback. No package, installer, signed
artifact, tag, GitHub Release, or public publication is created here.

## Tray tooltip and ownership contract

The tooltip and icon consume the same derived lifecycle state. Verified Running is green. Stopped is red. Scheduled, paused, transitioning, handoff, and unverified states are yellow. Warning, Error, and policy-blocked states retain the existing safety mapping. Timing comes from the authoritative current effective snapshot. Requested and selected values never appear in the compact tooltip. Positive remaining schedule and pause durations round upward from the monotonic deadline. A plain Paused state has no countdown. Popup Status rows use the same state and timing source, with more detail in the running duration, next action, and ownership rows. The context-menu status row does not repeat the brand.

## Audited v1 UX and diagnostics update

The current menu hierarchy is `True™ Tick v<version>`, `Start`, `Stop`, `Pause >`, `Schedule >`, `Auto-start`, `Auto-time`, `Status >`, `Logs`, and `Quit`. Pause is one primary submenu with fixed 5 minute, 15 minute, 30 minute, and 1 hour choices. Schedule has Start in, Stop in, Cancel scheduled action, and `Resume now` when a pause is active. There is no duplicate Pause action. Start is disabled while paused. Status contains only read-only State, Timing, Running for, Next action, and Ownership rows. Logs is a separate clickable action.

An open popup retains its native menu handles and runs a popup-only 500 ms refresh timer for live Status values, ownership, Next action, Start and Stop enabled state, and cancellation state. A separate one-second UI timer runs only while a schedule or pause is active. Its publication key includes the schedule generation and rounded remaining-second bucket, so the shell tooltip receives `Shell_NotifyIconW(NIM_MODIFY)` for each displayed countdown change. These timers never change policy, acquire timing, release timing, or replace the authoritative deadline or handoff timer. A five-minute pause initially displays `5m 0s`. Elapsed durations remain floored.

Logs now shows a concise summary and a read-only native `SysListView32` grid. The list and tooltip classes are initialized once with `InitCommonControlsEx` before controls are created, and the embedded v6 manifest remains required. The columns are `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. `Row` is the current 1-based retained-session row position. `Sequence` remains the event sequence number. Rows come from the existing `DiagnosticStore` snapshot through typed `DiagnosticEvent` conversion. The Display group accepts a positive `Show rows:` count from 1 through 512 and has a `Show all` override. It shows the newest retained rows in chronological order and reports `Showing X of Y retained rows`. The separate Transfer group accepts empty or `all`, a single retained row, or an inclusive `start-end` retained-row range. Hidden retained rows can be copied or exported intentionally. Invalid input keeps the previous valid display, disables Copy and Export for invalid transfer input, and shows a short bounded validation state. List creation failure, negative `LVM_INSERTITEMW`, failed `LVM_SETITEMTEXTW`, and the optional row or item count after refresh are recorded with bounded details, native return values, and raw Win32 status values. Details preserve raw HNS, operation lineage, pause and schedule generations, timing observations, NTSTATUS, and Win32 error values. Refresh is coalesced through the UI message loop without background polling. The local session retains at most 512 newest events and one truncation marker. It is not disk telemetry. Unknown or stale timing evidence is displayed as `Unknown`.

The Logs window has a two-row toolbar with separate Display and Transfer groups. The Display group accepts a positive `Show rows:` count from 1 through 512 and has a `Show all` override. The Transfer group accepts empty or `all` for all retained rows, one retained 1-based row, or an inclusive `start-end` range such as `12-24`. Input is always a retained row position, never an event sequence ID. For example, retained rows `353-354` can display event sequences `753-754`. Whitespace is trimmed. Invalid display input preserves the last valid view. Invalid, reversed, zero, negative, and out-of-range transfer values show a short message, create a bounded diagnostic validation event, and keep Copy and Export disabled. Retention truncation retains the newest rows and a truncation marker, so earlier row positions may no longer exist. Hidden retained rows remain transferable by intentional range selection. Refresh preserves selected events when possible and visibly resets and logs the transfer selection when it cannot.

Copy and Export use exactly the selected retained rows from the full bounded snapshot, independent of the displayed slice. Copy places a sanitized, bounded TSV with the eleven report columns `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details` on the standard Windows clipboard. Export opens the standard Save dialog with `true-tick-log.tsv` as the default filename and writes a UTF-8 TSV through a same-directory temporary replacement. Display-limit changes, Show all, transfer-range parsing, Copy requests and results, Export requests and results, and schedule menu changes are logged with operation context. Dialog cancellation is logged without error noise. Native clipboard, dialog, and file failures show a short window warning and retain raw native status values in session diagnostics. The toolbar creates explicit HWND controls, follows normal DPI and window resizing, and does not acquire, release, pause, schedule, or otherwise alter timer state. No automatic files, telemetry, credentials, secrets, raw pointers, or uncontrolled logs are produced.
