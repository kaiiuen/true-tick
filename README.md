# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

## Lifecycle and timing invariants

The timer lifecycle is authoritative for the tray status. While a release handoff is active, the derived status is always yellow `Stopping, handoff` and unrelated configuration, startup, logging, or power diagnostics cannot replace it with red `Error`, `Stopped`, or `Blocked`. The icon color is derived from that same lifecycle state. A handoff completion clears the tracker and publishes red `Stopped` with the current effective timing. A bounded timeout clears the tracker and publishes red `Stopped` with the current effective timing while ownership remains released.

Manual and automatic changes use a latest-wins desired intent queue with only `Acquire` and `Release`. An intent received during `Starting` or `Stopping` is retained and processed after the transition. Native request and release calls remain serialized and idempotent. Power policy is applied when the queued intent is processed. Auto-time off on AC therefore remains stopped when no release is pending.

All tray text, menu status, diagnostic headers, handoff classification, and timer logs read from one synchronized timing snapshot. The snapshot keeps requested, selected, effective, raw bounds, and raw status distinct. A failed current query invalidates effective timing and displays `Timing unknown` rather than retaining stale current data. Handoff polling observes the stored release boundary and never selects a new request interval.

Live timer selection is query-driven. The persisted zero value is the automatic-selection sentinel. The named legacy migration value is accepted only while migrating old configuration. Timer-resolution examples and fixture values belong only in tests or migration fixtures. Handoff polling intervals and observation budgets are separate handoff policy constants and are not timer-resolution defaults.

## Development commands

The workspace has two app packages, so commands from the workspace root select
the package and target explicitly. These checks do not launch an app or change
Windows state:

```text
cargo metadata --no-deps --format-version 1
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p true-tick --bin true-tick
py -3 ..\tools\true-tick-dev\check_live_timer_values.py
```

The timer and documentation validation scripts are private workspace development
tools stored outside this repository at `..\tools\true-tick-dev\`. They are not
part of the GitHub project payload. Run the tools from the repository root.

To launch the tray app during an explicitly authorized development run, use:

```text
cargo run -p true-tick --bin true-tick
```

The portable Launcher target is separate:

```text
cargo run -p true-tick-launcher --bin Launcher
```

Within `apps/true-tick`, `cargo run` defaults to the `true-tick` tray target.
The workspace-root command remains explicit because both app targets are
intended development targets. The exact non-running tray development build command is
`cargo build -p true-tick --bin true-tick`. It selects the tray binary and does
not launch the app.

The diagnostic grid keeps `LVM_INSERTITEMW` at `LVM_FIRST + 77` and uses the
corrected `LVM_SETITEMTEXTW` value `LVM_FIRST + 116`. Each refresh converts the
bounded typed snapshot and records only snapshot row count, inserted row count,
item count, insert failures, and set-text failures. Common Controls initialization
remains explicit before the list and tooltip controls are created.

The commands above are not runtime validation of
tray behavior, startup registration, or launcher handoff.

The v1 uses a narrow native timer adapter with raw status preservation, an
explicit released, owned, or uncertain ownership lifecycle, controlled recovery,
conservative power policy, and a native tray
surface, atomically replaced local configuration, per-user boot startup
registration, and a bounded portable A/B launcher scaffold. The persisted
`request_interval_hns = 0` value is an automatic-selection sentinel. Before each
acquisition, the adapter queries the current native boundaries and selects the
numerically smallest supported boundary. It does not use a universal fixed
`1 ms` or `0.5 ms` value. In the captured case, raw values `minimum_hns=156250`
and `maximum_hns=5000` select `5000 HNS`, or `0.500 ms`, when that boundary is
reported by the current system. The adapter retains raw boundary values and
selected HNS in diagnostics, then validates the selected value before requesting
it. The compact tray menu starts with a clickable `True™ Tick v<version>` header sourced from Cargo package metadata. It then exposes `Start`, `Stop`, a primary `Pause >` submenu, a `Schedule >` submenu, `Auto-start: On/Off`, `Auto-time: On/Off`, a read-only `Status` submenu, and `Quit`. Pause offers 5 minutes, 15 minutes, 30 minutes, and 1 hour. Schedule offers Start in, Stop in, Cancel scheduled action, and `Resume now` when a pause is active. There is no duplicate Pause action, custom duration, persistence, stacking, or indefinite pause. Auto-start controls launch at Windows login. Auto-time controls automatic timer acquisition after launch. The current defaults are `startup_enabled = true` and `automatic = false`, so login launch does not acquire timing until the user manually starts it.

`Status` contains disabled State, Timing, Running for, Next action, and Ownership rows. Each informational row has a stable read-only menu identity so `WM_MENUSELECT` can show its hover description while the row remains disabled. The descriptions are `Current True Tick lifecycle state`, `Latest verified effective timing observation`, `Elapsed time since verified running`, `Scheduled action and remaining time`, and `True Tick ownership versus external timing`. These rows cannot dispatch commands or open Logs. `Logs` is the only clickable row in that submenu. Status and Logs do not change timer state. The Status Timing row uses the latest valid authoritative effective observation and displays `Timing unknown` when evidence is invalid or stale. Running for uses monotonic time from the last verified Running transition. The diagnostic window remains titled `True™ Tick Status and Diagnostics`, with a read-only session view and snapshot refresh on reopen. It uses a normal taskbar window with standard title-bar controls and does not change timer state when closed.

The native menu keeps Start, Stop, and both setting toggles open after successful or failed handling. Each reopen uses the original popup anchor POINT. Highlighting a command uses a standard Windows tooltip. Descriptions cover Duration, Start in, Stop in, Pause for, Cancel scheduled action, Status, Logs, Start, Stop, startup, automatic timing, and Quit. The tooltip is destroyed when the popup closes and never changes timer state. Logs closes the menu when it opens diagnostics. Quit is dispatched from the returned `TPM_RETURNCMD` command. Tray left-button-up and right-button-up notifications open this same menu. Button-down and double-click notifications are ignored. Start and Stop remain manual controls and use the same guarded policy and ownership lifecycle as automatic activation. Start remains yellow through query, request, and verification, then turns green only after a verified request. Quit logs its request, active-state decision, dialog result, cleanup result, and exit permission. If timing is running, starting, stopping, pending, degraded, unverified, or ownership is uncertain, the warning offers exactly `Cancel` and `Stop and Quit`. A failed or uncertain release keeps the app open and allows retry. The diagnostic window shows a bounded local in-memory session log.

The quit warning uses only the built-in warning-style `MessageBoxW`. When Tick owns
an active request it says `True™ Tick is currently controlling timer resolution. Stop
timing and quit?`. When ownership is uncertain it says `True™ Tick could not verify
that timing is fully released. Keep the app open and retry cleanup?`. `Yes` maps to
Stop and Quit. `No`, close, zero, unknown, and MessageBox failure map to Cancel. The
exact result and fail-closed decision are logged. A released post-release handoff is
observational only, so normal Quit records remaining external timing and does not
wait for another application.
It excludes raw pointers, private tokens, credentials, arbitrary secrets, and
unbounded sensitive paths. The branded tooltip uses short runtime values such as
`True™ Tick: Running · 0.497 ms`, `True™ Tick: Stopped · 0.997 ms`,
`True™ Tick: Starting · 0.997 ms`, `True™ Tick: Stopping · 0.497 ms`,
`True™ Tick: Warning`, and `True™ Tick: Error`. The read-only Status submenu uses concise rows such as
`State: Running`, `Timing: 0.497 ms`, `Running for: 4m 12s`, and
`Ownership: True™ Tick`. Values use the selected request and the latest verified effective
observation as distinct fields, and another platform boundary is allowed. After a
	successful request, the returned effective observation replaces the preflight
	current value in controller and app state. After release, the returned current
	observation is retained as effective external state when another client remains
	finer or otherwise active, without implying Tick ownership. After True™ Tick releases its request, a remaining external effective value enters yellow `Stopping, handoff` until the bounded observation completes.
The app queries the effective value only while that handoff is pending, using a
250 ms Windows timer for at most 12 observations. A non-finer observation completes
the handoff and shows red `Stopped (current: X ms)`. If the bound expires while a
finer value remains, the app shows red `Stopped (external: X ms)` and logs that True
Tick ownership is released while another or unknown client keeps finer timing. This
classification also applies to battery-policy and other owned-request releases.
The captured `current_hns=9966` value was about `0.997 ms` because the old config requested
`10000 HNS`. That config is migrated to automatic selection. At startup, the controller always queries the current effective timing, even when True™ Tick is stopped and Auto-time is off. A successful query is displayed as stopped current timing without claiming Tick ownership. A failed query is displayed as `Stopped · Timing unknown` and recorded as a diagnostic failure. The selected or requested boundary remains separate from the current effective observation.

Power broadcasts refresh timing and recalculate the visible state for both Auto-time settings. With Auto-time off, AC and no owned request show stopped current timing and wait for manual Start. Battery, Battery Saver, and unknown power remain conservative and release owned timing when policy requires it. Returning to AC with Auto-time on attempts acquisition. Returning to AC with Auto-time off shows stopped current timing rather than leaving a stale blocked state.

Duration scheduling is session-only and uses monotonic deadlines. Only one scheduled action or pause exists. A new selection logs the replacement before it replaces the old generation. Cancellation invalidates the generation and kills the bounded coordinator timer, so stale timer events do nothing. Start in queues a future acquire intent, refreshes current power at the deadline, and applies AC, battery, Battery Saver, and unknown policy again. A blocked start is logged as suppressed and does not acquire. Stop in uses the guarded release path, records a no-op when already released, and uses the existing yellow handoff watcher when release leaves finer external timing. Pause for immediately suppresses acquisition and releases through ownership logic. A scheduled Start remains released until its deadline. A scheduled Stop keeps owned timing until its deadline unless policy blocks it. Expiry or cancellation refreshes policy and re-evaluates it through the same serialized path. The coordinator uses at most one bounded Windows timer and does not poll permanently.

The display uses `Unknown` when no valid observation is available. Raw HNS and full event details remain in the diagnostic window. The read-only Status submenu records current state, effective timing, monotonic running duration, next action, and ownership. The local diagnostic session records automatic selection,
raw native boundaries, selected HNS, requested HNS, effective HNS, raw status,
and an `equal`, `finer`, or `unverified` effective relation. The tooltip, status
summary, and diagnostic header use the latest verified effective observation from
request, release, query, or power reconciliation. Query, request, release, and
power reconciliation observations are carried through controller and app state.
The tray popup has a non-reentrant active guard. Tray notifications and window
commands received while the popup is active are ignored, while the one returned
`TPM_RETURNCMD` value is dispatched once. Command IDs are checked against the
current enabled state before any action. Repeated Start, Stop, and setting
commands therefore remain idempotent, and Quit remains a single exit path.
A reported current
value at or below the requested value is satisfied, with a lower value labeled
finer. A higher value remains unverified. Unsupported native timer capability
is surfaced as `Unsupported`, while error, blocked, degraded, unverified,
starting, stopping, running, and stopped retain consistent icon colors and menu
meanings. Status icon canvases use the current
window DPI where available: 16, 20, 24, 32, or 64 pixels for 100, 125, 150, 200,
or 400 percent. Unsupported intermediate DPI values use the nearest supported canvas.
The tray shell may apply its own additional rendering scale. Calibration remains an
explicit future-only boundary. The internal v1 has no profile configuration, application detection, foreground hooks, or profile hysteresis. All activation paths use
the same conservative power policy, so battery, Battery Saver, and unknown power
states do not acquire. If a native request may have succeeded but its postcondition
is inconclusive, the runtime records uncertain ownership, blocks duplicate acquire,
and attempts only a controlled matching release. Normal message-loop shutdown
attempts this cleanup once and preserves an unverified warning when it fails. Events use monotonic sequence numbers and elapsed time
from process start. The default bound is 512 events. Newest events are retained
with a truncation marker when the bound is reached. Disk persistence for full
session logs is deferred. Session events retain operation_id, optional
parent_operation_id, correlation_id, finite phase, finite source, finite outcome,
and typed native NTSTATUS, Win32 last-error, requested HNS, selected HNS, and
effective HNS fields where available. Rendered text is bounded and sanitized.
The header uses a fixed safe Windows shell URL operation for GitHub. Tooltip
tracking uses absolute signed screen coordinates and deactivates on failed
`GetCursorPos`. Runtime shell, taskbar, and Windows notification behavior remains
unverified because validation does not launch the app.

Portable A/B boot registration targets the buildable portable `Launcher.exe`
entry point, never a slot payload. The launcher requires `active-slot.txt`, selects only
the named A or B slot, validates the expected `true-tick.exe` file, and launches
that slot with forwarded arguments. Missing or invalid metadata reports repair
required and never silently selects A. Secure signatures and rollback are not
implemented and remain deferred. Boot registration is opt-in through local
config and is distinct from automatic timer activation after launch. The path resolver only derives `Launcher.exe` from a `Slots\A` or `Slots\B`
executable shape. Normal debug runs use a separate explicit fallback: a debug
build whose actual executable is `target/debug/true-tick.exe` registers that
current tray executable for the current user when no portable launcher path is
available. Portable A/B paths always retain launcher semantics. A nonportable
non-debug path keeps the preference with a visible registration-unavailable
status rather than writing an unvalidated target. The launcher performs the
separate runtime metadata and file checks before handoff. It does not include
profiles, process detection, True™ Time, NTP, an installer, a secure updater, or
release signing.

Unsupported platform behavior remains explicit. Native API acceptance and the
adapter postcondition are not claims about a universal effective system value.
The controller retains the latest raw-status timer observation from query,
request, or release, and the app refreshes it during power reconciliation.
Power observation currently verifies only the available system power query and
one power broadcast path. Battery Saver, session, lock, suspend, and resume
notification coverage remains incomplete and is shown as unknown or degraded
rather than claimed as fully observed.

Portable startup registration validates that the target exists and is exactly
`Launcher.exe` before writing the current-user value. The debug fallback validates
the current executable shape and existence before writing. Registration is
non-elevated, idempotent, removable, and not performed by tests. Config
persistence and a real registry operation are kept transactionally consistent
with a rollback attempt or an explicit repair-needed status. Initial power
observation records success or the failure reason in the diagnostic log. A failed
A failed observation remains unknown and blocks acquisition. A later power
query failure explicitly clears any previous AC or battery state, records the
failure, blocks new acquisition, and follows the conservative release path for
owned timing. Session, lock, suspend, and resume behavior is not inferred from
this query.

Configuration input is bounded before parsing. The internal v1 rejects oversized
files, lines, keys, values, duplicate keys, invalid UTF-8, malformed assignments,
and invalid booleans or integers. Diagnostic fields and native diagnostic text
are bounded at valid UTF-8 boundaries. Startup status is bounded before it is
shown. Native class, window, menu, tray icon, bitmap, diagnostic control, text,
and layout failures are checked and record raw native status codes. Startup fails
closed before automatic acquisition when the tray surface cannot be created.

Normal `WM_QUIT` and `GetMessageW` failure are modeled separately. Every normal
exit attempts the guarded owned-request cleanup. Unresolved cleanup keeps a usable
message loop alive for retry or produces a built-in warning before an irrecoverable
exit. Tray and diagnostic resources are destroyed before `App` is dropped.

Active documentation is checked with the private workspace tool
`..\tools\true-tick-dev\check_doc_punctuation.py`. Run it from the repository
root against the active documentation files. The checker rejects em dash and
semicolon characters and skips historical archive material. The tool is outside
the GitHub project payload.

## Tray tooltip and ownership contract

The tray tooltip is branded and concise. It uses the current effective timing from the authoritative snapshot, never the requested interval. Valid timing is shown with a middle dot, for example `True™ Tick: Running · 0.497 ms`. Invalid timing is shown as `Timing unknown`. The states are `Stopped`, `Running`, `Starting`, `Stopping`, `Stopping, handoff`, `Starting in 5m`, and `Paused for 5m`. A plain paused state has no countdown. Positive schedule and pause time rounds upward from the monotonic deadline.

The icon follows the same derived lifecycle state as the tooltip. Running with verified ownership is green. Stopped is red. Scheduled, paused, transitioning, handoff, and unverified states are yellow. Warning, error, and policy-blocked states retain the existing red safety mapping. Scheduled Start and Pause release True™ Tick ownership immediately. Scheduled Stop retains ownership until its deadline unless AC, battery, Battery Saver, or unknown policy requires an earlier release. The popup Status rows use the same snapshot and lifecycle wording, with additional running duration, next action, and ownership detail.

## Windows DLL boundary

Core Windows DLLs such as `kernel32.dll`, `user32.dll`, `ntdll.dll`, `shell32.dll`,
`gdi32.dll`, and `comctl32.dll` are operating system components. They are linked as
system dependencies and must not be copied into this repository or bundled as a
compatibility workaround. The embedded Common Controls v6 manifest is the correct
compatibility mechanism for the Common Controls API used by the tray.

The current MSVC build has a non-system runtime dependency on the Microsoft Visual C++
runtime and Universal CRT. A static scan of the rebuilt PE found `VCRUNTIME140.dll`
and `api-ms-win-crt-*` imports. Exact release imports still require a PE import tool in
a Visual Studio developer environment. The eventual distribution decision is deferred
to a documented Microsoft VC++ Redistributable prerequisite or a validated static CRT
build. No installer is added here, and no arbitrary runtime DLLs are copied into the
project.

## Native loader diagnosis

The confirmed pre-main failure was in the tray PE. `target/debug/true-tick.exe`
imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, but
had no embedded application manifest. Windows therefore loaded legacy
Common Controls without version 6 activation and failed with
`STATUS_ORDINAL_NOT_FOUND` before `main`.

The tray target now embeds `apps/true-tick/windows/true-tick.manifest`. The
package-local `build.rs` passes `/MANIFEST:EMBED` and `/MANIFESTINPUT` only for
MSVC Windows builds. The manifest requests Common Controls version 6, declares
Windows 10 and later compatibility through the Windows 10 supported-OS
identifier, and requests normal user execution with `asInvoker`. It makes no
administrator, UI access, legacy Windows, performance, or runtime success
claim. Before any diagnostic or tooltip control is created, the tray calls
`InitCommonControlsEx` from `comctl32.dll` once with `ICC_LISTVIEW_CLASSES` and
`ICC_BAR_CLASSES`, the standard bar-class group that includes tooltips. A failure
records the raw Win32 error and aborts startup with
a clear diagnostic window failure. The embedded v6 manifest remains required
for this runtime initialization and the `SysListView32` Logs grid. The Launcher
does not import `TaskDialogIndirect` or use common controls version 6 APIs, so it
does not need this target-specific manifest.

The Windows timer adapter links `NtQueryTimerResolution` and
`NtSetTimerResolution` explicitly from `ntdll`. Its `NtSetTimerResolution`
BOOLEAN argument crosses the Rust boundary as an explicit `u8` value of `1` or
`0`. Native NTSTATUS values cross the boundary as signed `i32` values and are
preserved in timer observations and errors. Manually declared kernel32 APIs
for configuration replacement, power observation, last-error retrieval, and
module-path lookup also have explicit `kernel32` links.

Use this deterministic non-running PE check after building the exact tray
binary:

```text
cargo build -p true-tick --bin true-tick
dumpbin /DEPENDENTS target/debug/true-tick.exe
dumpbin /IMPORTS target/debug/true-tick.exe
dumpbin /HEADERS /SECTION:.rsrc target/debug/true-tick.exe
dumpbin /RAWDATA /SECTION:.rsrc target/debug/true-tick.exe
```

The evidence should show `COMCTL32.dll` with ordinal 345, a non-empty resource
directory and `.rsrc` section, and the embedded Common Controls dependency in
resource data. These checks do not launch the executable. They establish
embedding and static dependencies only. Runtime Windows resolution remains
unverified until the user runs the rebuilt tray app. The diagnostic window's
actual appearance, taskbar presence, title-bar controls, restore, and close
behavior also remain runtime-unverified because validation does not launch this
internal application.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).

## Audited v1 UX and diagnostics contract

The current True™ Tick menu order is `True™ Tick v<version>`, `Start`, `Stop`, `Pause >`, `Schedule >`, `Auto-start`, `Auto-time`, `Status >`, `Logs`, and `Quit`, with separators matching the native layout. Pause is one primary submenu with 5 minute, 15 minute, 30 minute, and 1 hour choices. Schedule contains Start in, Stop in, Cancel scheduled action, and `Resume now` when a pause is active. There is no duplicate Pause action, custom duration, persistence, stacking, or indefinite pause. Start is disabled while paused. Status is read-only and contains State, Timing, Running for, Next action, and Ownership. Logs is a separate top-level action.

While the popup is open, True™ Tick retains its root, Schedule, Pause, and Status menu handles and runs a popup-only 500 ms UI refresh timer. The timer updates dynamic Status rows, ownership, enabled states, and cancellation state. A separate one-second UI timer runs only while a schedule or pause is active. Its publication key includes the action generation and the rounded remaining-second bucket, so `Shell_NotifyIconW(NIM_MODIFY)` publishes each displayed countdown change without a high-frequency loop. Positive fractional remaining seconds round upward, so a new five-minute pause initially displays `5m 0s`. Elapsed durations remain floored.

Logs opens a normal taskbar window with a concise summary above a read-only native `SysListView32` report. The list and tooltip classes require one explicit `InitCommonControlsEx` call before controls are created, together with the embedded Common Controls v6 manifest. Columns are `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. `Row` is the current 1-based retained-session row position. `Sequence` is the event sequence number and is not a row selector. Each existing `DiagnosticStore` snapshot row is converted from typed `DiagnosticEvent` fields, inserted with `LVM_INSERTITEMW` on the list HWND, then filled with `LVM_SETITEMTEXTW` for each remaining column. List creation failure, negative row insertion, failed cell text updates, and a bounded refresh row count are recorded with native return values and raw Win32 status. The native control copies the bounded UTF-16 text during each synchronous message. Refresh is posted and coalesced on the diagnostic window UI thread. The session is local and bounded to 512 retained events with a truncation marker. It is not persisted or public telemetry. Invalid current timing is shown as `Unknown`, never as an old current value.

## Diagnostic report toolbar

The `True™ Tick Status and Diagnostics` window places two clearly separate toolbar groups between the status summary and the read-only Logs report. The Display group says `Show rows:`, accepts a positive number from 1 through the hard retained maximum of 512, and has a `Show all` override. The default is 100 rows. Display changes refresh the grid from the newest retained rows in chronological order. Empty or invalid display input leaves the last valid view unchanged and shows a short validation state.

The Transfer group says `Rows to copy/export:`. Its input is independent from `Show rows` and accepts empty or `all` for all retained rows, `12` for one retained row, or `12-24` for an inclusive range. Whitespace is trimmed. Row numbers are 1-based positions in the current retained snapshot, not event sequence IDs. For example, retained rows `353-354` can contain event sequences `753-754`. Hidden retained rows can be copied or exported intentionally. Malformed, reversed, zero, negative, and out-of-range values show a short validation message, disable both actions, and create a diagnostic validation event. A refresh preserves selected events by sequence when they remain retained. If they are gone, the transfer selection is visibly reset and recorded.

`Copy` and `Export` use exactly the selected retained rows from the full bounded snapshot, not the displayed slice. Both include the eleven report columns `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. The clipboard path uses standard Windows ownership transfer and releases temporary allocations safely. `Export` opens the standard Windows Save dialog with the concise default filename `true-tick-log.tsv`, then writes a UTF-8 TSV file with the same header and selected rows through a bounded same-directory temporary file replacement. Canceling the dialog is a normal logged cancellation and is not shown as an error.

Copy and export remain read-only with respect to timer ownership and timing state. Clipboard, dialog, and file failures show short messages in the window and retain raw native failure information in the bounded session diagnostics. The existing sanitization, field limits, row retention limit, and TSV row limit remain authoritative. No credentials, secrets, raw pointers, unbounded values, automatic files, telemetry, or uncontrolled logs are created.
