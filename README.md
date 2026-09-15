# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

User-facing surfaces and notifications consistently use the branded product name
True™ Tick. Repository identifiers, package names, binary targets, and
technical paths retain the lowercase unspaced technical identifier true-tick.

## Lifecycle and timing invariants

The timer lifecycle is authoritative for the tray status. While a release handoff is active, the derived status is always yellow `Stopping, handoff` and unrelated configuration, startup, logging, or power diagnostics cannot replace it with red `Error`, `Stopped`, or `Blocked`. The icon color is derived from that same lifecycle state. A handoff completion clears the tracker and publishes red `Stopped` with the current effective timing in the standard `True™ Tick: Stopped · <timing>` tooltip format. A bounded timeout clears the tracker and publishes red `Stopped` with the current effective timing while ownership remains released. The tooltip always displays `True™ Tick: <state> · <timing>`, and never uses synthetic parenthetical status forms.

Manual and automatic changes use a latest-wins desired intent queue with only `Acquire` and `Release`. An intent received during `Starting` or `Stopping` is retained and processed after the transition. Native request and release calls remain serialized and idempotent. Power policy is applied when the queued intent is processed. Auto-time off on AC therefore remains stopped when no release is pending.

All tray text, menu status, diagnostic headers, handoff classification, and timer logs read from one synchronized timing snapshot. The snapshot keeps requested, selected, effective, raw bounds, and raw status distinct. Displayed timing is never hard coded. Every surface reads one synchronized runtime snapshot populated strictly from live queries. A failed current query invalidates effective timing and displays `Timing unknown` rather than retaining stale current data. Handoff polling observes the stored release boundary and never selects a new request interval.

Live timer selection is query-driven. The persisted zero value is the automatic-selection sentinel. The named legacy migration value is accepted only while migrating old configuration. Timer-resolution examples and fixture values belong only in tests or migration fixtures. Handoff polling intervals and observation budgets are separate handoff policy constants and are not timer-resolution defaults.

## Development commands

The workspace has two app packages, so commands from the workspace root select
the package and target explicitly. A bare run command from the workspace root
intentionally fails because multiple binary application packages exist without a declared
workspace default. Explicit package and binary selection is required to avoid ambiguous selection:

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

The report grid uses normal Windows multi-row selection with full-row selection,
visible selection, gridlines, and normal mouse drag selection. The selected summary
near the toolbar reports `Selected: N rows`. Row order in a transfer follows the
retained chronological event order, even when selection input arrives in another
order. `Row` is the retained snapshot position and `Sequence` is the event identity.
The grid shows a one-based retained row index separately from the monotonic sequence identity.
A trailing ellipsis in a details cell indicates field truncation due to buffer width,
not session event truncation. Grid accessibility ensures that every state has an explicit
text outcome column so status never depends on tint or colour alone.
Ctrl+C works when the grid has focus. Copy and Export use selected grid rows first.
When no grid rows are selected, both use the separate `Rows to copy/export:` range
input over the full retained snapshot. Display slicing is strictly independent from the
transfer range, which prevents a view filter from corrupting an export. A separate
`Export all` button bypasses both the grid selection and the transfer range and always
exports the complete unfiltered event snapshot, so an active category filter or
selection can never silently truncate a full export.
The `Show rows:` display limit never changes transfer selection. A `CBS_DROPDOWNLIST`
category combo box on the toolbar filters the grid by `EventCategory` in real time,
offering `All categories`, Startup, Timer, Power, UI, Schedule, and System. A
`CBN_SELCHANGE` selection records `diagnostic.category_filter.changed` and refreshes
the grid to show only matching events. Ctrl+A selects all currently
displayed rows when the grid has focus. A refresh preserves selected event sequences when retained. Truncation drops
invalid selections, preserves valid selected events, and shows and logs a concise
selection reset. These UI actions do not change timer state.

The diagnostic window uses a stable native rendering pipeline. Common Controls and
all child HWNDs are created before the parent is shown. Its compact resizable default
outer request is 960x520 at 96 DPI. The intended client minimum remains 916x324
logical pixels, including the one-row toolbar and a 96 pixel grid viewport, and
`WM_GETMINMAXINFO` derives the outer minimum from DPI-aware frame metrics. The
initial loading text is replaced by one synchronous bounded snapshot while the
parent is still hidden. That snapshot supplies the real HUD, summary, and grid rows,
converts each event to its eleven bounded cells once, and completes before the first
`ShowWindow`. Later refresh requests remain coalesced, and the retained event bound
remains 512.

A refresh suspends redraw while controls and the list are updated, then re-enables
redraw, explicitly invalidates and updates the summary `STATIC` and `ListView`, and
updates the parent window once within the same batch. The explicit post-redraw target
list now covers all 16 diagnostic children, including the category filter combo and
the Export all button. `WM_SIZE` only performs one deferred control
reposition pass. It does not snapshot data, rebuild rows, or fit columns. Horizontal
scrolling and header interaction do not fit columns. Auto-fit runs after the initial
population or a material data snapshot change, once per snapshot generation. Fixed
columns are bounded and Details absorbs remaining width. Normal refresh restores the
horizontal scroll position when Windows permits it. Reopening reuses the existing
window and posts one refresh rather than rebuilding synchronously.

The diagnostic summary is a restrained native HUD. A lifecycle-derived State line is larger and bold, followed by a compact three-line telemetry block with `|` separators that carries Effective timing, Ownership, Power, Startup, Running duration, Next action, the retained event history and visible row count, and the event-chain integrity status. The integrity field runs `verify_event_chain` over the current snapshot and shows `Chain: OK` when every entry hash and link verifies, or `Chain: Error at #<index>` naming the first offending retained row. Native etched separators make the HUD, one-row toolbar, and report grid distinct. The HUD is a read-only `STATIC` presentation with no `WS_VSCROLL`, `ES_AUTOVSCROLL`, or multiline edit style. Its base height is 104 logical pixels. At 96 DPI the toolbar client minimum is 830 pixels and the minimum client height is 240 pixels. `WM_GETMINMAXINFO` converts those intended client minimums to outer tracking dimensions with `AdjustWindowRectExForDpi`, with the legacy frame API as a fallback. Completing a redraw pass invalidates and updates all 14 diagnostic child controls explicitly, so no control stays blank after `WM_SETREDRAW` while it waits for a mouse hover.

Layout failure handling records `BeginDeferWindowPos`, every failed `DeferWindowPos`, and `EndDeferWindowPos`. It then positions every child with individual `SetWindowPos` calls. Child creation returns `-1` after every partial child set is destroyed. Reuse verifies the parent and every required child before showing the window.

`WM_SIZE` performs layout and Details viewport fitting only. `WM_DPICHANGED` applies the suggested parent rectangle, reapplies the system GUI font, recalculates layout, and preserves horizontal scroll when possible. The embedded Windows manifest declares Per-Monitor V2 with a `true/pm` fallback for the Windows 10 and Windows 11 target. The manifest omits explicit older DPI awareness declarations. While `GetDpiForWindow` and `WM_DPICHANGED` are present runtime paths, DPI compatibility remains unverified until a full Windows run. System colors and high-contrast behavior remain authoritative. Content auto-fit is limited to initial data load or a material snapshot generation. A refresh uses one snapshot and permits at most one follow-up pass for records written during that refresh. The retained session remains capped at 512 events.

The rendering contract is source-tested. Actual first-open paint timing, interactive
resize behavior, horizontal scroll appearance, and native taskbar window behavior
remain runtime-required checks because validation does not launch the app.

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
it. The compact tray menu starts with a clickable `True™ Tick v<version>` header sourced from Cargo package metadata. Root menu order is `True™ Tick v<version>`, a separator, `Start`, `Stop`, `Schedule >`, a separator, `Auto-start: On/Off`, `Auto-time: On/Off`, a separator, `Status >`, `Logs`, a separator, and `Quit`. Start and stop are primary and stay at the top. Scheduling and pausing are secondary and now share one submenu instead of two, so there is a single duration surface. The `Schedule >` submenu contains, in order, `Start in >`, `Stop in >`, `Pause for >`, a separator, `Cancel scheduled action`, and `Resume now`. The native menu builder, the internal label vector, and its tests all use `Pause for >`, so the label has one source of truth. Pause offers 5 minutes, 15 minutes, 30 minutes, and 1 hour. Schedule offers Start in, Stop in, Cancel scheduled action, and `Resume now` when a pause is active. Start and stop schedule choices offer 1 minute, 5 minutes, 15 minutes, 30 minutes, and 1 hour. Tooltips for Schedule actions explicitly state that selecting a new choice replaces any currently pending scheduled action. There are no two scheduling surfaces, custom duration, persistence, stacking, or indefinite pause. Auto-start controls launch at Windows login. Auto-time controls automatic timer acquisition after launch. The current defaults are `startup_enabled = true` and `automatic = false`, so login launch does not acquire timing until the user manually starts it.

`Status >` contains disabled State, Timing, Running for, Next action, and Ownership rows. Each informational row has a stable read-only menu identity so `WM_MENUSELECT` can show its hover description while the row remains disabled. The distinct hover descriptions per row are `Current True Tick lifecycle state`, `Latest verified effective timing observation`, `Elapsed time since verified running`, `Scheduled action and remaining time`, and `True Tick ownership versus external timing`. These rows cannot dispatch commands or open Logs. `Logs` is a separate top-level item below `Status >`, not a row inside Status. Status and Logs do not change timer state. The Status Timing row uses the latest valid authoritative effective observation and displays `Timing unknown` when evidence is invalid or stale. Running for uses monotonic time from the last verified Running transition. The diagnostic window remains titled `True™ Tick Status and Diagnostics`, with a read-only session view and snapshot refresh on reopen. It uses a normal taskbar window with standard title-bar controls and does not change timer state when closed.

The native menu keeps Start, Stop, and both setting toggles open after successful or failed handling. Each reopen uses the original popup anchor POINT. Highlighting a command uses a standard Windows tooltip. Descriptions cover Duration, Start in, Stop in, Pause for, Cancel scheduled action, Status, Logs, Start, Stop, startup, automatic timing, and Quit. The tooltip is destroyed when the popup closes and never changes timer state. Logs closes the menu when it opens diagnostics. Quit is dispatched from the returned `TPM_RETURNCMD` command. Tray left-button-up and right-button-up notifications open this same menu. Button-down and double-click notifications are ignored. Start and Stop remain manual controls and use the same guarded policy and ownership lifecycle as automatic activation. Start remains yellow through query, request, and verification, then turns green only after a verified request. Quit logs its request, active-state decision, dialog result, cleanup result, and exit permission. If timing is running, starting, stopping, pending, degraded, unverified, or ownership is uncertain, the warning offers exactly `Cancel` and `Stop and Quit`. Quitting while running, transitioning, degraded, unverified, or uncertain must confirm through this dialog, must complete cleanup before exit, and must keep the app open when cleanup fails. A failed or uncertain release keeps the app open and allows retry. The diagnostic window shows a bounded local in-memory session log.

The quit warning uses only the built-in warning-style `MessageBoxW` and remains a system-owned surface. When Tick owns
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
the handoff and shows red `Stopped` with the current effective timing. If the bound expires while a
finer value remains, the app shows red `Stopped` with external timing and logs that True
Tick ownership is released while another or unknown client keeps finer timing. There is no
synthetic parenthetical status string rendered by the application. This
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
session logs uses a rolling daily file rather than an unbounded single log. After
each diagnostic refresh and once at shutdown, a background thread appends every
newly recorded event to `Data/logs/true-tick-YYYY-MM-DD.csv` under the portable
root, or to `logs/` beside the executable in a non-portable layout. Each line is
an RFC 4180 CSV row with the eleven report columns plus hex-encoded `PrevHash` and
`EntryHash` columns. The app tracks `last_persisted_event_sequence` so only new
events are appended, and write failures are reported to stderr without altering
timer state. Session events retain operation_id, optional
parent_operation_id, correlation_id, finite phase, finite source, finite outcome,
and typed native NTSTATUS, Win32 last-error, requested HNS, selected HNS, and
effective HNS fields where available. Rendered text is bounded and sanitized.

Every event participates in a SHA-256 integrity chain. The store seeds the chain
from a deterministic genesis digest of `TrueTick-Genesis-v1`. Each event stores
the preceding entry hash in `prev_hash` and its own `entry_hash` over the link,
sequence, timestamp, name, and details. `verify_event_chain` replays a snapshot
and reports the first index whose content or link hash fails, so any modification
to a recorded event is detected. A six-variant `EventCategory` enum classifies
every event as Startup, Timer, Power, UI, Schedule, or System for the category
filter and for log review.
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
release signing. Furthermore, any future out-of-process observation is less invasive
than process injection but cannot guarantee anti-cheat compatibility or complete undetectability.
For packaging scope, per-user scope uses a user application data location, the
current-user Run key, and no elevation. Machine-wide scope uses a protected location
and elevation. The installer must never perform timer calls or install a driver.
Portable mode keeps state strictly with the executable, leaves no registry artifacts,
and skips silent startup registration.

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
persistence and a real registry operation are kept transactionally consistent. A
failed configuration save after an enable deliberately leaves the written registry
value in place and reports repair required instead of deleting a value this process
did not create, and a failed save after a disable restores the registration that
the persisted configuration still describes. Initial power
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

## Internal test package

The internal test package is assembled from a release build of both application
binaries. Packaging writes only to `artifacts/`, which is not tracked by Git. It
creates no tag, GitHub Release, or public artifact.

```text
cargo build --release -p true-tick -p true-tick-launcher --target-dir target/internal-package
```

The assembled portable layout matches the paths that the launcher and the tray
resolve at runtime (`apps/launcher/src/main.rs` and `apps/true-tick/src/portable.rs`):

```text
True-Tick-internal-v0.1.0/
  Launcher.exe
  active-slot.txt
  INTERNAL_TEST_README.txt
  SHA256SUMS.txt
  Slots/A/true-tick.exe
  Slots/A/true-tick.toml
  Slots/B/true-tick.exe
  Slots/B/true-tick.toml
```

`active-slot.txt` sits at the package root because the launcher resolves it from
its own directory, and each slot payload is resolved from the
`Slots/<slot>/true-tick.exe` shape. The tray derives the same portable root, so
the launcher and the portable path resolver agree on one layout.

Each slot ships a seed `true-tick.toml`. The seed uses `automatic = false`,
`startup_enabled = false`, and `request_interval_hns = 0`. The compiled default
for `startup_enabled` is `true`, and the seed differs deliberately. A test
package can be extracted to a temporary folder, so a disabled value removes an
existing TrueTick boot entry instead of writing an entry that points at a
temporary path. `request_interval_hns = 0` is the automatic-selection sentinel,
so the packaged configuration never pins a timer value. Every displayed timing
value still comes from a live native observation.

`SHA256SUMS.txt` records the hash of every other packaged file and excludes
itself. The archive round trip is verified by extracting the package again and
running `sha256sum -c SHA256SUMS.txt` in the extracted folder.

## Tray tooltip and ownership contract

The tray tooltip is branded and concise. It uses the current effective timing from the authoritative snapshot, never the requested interval. Valid timing is shown with a middle dot, for example `True™ Tick: Running · 0.497 ms`. Invalid timing is shown as `True™ Tick: Stopped · Timing unknown` or `Timing unknown`. Tooltips are always of the format `True™ Tick: <state> · <timing>`, and parenthetical forms such as `Stopped (current: X ms)` or `Stopped (external: X ms)` are not used on any surface. The states are `Stopped`, `Running`, `Starting`, `Stopping`, `Stopping, handoff`, `Starting in 5m 0s`, and `Paused for 5m 0s`. A plain paused state has no countdown. Countdowns explicitly include seconds to prevent ambiguity. Positive schedule and pause time rounds upward from the monotonic deadline. For a scheduled start, the tooltip displays the countdown while the timer remains released. For a scheduled stop, the tooltip shows the countdown while the timer remains owned until the deadline. Positive fractional remaining seconds round upward so a five minute choice initially reads as five minutes zero seconds (`5m 0s`).

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

## Diagnostic HUD, presentation, and evidence limits

The HUD State value is derived from the same lifecycle state used by the tray, not from a stale raw status field. The history field reports the actual retained event count, the 512 event cap, and whether a truncation marker is present. The grid retains all bounded raw details that fit the current snapshot.

The diagnostic window uses a small presentation layer only. It reapplies the system GUI font, a system-derived bold font for State, system background and text colors, ListView colors, and native control repainting for `WM_THEMECHANGED`, `WM_SYSCOLORCHANGE`, and `WM_SETTINGCHANGE`. High contrast follows system colors first. Tray status icon colors remain separate. Native menus, `MessageBoxW`, tooltips, and title-bar behavior remain Windows-owned. Native menus and dialogs may follow Windows theme behavior separately from this diagnostic surface.

The embedded Windows manifest declares Per-Monitor V2 with a `true/pm` fallback for the current Windows 10 and Windows 11 scope. The source handles `GetDpiForWindow` and `WM_DPICHANGED` with DPI-scaled layout plus DPI-aware frame metrics. Runtime DPI compatibility remains unverified until validation on supported displays is complete.

Every root tray operation records an `operation.begin` row with `phase=Begin` and `outcome=InProgress`, then exactly one terminal `operation.complete` outcome. Logs opening reports Failed when native window creation or initial layout fails. Refresh and nested presentation events retain the active operation context, and sub-events or native calls recorded while a root operation is active carry the root operation id as `parent_operation_id`, so the `Parent` column shows real operation lineage. Repeated identical layout failures are coalesced with a bounded repeat count and never report Completed.

The TSV header remains the eleven detailed columns `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. `Row` is the current retained snapshot position and `Sequence` is event identity. Copy and Export use the actual selected rows from one bounded snapshot, report retained count and truncation status, and never claim a complete session when the snapshot is filtered or truncated. The TSV row bound remains the 512 event retention bound.

## Audited v1 UX and diagnostics contract

The current True™ Tick menu order is `True™ Tick v<version>`, a separator, `Start`, `Stop`, `Schedule >`, a separator, `Auto-start`, `Auto-time`, a separator, `Status >`, `Logs`, a separator, and `Quit`, with separators matching the native layout. Start and stop are primary and stay at the top. Scheduling and pausing are secondary and now share one submenu instead of two, so there is a single duration surface. Inside `Schedule >`, contents are, in order, `Start in >`, `Stop in >`, `Pause for >`, a separator, `Cancel scheduled action`, and `Resume now`. The declared contract vector in `tray_surface.rs` and the rendered menu both use `Pause for >`. Pause contains fixed 5 minute, 15 minute, 30 minute, and 1 hour choices. There are no two scheduling surfaces, custom duration, persistence, stacking, or indefinite pause. Start is disabled while paused. Status is read-only and contains State, Timing, Running for, Next action, and Ownership. Logs is a separate top-level action below `Status >`, not an entry inside Status.

While the popup is open, True™ Tick retains its root, Schedule, and Status menu handles and runs a popup-only 500 ms UI refresh timer. The timer updates dynamic Status rows, ownership, enabled states, and cancellation state. A separate one-second UI timer runs only while a schedule or pause is active. Its publication key includes the action generation and the rounded remaining-second bucket, so `Shell_NotifyIconW(NIM_MODIFY)` publishes each displayed countdown change without a high-frequency loop. Positive fractional remaining seconds round upward, so a new five-minute pause initially displays `5m 0s`. Elapsed durations remain floored.

Logs opens a normal taskbar window with a concise summary above a read-only native `SysListView32` report. The window opens at a compact 960x520 client area at 96 DPI, scaled for the current DPI, converted to outer dimensions with DPI-aware frame metrics, and clamped to the primary work area so high-DPI displays no longer open an oversized window. The list and tooltip classes require one explicit `InitCommonControlsEx` call before controls are created, together with the embedded Common Controls v6 manifest. Columns are `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. `Row` is the current 1-based retained-session row position. `Sequence` is the event sequence number and is not a row selector. Each existing `DiagnosticStore` snapshot row is converted from typed `DiagnosticEvent` fields, inserted with `LVM_INSERTITEMW` on the list HWND, then filled with `LVM_SETITEMTEXTW` for each remaining column. A `CBS_DROPDOWNLIST` category combo box filters the grid by `EventCategory` in real time. List creation failure, negative row insertion, failed cell text updates, and a bounded refresh row count are recorded with native return values and raw Win32 status. The native control copies the bounded UTF-16 text during each synchronous message. Refresh is posted and coalesced on the diagnostic window UI thread. The session is local and bounded to 512 retained events with a truncation marker. A rolling daily CSV file under `Data/logs/` persists newly recorded events on a background thread, but the window itself is not public telemetry. Diagnostic logs include verified outcome evidence for window creation, icon registration, menu dispatches, and timer changes. Invalid current timing is shown as `Timing unknown`, never as an old current value.

## Diagnostic report toolbar

The `True™ Tick Status and Diagnostics` window places one fixed toolbar row between the status summary and the read-only Logs report. It keeps `Show rows:`, its input, `Show all`, a category filter combo box, `Rows to copy/export:`, its input, `Selected: N rows`, `Copy`, `Export`, and `Export all` on that row with compact fixed control widths. One shared `DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS` table defines the compact logical widths consumed by both `WM_CREATE` control creation and `WM_SIZE` layout, so every control stays inside the window at any DPI. The row is recalculated on `WM_SIZE` and never wraps or stacks. The default display limit is 100 rows. Display changes refresh the grid from the newest retained rows in chronological order. Empty or invalid display input leaves the last valid view unchanged and shows a short validation state.

The Transfer group says `Rows to copy/export:`. Its input is independent from `Show rows` and accepts empty or `all` for all retained rows, `12` for one retained row, or `12-24` for an inclusive range. Whitespace is trimmed. Row numbers are 1-based positions in the current retained snapshot, not event sequence IDs. For example, retained rows `353-354` can contain event sequences `753-754`. Hidden retained rows can be copied or exported intentionally. Malformed, reversed, zero, negative, and out-of-range values show a short validation message, disable both actions, and create a diagnostic validation event. A refresh preserves selected events by sequence when they remain retained. If they are gone, the transfer selection is visibly reset and recorded.

`Copy` and `Export` use exactly the selected retained rows from the full bounded snapshot, not the displayed slice. `Export all` ignores the selection, the transfer range, and the category filter entirely and exports the complete snapshot. Both include the eleven report columns `Row`, `Sequence`, `Elapsed`, `Operation`, `Parent`, `Correlation`, `Phase`, `Source`, `Outcome`, `Event`, and `Details`. All eleven columns are preserved as tab separated values to guarantee lossless downstream analysis. The clipboard path uses standard Windows ownership transfer and releases temporary allocations safely. `Export` and `Export all` open the standard Windows Save dialog with the concise default filename `true-tick-log.csv`, then write a UTF-8 file with the same header and selected rows through a bounded same-directory temporary file replacement. The dialog filter offers CSV first, then TSV, then all files, and the chosen extension selects the output format. Canceling the dialog is a normal logged cancellation and is not shown as an error.

The grid auto-fits headers and visible content through native ListView column-width messages after refresh and on resize. Fixed columns use DPI-scaled minimum and maximum widths. Details fills remaining client width and keeps horizontal scrolling available when bounded content needs more space. Edit controls use the system GUI font and a minimum height that keeps text vertically centered. The diagnostic class paints its client background with Windows system colors and handles erase, paint, and static or edit colors so widened areas do not become black while high-contrast colors remain available.

The report keeps native multi-row selection, including Shift+Click range selection, Ctrl+Click toggle selection, and rubber-band marquee selection. A dedicated list subclass draws a focus-rectangle band on left-drag once the system drag threshold is crossed and selects every displayed row whose bounds intersect the band. Holding Ctrl makes the band additive to the selection captured when the drag began, and a capture change ends an interrupted drag cleanly. Full-row selection, visible selection, and gridlines remain enabled. `LVN_ITEMCHANGED` updates `Selected: N rows`. Copy, Export, and grid-focused Ctrl+C use grid selection first and retain the range fallback when no grid rows are selected. These actions remain read-only with respect to timer ownership and timing state. Clipboard, dialog, and file failures show short messages in the window and retain raw native failure information in the bounded session diagnostics. The existing sanitization, field limits, row retention limit, and TSV row limit remain authoritative. No credentials, secrets, raw pointers, unbounded values, automatic files, telemetry, or uncontrolled logs are created.
