# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

## Lifecycle and timing invariants

The timer lifecycle is authoritative for the tray status. While a release handoff is active, the derived status is always yellow `Stopping` and unrelated configuration, startup, logging, or power diagnostics cannot replace it with red `Error`, `Stopped`, or `Blocked`. The icon color is derived from that same lifecycle status. A handoff completion clears the tracker and publishes red `Stopped` with the current timing. A bounded timeout clears the tracker and publishes red `Stopped, external timing`.

Manual and automatic changes use a latest-wins desired intent queue with only `Acquire` and `Release`. An intent received during `Starting` or `Stopping` is retained and processed after the transition. Native request and release calls remain serialized and idempotent. Power policy is applied when the queued intent is processed. Auto-time off on AC therefore remains stopped when no release is pending.

All tray text, menu status, diagnostic headers, handoff classification, and timer logs read from one synchronized timing snapshot. The snapshot keeps requested, selected, effective, raw bounds, and raw status distinct. A failed current query invalidates effective timing and displays `unknown` rather than retaining stale current data. Handoff polling observes the stored release boundary and never selects a new request interval.

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
py -3 scripts/check_live_timer_values.py
```

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
intended development targets. The commands above are not runtime validation of
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
it. The compact tray menu starts with a disabled `True™ Tick v<version>` header sourced from Cargo package metadata. It then exposes `Start`, `Stop`, `Auto-start: On/Off`, `Auto-time: On/Off`, a disabled concise status summary, a clickable `Status` command, and `Quit`. Auto-start controls launch at Windows login. Auto-time controls automatic timer acquisition after launch. The current defaults are `startup_enabled = true` and `automatic = false`, so login launch does not acquire timing until the user manually starts it. The summary row does not dispatch commands. `Status` opens a normal taskbar
diagnostic window titled `True™ Tick Status and Diagnostics` without changing timer
state. It is a normal taskbar window with standard title-bar controls, a resizable
read-only status and session log view, and snapshot refresh on reopen. The native
window uses a normal overlapped style with `WS_EX_APPWINDOW`, no
`WS_EX_TOOLWINDOW`, no child style, and no owner, so it is intended to appear in
the taskbar with minimize, maximize, restore, close, and resize behavior. Closing it only destroys the diagnostic
window and does not change timer state. The native menu keeps Start, Stop, and both setting toggles open after successful
or failed handling. Each reopen uses the original popup anchor POINT, so the menu
does not jump when the cursor moves. While a command is highlighted, the native
menu-selection path shows a short standard Windows tooltip. Start, Stop, startup,
automatic timing, Status, and Quit use the descriptions `Request the best supported
timing`, `Release True Tick timing`, `Launch True Tick when you sign in`, `Request
timing automatically on AC power`, `Open status and session logs`, and `Stop safely
and quit`. The tooltip is destroyed when the popup closes and it never changes timer
state. Status closes the menu when it opens the diagnostic window. Quit is dispatched
from the returned `TPM_RETURNCMD` command. Tray
left-button-up and right-button-up
notifications open this same menu. Button-down and double-click notifications are
ignored, so one Windows notification does not create duplicate menus. Start and Stop
remain manual controls and use the same guarded policy and ownership lifecycle as
automatic activation. Start remains yellow through query, request, and verification, and turns green only after a verified request. Quit logs its request, active-state decision, dialog result, cleanup result, and exit permission. If timing is running, starting, stopping, pending, degraded, unverified, or ownership is uncertain, the warning offers exactly `Cancel` and `Stop and Quit`. Cancel leaves timing, the application, and the popup command loop unchanged. Stop and Quit uses the same guarded release path as Stop and exits only after release verification. A failed or uncertain release keeps the app open, updates status and diagnostics, and allows retry. If the state is definitely stopped with no pending ownership, Quit exits without a warning. The diagnostic window shows a bounded, local in-memory session log.

The quit warning now uses the built-in warning-style `MessageBoxW` as its primary
path because the captured `TaskDialogIndirect` HRESULT was `0x80070057`. `Yes`
means Stop and Quit and `No`, close, zero, unknown, or MessageBox failure means
Cancel. The exact MessageBox result and final fail-closed decision are logged. The
retained Task Dialog implementation is explicitly secondary and is not invoked before
the reliable MessageBox path. If no warning is shown, the menu is closed rather than
silently reopened.
It excludes raw pointers, private tokens, credentials, arbitrary secrets, and
unbounded sensitive paths. The branded tooltip uses short runtime values such as
`True™ Tick: Running 0.497 ms`, `True™ Tick: Stopped 0.997 ms`,
`True™ Tick: Starting 0.500 ms`, `True™ Tick: Stopping`, `True™ Tick: Warning`,
and `True™ Tick: Error`. The disabled menu summary uses concise labels such as
`Status: Running (0.497 ms)`, `Status: Stopped (0.997 ms)`,
`Status: Starting (0.500 ms)`, `Status: Stopping (external timing)`, or
`Status: Error (invalid interval)`. Values use the selected request and the latest verified effective
observation as distinct fields, and another platform boundary is allowed. After a
successful request, the returned effective observation replaces the preflight
current value in controller and app state. After release, the returned current
observation is retained as effective external state when another client remains
finer or otherwise active, without implying Tick ownership. After True™ Tick releases
its request, a finer effective value enters yellow `Stopping (waiting for handoff)`.
The app queries the effective value only while that handoff is pending, using a
250 ms Windows timer for at most 12 observations. A non-finer observation completes
the handoff and shows red `Stopped (current: X ms)`. If the bound expires while a
finer value remains, the app shows red `Stopped (external: X ms)` and logs that True
Tick ownership is released while another or unknown client keeps finer timing. This
classification also applies to battery-policy and other owned-request releases.
The captured `current_hns=9966` value was about `0.997 ms` because the old config requested
`10000 HNS`. That config is migrated to automatic selection. At startup, the controller always queries the current effective timing, even when True™ Tick is stopped and Auto-time is off. A successful query is displayed as stopped current timing without claiming Tick ownership. A failed query is displayed as `Stopped (timing unknown)` and recorded as a diagnostic failure. The selected or requested boundary remains separate from the current effective observation.

Power broadcasts refresh timing and recalculate the visible state for both Auto-time settings. With Auto-time off, AC and no owned request show stopped current timing and wait for manual Start. Battery, Battery Saver, and unknown power remain conservative and release owned timing when policy requires it. Returning to AC with Auto-time on attempts acquisition. Returning to AC with Auto-time off shows stopped current timing rather than leaving a stale blocked state.

The display uses `unknown` when no observation is available. Raw HNS and full event details remain in the diagnostic window. The local diagnostic session records automatic selection,
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
session logs is deferred.

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

Active documentation is checked with `scripts/check_doc_punctuation.py`. The
checker rejects em dash and semicolon characters and skips historical archive
material.

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
claim. The Launcher does not import `TaskDialogIndirect` or use common
controls version 6 APIs, so it does not need this target-specific manifest.

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
