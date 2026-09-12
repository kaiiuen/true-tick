# Architecture

This repository contains an internal v1 runtime, not a production release.

## Runtime invariants

The ownership lifecycle is the source of truth for visible timer status. An active release handoff always derives to yellow `Stopping, handoff` and remains protected from unrelated startup, configuration, logging, or power diagnostic publication. Handoff completion clears the tracker and publishes red `Stopped` with the observed current effective timing. A bounded timeout clears the tracker and publishes red `Stopped` with the observed current effective timing while ownership remains released. The icon uses the same derived lifecycle state as the text.

A latest-wins desired intent queue contains `Acquire` or `Release`. Start, Stop, automatic changes, and power transitions submit intent through the same serialized path. Requests received during `Starting` or `Stopping` are not dropped. Repeated native operations are idempotent, and only the ownership manager calls the native request and release operations.

`tick-ownership` owns one synchronized timing snapshot. It is the source for tray tooltip text, menu status, diagnostic headers, handoff classification, and timer logs. Requested interval, selected interval, effective timing, native bounds, raw status, and current validity remain separate. A failed current query invalidates effective timing. Handoff observation uses the recorded release boundary without resolving a new request.

Timer-resolution selection is always query-driven. Zero is the automatic-selection sentinel. The explicitly named legacy migration value is accepted only while migrating old configuration. Numeric resolution examples and fixture values are restricted to tests or migration fixtures. Handoff polling interval and observation budget are separate policy constants and must not become timer-resolution defaults.

- `tick-core` contains platform-neutral HNS and lifecycle/status/event/error types.
- `tick-policy` contains pure policy decisions and deduplicated logical reasons.
- `tick-ownership` models one serialized runtime instance's tracked contribution and idempotent transitions.
- `tick-platform-windows` isolates the native timer request and release calls. Its manual `ntdll` boundary uses signed `i32` NTSTATUS values and an explicit `u8` Windows BOOLEAN representation, with raw statuses preserved.
- `tick-observation-windows` is the event and power observation boundary.
- `tick-ownership` serializes preflight, request, verification, postcondition, and release while retaining the latest timer observation.
- `tick-diagnostics` owns truthful status formatting and a bounded synchronized in-memory session event store.
- `tick-calibration` remains an explicit unsupported future boundary. It is not a
  production control path.
- `tick-startup-windows` isolates opt-in current-user Run-key registration and removal.
- `apps/true-tick` owns the tray composition, startup configuration, and portable launcher framing.
- `apps/true-tick/src/tray_surface.rs` owns pure compact labels, tooltip text, status-to-color mapping, and status command mapping.

The tray's native loader failure was confirmed from PE inspection. `true-tick.exe` imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, without an embedded application manifest. Windows therefore selected legacy Common Controls and failed before `main` with `STATUS_ORDINAL_NOT_FOUND`. The tray package now uses `build.rs` to pass MSVC `/MANIFEST:EMBED` and `/MANIFESTINPUT` for its checked-in manifest. That manifest activates Common Controls version 6, declares Windows 10 and later compatibility, and requests `asInvoker` execution without administrator or UI access claims. Before any diagnostic or tooltip control is created, the tray calls `InitCommonControlsEx` from `comctl32.dll` once with `ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES`, where the standard bar-class group includes tooltips. Initialization failure records the raw Win32 error and aborts with a clear diagnostic window failure. The embedded v6 manifest is still required for the `SysListView32` Logs grid. The Launcher has no corresponding common controls or `TaskDialogIndirect` import and remains unchanged. Static PE checks and runtime Windows validation remain separate tasks.

Manual declarations for the timer functions link explicitly to `ntdll`. Manual kernel32 declarations for file replacement, power observation, last-error retrieval, and module-path lookup explicitly link to `kernel32`. The explicit Common Controls FFI links to `comctl32.dll`. The manifest integration is limited to the tray binary and does not add a GUI framework or a runtime import workaround.

The timer adapter retains the raw `minimum` and `maximum` output labels and values from `NtQueryTimerResolution` for diagnostics. It normalizes the two numeric values into a lower and upper bound, then automatic mode selects the numerically smallest supported boundary from the current query before inclusive validation. The output labels describe API parameters and do not guarantee ascending order. In the captured case, raw `minimum_hns=156250` and `maximum_hns=5000` select `selected_hns=5000`, or `0.500 ms`. This is a current-system result, not a universal 0.5 ms promise. Legacy `request_interval_hns=10000` is migrated to the zero-valued automatic sentinel. The earlier `effective_hns=9966`, or about `0.997 ms`, occurred because the old config requested `10000 HNS`.
Core Windows DLLs such as `kernel32.dll`, `user32.dll`, `ntdll.dll`, `shell32.dll`,
`gdi32.dll`, and `comctl32.dll` are OS components, not bundled compatibility files.
The embedded Common Controls v6 manifest remains the compatibility mechanism. The
current MSVC build uses the non-system Microsoft Visual C++ runtime and Universal
CRT, with `VCRUNTIME140.dll` and `api-ms-win-crt-*` observed in a static binary scan.
The eventual distribution choice is a documented VC++ Redistributable prerequisite
or a validated static CRT build. No installer or copied DLLs are part of this v1.

The internal v1 has no profile configuration, application detection, foreground hooks, or profile hysteresis. The v1 tray menu is intentionally compact. It starts with a clickable `True™ Tick v<version>` header sourced from Cargo package metadata, then contains `Start`, `Stop`, a primary `Pause >` submenu, a `Schedule >` submenu, current `Auto-start: On/Off` and `Auto-time: On/Off` toggles, a read-only `Status` submenu, and Quit. Pause contains 5 minute, 15 minute, 30 minute, and 1 hour choices. Schedule contains Start in, Stop in, Cancel scheduled action, and `Resume now` when a pause is active. Start and stop schedule choices are 1 minute, 5 minutes, 15 minutes, 30 minutes, and 1 hour. Defaults are `startup_enabled = true` and `automatic = false`. There is no duplicate Pause action, custom duration, persistence, stacking, or indefinite pause. Left-button-up and right-button-up tray notifications both open this same menu. Button-down and double-click notifications are ignored, so Windows notification delivery cannot open duplicate menus. Start and Stop remain the core manual controls and use the same guarded
policy and ownership lifecycle as automatic activation. Start, Stop, and both
setting toggles keep the native context menu open after success or failure. Each
reopen reuses the original popup anchor POINT. The read-only Status rows never dispatch commands. Each disabled informational row has a stable read-only menu identity for `WM_MENUSELECT` hover help. State describes the current True Tick lifecycle state. Timing describes the latest verified effective timing observation. Running for describes elapsed time since verified running. Next action describes the scheduled action and remaining time. Ownership describes True Tick ownership versus external timing. Logs is the only clickable Status row and opens a normal overlapped taskbar diagnostic window without changing timer state.
The two setting toggles use a non recursive return-command loop so the menu stays
open after a toggle. The original popup anchor POINT is captured once and reused
when the menu reopens. Returned command IDs are dispatched once, and toggle
requests record command, save, and resulting-value events. The diagnostic window is
a normal taskbar window titled
`True™ Tick Status and Diagnostics`. It uses `WS_OVERLAPPEDWINDOW` and
`WS_EX_APPWINDOW` without `WS_EX_TOOLWINDOW`, so it has standard minimize,
maximize, restore, close, taskbar, and resize behavior. It shows current status,
power observation, startup result, and the bounded read-only session snapshot.
Reopening validates the existing handle, restores minimized state, activates the
same window, and refreshes its snapshot. The style contract is source-tested, but
actual taskbar appearance and runtime control behavior remain unverified because
validation does not launch the app. Closing it destroys only the window and
does not affect timer ownership. Full system reports, config paths, raw HNS values,
power explanations, and full errors remain excluded from the compact menu and tooltip. The version header is clickable. Status contains disabled State, Timing, Running for, Next action, and Ownership rows, with Logs as the only clickable row. While a menu command is highlighted, `WM_MENUSELECT` drives a standard Windows tooltip with concise descriptions for Duration, Start in, Stop in, Pause for, Cancel scheduled action, Status, Logs, Start, Stop, settings, and Quit. The tooltip is destroyed when the popup closes and does not change timer state. The local diagnostic session records automatic selection, raw native boundaries, selected HNS, requested HNS, effective HNS, raw status, and an `equal`, `finer`, or `unverified` effective relation. After a successful request, the controller replaces the preflight current observation with the returned verified effective observation. Release also retains its returned current observation so the UI can show a remaining external effective state without claiming Tick ownership. A remaining external effective value enters yellow `Stopping, handoff`. The active-only watcher queries every 250 ms for at most 12 observations. Completion shows red `Stopped` with current effective timing. Timeout shows red `Stopped` with current effective timing and logs released ownership with another or unknown client. Battery-policy and other owned-request releases use the same classification.

Startup always performs a current timing query after the tray surface is ready, even when the runtime is stopped and Auto-time is off. A successful query produces stopped current timing. A failed query produces `Stopped · Timing unknown` and a diagnostic error. Selected or requested boundaries remain separate from the current effective observation.

Power broadcasts refresh timing and recalculate status regardless of the Auto-time setting. AC with Auto-time off and no ownership is stopped and waits for manual Start. Restrictive power states release owned timing and report the policy reason. AC with Auto-time on acquires when no request is owned. AC with Auto-time off does not acquire and does not remain blocked after a battery-to-AC transition.

Detailed evidence is recorded in a local bounded in-memory session log. Events have
monotonic sequence numbers and elapsed process time. The default limit is 512
events, with newest-event retention and one truncation marker. The store sanitizes
control layout and bounds each field. Disk persistence for a full session log is
deferred. Session events retain operation_id, optional parent_operation_id,
correlation_id, finite phase, finite source, finite outcome, and typed native
NTSTATUS, Win32 last-error, requested HNS, selected HNS, and effective HNS fields
where available. Retention is hard-capped at 512 events with a truncation marker.
Rendered text and fields are bounded and sanitized. The UI refreshes from a current snapshot and shows the current tray state. The
status tooltip and diagnostic header use the latest verified effective observation
in that shared snapshot, not a stale preflight query. Requested, selected,
effective, raw boundaries, raw status, and relation remain distinct.
Status icon canvases use the current display DPI where available. The policy maps
100, 125, 150, 200, and 400 percent to 16, 20, 24, 32, and 64 pixel canvases,
then clamps intermediate values to the nearest supported size. Status colors remain
green for verified running, yellow for transition or uncertainty, and red for
stopped or failed states. The tray shell may apply its own rendering scale.

Duration scheduling is session-only with fixed choices. Start in and Stop in offer 1 minute, 5 minutes, 15 minutes, 30 minutes, and 1 hour. Pause for offers 5 minutes, 15 minutes, 30 minutes, and 1 hour. There is no custom input, persistence, stacking, or indefinite pause. Only one scheduled action or pause exists. Scheduled Start remains released until its monotonic deadline. Pause releases immediately and suppresses reacquisition until expiry. Scheduled Stop remains owned until its deadline unless policy blocks it. A new selection logs its replacement before changing the monotonic deadline. Cancellation increments the generation and clears the bounded coordinator timer. Start in refreshes current power at its deadline, queues a future acquire intent, and rechecks AC, Battery, Battery Saver, and unknown policy. Stop in uses guarded release and records an already released no-op. Pause for suppresses acquisition, releases through ownership logic, and re-evaluates current policy on expiry or cancellation. Handoff remains the existing yellow bounded watcher. Stale timer generations do nothing. The GitHub header uses a fixed safe shell URL operation. Tooltip tracking uses absolute signed screen coordinates and deactivates after a failed cursor query.

An inconclusive adapter postcondition enters an explicit uncertain ownership state
and suppresses repeat acquisition. The adapter retains enough request identity for
a controlled matching release or recovery attempt. Normal message-loop shutdown
uses one centralized cleanup guard. The `TPM_RETURNCMD` return ID is dispatched once by the tray command handler and Quit is not swallowed by the persistent-menu loop. Tray Quit records the active-state decision and uses only a concise built-in `MessageBoxW` warning. An owned request uses `True™ Tick is currently controlling timer resolution. Stop timing and quit?`. Uncertain ownership uses `True™ Tick could not verify that timing is fully released. Keep the app open and retry cleanup?`. `Yes` maps to Stop and Quit. `No`, close, zero, unknown, and MessageBox failure map to Cancel. A released post-release handoff is observational only and does not block normal Quit. The exact result, external timing, and cleanup decision are logged. A failed release leaves the app alive, updates the icon and diagnostic log, and allows retry. Normal cleanup preserves an unverified warning when cleanup cannot be confirmed. The runtime does not use a busy loop,

high priority, affinity, QoS, execution-state requests, power-plan changes,
registry tuning beyond the explicit current-user startup boundary, driver,
hardware clock control, process detection, network services, installer, secure
updater, or release machinery.

The native tray boundary has a non-reentrant menu-active guard. Reentrant tray,
command, and power messages are ignored while `TrackPopupMenu` owns the popup.
The returned command ID is the only popup dispatch source and is validated against
current enabled state before handling. Quit and cleanup remain serialized through
the same command path.

Shutdown models normal `WM_QUIT` separately from `GetMessageW` failure. A guarded
cleanup gate attempts owned-request release once per successful shutdown path,
allows retry after an unresolved release while the message loop is usable, and
records a built-in warning before an irrecoverable exit. Tray icon deletion,
diagnostic window destruction, callback detachment, and main window destruction
occur before `App` is dropped.

Power query errors clear the current snapshot to `Unknown`, retain the structured
failure, block acquisition, and use the conservative release policy. They do not
claim suspend, session, lock, or resume coverage.

Configuration and diagnostic text have byte limits. Parsing rejects oversized
files, lines, keys, values, duplicate keys, invalid UTF-8, malformed assignments,
and invalid scalar values. Diagnostic truncation preserves UTF-8 boundaries.
Native class, window, menu, tray, icon, bitmap, diagnostic edit, text, and layout
results are checked. Partial native resources are released in reverse order, and
startup stops before timing acquisition if the tray surface is unavailable.

The quit warning uses only the built-in warning-style `MessageBoxW`. Its owned and
uncertain text is concise and its `Yes` result maps to Stop and Quit. `No`, close,
unknown or zero results, and MessageBox failure are fail-closed Cancel decisions. A
released handoff is already outside Tick ownership, so shutdown stops its watcher,
records that external timing remains, and exits without waiting for another client.
A failed owned release keeps the app open and allows retry.
Boot startup registration is a per-user Run-key operation controlled by
`startup_enabled` in the local config. Portable A/B mode registers the buildable
`Launcher.exe` entry point, not `true-tick.exe` from either slot. Normal debug
runs have a separate explicit fallback that registers the actual
`target/debug/true-tick.exe` tray executable when no portable launcher path is
available. The launcher owns slot selection and validation before activation, and launches only
the active slot with forwarded arguments. Missing or invalid metadata fails with
repair required. Secure signatures and rollback are not implemented. The bounded
path resolver derives the launcher path from a `Slots\A` or `Slots\B` executable
shape without claiming runtime filesystem validation. Registration is non-elevated,
idempotent, removable, and not performed by tests. It is not machine-wide
installation and it is not the same as `automatic`, which controls timer
activation after launch. Both settings are persisted through a flushed temporary
file replacement. Parse and write failures are surfaced in the tray status.
Missing or invalid A/B metadata requires repair and never defaults to slot A.
Portable startup registration validates launcher existence and executable identity before
writing. The debug fallback validates the current executable shape and existence.
If config persistence fails after a registry change, the inverse operation is
attempted and failure is marked repair required. If no safe target is available,
the preference remains persistent with a visible registration-unavailable status.
Initial power query
errors are recorded with their native status and remain conservative unknown
observation. True™ Time is not a dependency. Platform behavior that cannot be verified remains
yellow or red rather than being reported as green. The tray maps running and verified ownership to green, starting, stopping,
pending, degraded, or unverified behavior to yellow, and stopped, blocked,
unsupported, or error behavior to red. Each public tray status is reachable from
an explicit runtime path or is covered by a deterministic boundary test.
The branded tooltip uses concise values such as
`True™ Tick: Running · 0.497 ms`, `True™ Tick: Stopped · 0.997 ms`,
`True™ Tick: Starting · 0.997 ms`, `True™ Tick: Stopping · 0.497 ms`,
`True™ Tick: Warning`, and `True™ Tick: Error`. Scheduled and paused states use
`Starting in 5m · 0.997 ms`, `Stopping in 5m · 0.497 ms`, and
`Paused for 5m · 0.997 ms`. It uses `Timing unknown` when evidence is invalid.
The read-only Status submenu uses the same state and timing wording, then adds
`Running for`, `Next action`, and `Ownership` rows. The context-menu status row
does not repeat the brand. Raw HNS and full event details remain in the diagnostic window.
Controller and app state carry observations through query, request, release,
and power reconciliation. A current value at or below the requested interval
satisfies the postcondition. A lower current value is explicitly finer than
requested. A higher current value is unverified. Current observation verifies
only the available system power query and one power broadcast path. Full Battery
Saver, session, lock, suspend, and resume notification support is not claimed.

## Tray tooltip and ownership contract

Tray rendering consumes one derived lifecycle state and one authoritative timing snapshot. Verified Running is green. Stopped is red. Scheduled, paused, transitioning, handoff, and unverified states are yellow. Warning, Error, and policy-blocked states retain their existing safety mapping. Tooltip timing is the current effective observation, never the requested or selected interval. Positive remaining schedule and pause durations round upward from the monotonic deadline. A plain Paused state has no countdown.

## Audited v1 tray and diagnostic surface

The current native menu is ordered as `True™ Tick v<version>`, `Start`, `Stop`, `Pause >`, `Schedule >`, `Auto-start`, `Auto-time`, `Status >`, `Logs`, and `Quit`. Pause is one primary submenu with fixed 5 minute, 15 minute, 30 minute, and 1 hour choices. Schedule has Start in, Stop in, Cancel scheduled action, and `Resume now` when a pause is active. There is no duplicate Pause action, custom duration, persistence, stacking, or indefinite pause. Start is disabled while paused and the read-only Status rows explain the paused state. Logs is not duplicated inside Status.

The active popup retains its root, Pause, Schedule, and Status handles. A popup-only 500 ms UI timer refreshes Status rows, ownership, Next action, Start and Stop enabled states, and cancellation state. A separate one-second UI timer runs only while a schedule or pause is active. Its publication key contains the action generation and rounded remaining-second bucket, so the shell tooltip receives `Shell_NotifyIconW(NIM_MODIFY)` for displayed countdown changes. Both timers are bounded and do not touch the authoritative deadline or handoff timer. Positive fractional remaining seconds are rounded upward, while elapsed durations remain floored.

The diagnostic window has an explicit summary control and a native `SysListView32` report control. Its columns are `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. `Row` is the current 1-based retained-session row position. `Sequence` is the event sequence number and is never used as a range selector. Each existing `DiagnosticStore` snapshot is converted from typed `DiagnosticEvent` values. The native message contract keeps `LVM_INSERTITEMW` at `LVM_FIRST + 77` and uses the corrected `LVM_SETITEMTEXTW` at `LVM_FIRST + 116`. The synchronous control messages copy bounded UTF-16 text. Every refresh records only the snapshot row count, inserted row count, item count, insert failures, and set-text failures. These values are bounded counts and do not include secrets or long cell values. List creation failure and native message failures retain bounded raw status details. The bounded Details cell preserves raw HNS, typed NTSTATUS and Win32 last-error values, timing observations, schedule generations, and operation lineage. Refreshes are posted and coalesced on the diagnostic window UI message loop. The report is read-only, session-local, newest-retained, capped at 512 events, and marked when truncation occurs. A failed timing observation is `Unknown` rather than stale current data. Closing Logs does not change timer state.

The exact non-running tray development build command is `cargo build -p true-tick --bin true-tick`. It selects the tray binary without launching the app.

The diagnostic window owns two explicit toolbar groups. The Display group says `Show rows:`, accepts a positive number from 1 through the hard retained maximum of 512, and provides a `Show all` override. The default is 100. Display uses the newest retained rows in chronological order and reports `Showing X of Y retained rows`. Empty or invalid display input keeps the last valid view and shows a short validation state.

The Transfer group says `Rows to copy/export:` and has its own input, selection summary, `Copy`, and `Export` controls. Empty input or `all` selects all retained rows. A single number selects one 1-based retained row. `start-end` selects an inclusive retained-row range. Whitespace is trimmed. Row numbers refer to the current retained snapshot, never event sequence IDs. Hidden retained rows can be transferred intentionally because Copy and Export parse and map the full bounded snapshot independently of Show rows. Invalid, reversed, zero, negative, and out-of-range values leave both actions disabled and record bounded validation diagnostics. A refreshed snapshot preserves selected event sequences when they remain retained, updates their Row values, and visibly resets and logs the transfer selection when retention removed them.

The report grid uses normal multi-row selection. Single-select mode is not requested. Full-row selection, visible selection, gridlines, and normal mouse drag selection are enabled. `Selected: N rows` is shown near the toolbar. Native `LVN_ITEMCHANGED` notifications update selected event sequences on the diagnostic window UI thread. Native `LVN_KEYDOWN` handles Ctrl+C and Ctrl+A only while the grid has focus. Ctrl+A selects all rows currently displayed by the grid.

Copy, Export, and grid-focused Ctrl+C use the same precedence. One or more selected grid rows are the `grid-selection` source. With no grid rows selected, the retained range field is the `range-selection` source. The range is applied to the full retained snapshot and never to the display-limit slice. Empty or invalid retained input produces a concise no-selection or validation message. Logs include the source, selected row count, and result. Grid selections are emitted in retained chronological order and keep the eleven report columns with `Row` before `Sequence`.

A diagnostic refresh preserves selected event sequences by identity where possible. If retention removes some selected events, valid selections remain and only invalid identities are removed. The UI shows and logs a concise selection reset. The retained event and field bounds cap all generated TSV output. Selection, refresh, Copy, and Export do not change timer state.

TSV conversion starts from the same eleven typed grid columns used by `SysListView32`, with `Row` before `Sequence`. Cells are sanitized again, control layout is removed, each field remains bounded, and output rows are capped by the hard diagnostic retention bound. Copy uses `OpenClipboard`, movable global memory, `SetClipboardData` with `CF_UNICODETEXT`, and guaranteed cleanup on failure. Successful clipboard ownership is transferred to Windows only after the data is ready. Export uses `GetSaveFileNameW`, treats a zero extended dialog error as user cancellation, writes UTF-8 bytes to a same-directory temporary path, and uses `MoveFileExW` with replacement and write-through flags. The selected path is not recorded in diagnostics. Display-limit changes, Show all, transfer-range parsing, Copy requests and results, Export requests and results, and menu schedule changes are recorded with operation context. Native clipboard, dialog, and file failures retain bounded raw Win32 values.
