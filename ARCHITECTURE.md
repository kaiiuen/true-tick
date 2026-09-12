# Architecture

This repository contains an internal v1 runtime, not a production release.

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

The tray's native loader failure was confirmed from PE inspection. `true-tick.exe` imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, without an embedded application manifest. Windows therefore selected legacy Common Controls and failed before `main` with `STATUS_ORDINAL_NOT_FOUND`. The tray package now uses `build.rs` to pass MSVC `/MANIFEST:EMBED` and `/MANIFESTINPUT` for its checked-in manifest. That manifest activates Common Controls version 6, declares Windows 10 and later compatibility, and requests `asInvoker` execution without administrator or UI access claims. The Launcher has no corresponding common controls or `TaskDialogIndirect` import and remains unchanged. Static PE checks and runtime Windows validation remain separate tasks.

Manual declarations for the timer functions link explicitly to `ntdll`. Manual kernel32 declarations for file replacement, power observation, last-error retrieval, and module-path lookup explicitly link to `kernel32`. The manifest integration is limited to the tray binary and does not add a GUI framework or a runtime import workaround.

The timer adapter retains the raw `minimum` and `maximum` output labels and values from `NtQueryTimerResolution` for diagnostics. It normalizes the two numeric values into a lower and upper bound, then automatic mode selects the numerically smallest supported boundary from the current query before inclusive validation. The output labels describe API parameters and do not guarantee ascending order. In the captured case, raw `minimum_hns=156250` and `maximum_hns=5000` select `selected_hns=5000`, or `0.500 ms`. This is a current-system result, not a universal 0.5 ms promise. Legacy `request_interval_hns=10000` is migrated to the zero-valued automatic sentinel. The earlier `effective_hns=9966`, or about `0.997 ms`, occurred because the old config requested `10000 HNS`.
Core Windows DLLs such as `kernel32.dll`, `user32.dll`, `ntdll.dll`, `shell32.dll`,
`gdi32.dll`, and `comctl32.dll` are OS components, not bundled compatibility files.
The embedded Common Controls v6 manifest remains the compatibility mechanism. The
current MSVC build uses the non-system Microsoft Visual C++ runtime and Universal
CRT, with `VCRUNTIME140.dll` and `api-ms-win-crt-*` observed in a static binary scan.
The eventual distribution choice is a documented VC++ Redistributable prerequisite
or a validated static CRT build. No installer or copied DLLs are part of this v1.

The internal v1 has no profile configuration, application detection, foreground hooks, or profile hysteresis. The v1 tray menu is intentionally compact. It contains `Start`, `Stop`, current
`Auto-start: On/Off` and `Auto-time: On/Off` toggles, a clickable short status item,
and Quit. Auto-start controls Windows login launch. Auto-time controls automatic timer acquisition after launch. Defaults are `startup_enabled = true` and `automatic = false`. Left-button-up and right-button-up tray notifications both open this same menu. Button-down and double-click notifications are ignored, so Windows notification delivery cannot open duplicate menus. Start and Stop remain the core manual controls and use the same guarded
policy and ownership lifecycle as automatic activation. Start, Stop, and both
setting toggles keep the native context menu open after success or failure. Each
reopen reuses the original popup anchor POINT. The clickable status row
opens a normal overlapped taskbar diagnostic window and never changes timer state.
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
power explanations, and full errors remain excluded from the compact menu and tooltip. While a menu command is highlighted, `WM_MENUSELECT` drives a standard Windows tooltip with concise descriptions. The six descriptions are `Request the best supported timing`, `Release True Tick timing`, `Launch True Tick when you sign in`, `Request timing automatically on AC power`, `Open status and session logs`, and `Stop safely and quit`. The tooltip is destroyed when the popup closes and does not change timer state. The local diagnostic session records automatic selection, raw native boundaries, selected HNS, requested HNS, effective HNS, raw status, and an `equal`, `finer`, or `unverified` effective relation. After a successful request, the controller replaces the preflight current observation with the returned verified effective observation. Release also retains its returned current observation so the UI can show a remaining external effective state without claiming Tick ownership. A released finer effective value enters yellow `Stopping (waiting for handoff)`. The active-only watcher queries every 250 ms for at most 12 observations. Completion shows red `Stopped (current: X ms)`. Timeout shows red `Stopped (external: X ms)` and logs released ownership with another or unknown finer client. Battery-policy and other owned-request releases use the same classification.

Startup always performs a current timing query after the tray surface is ready, even when the runtime is stopped and Auto-time is off. A successful query produces stopped current timing. A failed query produces stopped timing unknown and a diagnostic error. Selected or requested boundaries remain separate from the current effective observation.

Power broadcasts refresh timing and recalculate status regardless of the Auto-time setting. AC with Auto-time off and no ownership is stopped and waits for manual Start. Restrictive power states release owned timing and report the policy reason. AC with Auto-time on acquires when no request is owned. AC with Auto-time off does not acquire and does not remain blocked after a battery-to-AC transition.

Detailed evidence is recorded in a local bounded in-memory session log. Events have
monotonic sequence numbers and elapsed process time. The default limit is 512
events, with newest-event retention and one truncation marker. The store sanitizes
control layout and bounds each field. Disk persistence for a full session log is
deferred. The UI refreshes from a current snapshot and shows the current tray state. The
status tooltip and diagnostic header use the latest verified effective observation
in that shared snapshot, not a stale preflight query. Requested, selected,
effective, raw boundaries, raw status, and relation remain distinct.
Status icon canvases use the current display DPI where available. The policy maps
100, 125, 150, 200, and 400 percent to 16, 20, 24, 32, and 64 pixel canvases,
then clamps intermediate values to the nearest supported size. Status colors remain
green for verified running, yellow for transition or uncertainty, and red for
stopped or failed states. The tray shell may apply its own rendering scale.

An inconclusive adapter postcondition enters an explicit uncertain ownership state
and suppresses repeat acquisition. The adapter retains enough request identity for
a controlled matching release or recovery attempt. Normal message-loop shutdown
uses one centralized cleanup guard. The `TPM_RETURNCMD` return ID is dispatched once by the tray command handler and Quit is not swallowed by the persistent-menu loop. Tray Quit records the active-state decision and shows a native warning when timing is running, starting, stopping, pending, degraded, unverified, or ownership is uncertain. The primary warning is the built-in warning-style `MessageBoxW` because the captured Task Dialog HRESULT was `0x80070057`. `Yes` maps to Stop and Quit. `No`, close, zero, unknown, and MessageBox failure map to Cancel. The exact result is logged, and the retained Task Dialog path is explicitly secondary and is not invoked before MessageBox. `Cancel` leaves timing and the menu command loop unchanged. `Stop and Quit` uses the same guarded release path as Stop and exits only after ownership release and any pending handoff are verified. A failed release leaves the app alive, updates the icon and diagnostic log, and allows retry. Normal cleanup preserves an unverified warning when cleanup cannot be confirmed. The runtime does not use a busy loop,

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

The quit warning uses the built-in warning-style `MessageBoxW` as its primary path
because the captured `TaskDialogIndirect` HRESULT was `0x80070057`. Its text maps
`Yes` to Stop and Quit and `No` to Cancel. Close, unknown or zero results, and
MessageBox failure are fail-closed Cancel decisions. The exact MessageBox result is
logged. The retained Task Dialog implementation is explicitly secondary and is not
invoked before MessageBox. A failed warning path closes the menu without silently
reopening it.
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
The branded tooltip and status menu use concise observed or requested values such as
`True™ Tick: Running (0.500 ms)`, `True™ Tick: Running (0.497 ms, finer)`,
`True™ Tick: Stopped (current: 0.497 ms)`, `True™ Tick: Starting (0.500 ms)`,
`True™ Tick: Stopping (waiting for handoff)`, and
`Error (invalid interval)`. They use `unknown` when no observation is
available. Raw HNS and full event details remain in the diagnostic window.
Controller and app state carry observations through query, request, release,
and power reconciliation. A current value at or below the requested interval
satisfies the postcondition. A lower current value is explicitly finer than
requested. A higher current value is unverified. Current observation verifies
only the available system power query and one power broadcast path. Full Battery
Saver, session, lock, suspend, and resume notification support is not claimed.
