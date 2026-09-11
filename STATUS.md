# Status

**Phase:** internal v1 implementation, no release

The workspace builds an internal tray-only v1 with focused deterministic tests. PE inspection confirmed the pre-main loader failure: `target/debug/true-tick.exe` imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, without an embedded application manifest. Windows therefore loaded legacy Common Controls without version 6 activation and failed with `STATUS_ORDINAL_NOT_FOUND`. The tray package now embeds a checked-in manifest through its package-local `build.rs`. The manifest requests Common Controls version 6, declares Windows 10 and later compatibility, and requests normal `asInvoker` execution without administrator or UI access claims. The Launcher does not import `TaskDialogIndirect` or common controls version 6 APIs, so it does not receive this target-specific integration.

The timer adapter explicitly links `ntdll`, passes Windows BOOLEAN as `u8` values `1` or `0`, and preserves signed `i32` NTSTATUS values. It retains the raw `minimum` and `maximum` output labels and values from `NtQueryTimerResolution` for diagnostics, then selects the numerically smallest supported current boundary in automatic mode before inclusive validation. The labels are API field names, not an ordering guarantee. The captured values `minimum_hns=156250`, `maximum_hns=5000`, `current_hns=9966`, and old `requested_hns=10000` explain the observed effective value of about `0.997 ms`. Legacy `request_interval_hns=10000` is migrated to automatic selection. In that captured case, automatic selection would record `selected_hns=5000`, or `0.500 ms`, if the current system reports that boundary. This is not a universal fixed value. Manual kernel32 declarations for file replacement, power observation, last-error retrieval, and module-path lookup explicitly link `kernel32`. Static PE inspection is deterministic and non-running. Runtime Windows resolution of the loader failure remains unverified until the user runs the rebuilt tray binary.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior. A postcondition mismatch preserves
uncertain ownership and suppresses duplicate acquisition until controlled cleanup
recovers or reports the state. Normal message-loop shutdown attempts centralized
cleanup once and keeps an unverified warning if release is not confirmed. Tray Quit now requires a native confirmation when timing is active or ownership is uncertain. `Stop and Quit` exits only after a guarded release is verified. Failed cleanup keeps the app alive, updates the icon and diagnostic log, and allows retry. Opt-in current-user boot startup registration is isolated behind its own Windows
adapter. Portable A/B mode targets the buildable `Launcher.exe` entry point. The
launcher requires valid active-slot metadata, validates the named A or B
executable, and launches that slot before activation. A normal debug run uses a
separate fallback that registers the actual `target/debug/true-tick.exe` tray
executable when no portable launcher path is available. Registration is distinct
from automatic timer activation after the application has launched. Secure
signatures and rollback are not implemented.

The tray has compact `Start` and `Stop` manual controls, `Auto-start: On/Off` and
`Auto-time: On/Off` items, one clickable short status item, and Quit. Auto-start
launches the app at Windows login. Auto-time controls automatic timing acquisition
and defaults to off. Both tray left-button-up and right-button-up notifications open
the same compact menu. Button-down and double-click notifications are ignored to
avoid duplicate menus. The status row opens a normal taskbar diagnostic window titled `True Tick Status and Diagnostics` without changing timer state. It uses a normal overlapped style, `WS_EX_APPWINDOW`, no `WS_EX_TOOLWINDOW`, no child style, no owner, standard title-bar controls, a resizable read-only status and session log view, and a fresh snapshot each time it is reopened. Reopening restores and activates the existing window. The native menu keeps Start, Stop, and both setting toggles open after successful or failed handling. Reopening after any persistent command reuses the original popup anchor POINT, and returned command IDs are dispatched once. Status may close the menu when it opens diagnostics, and Quit closes normally.
Start and Stop use the same guarded policy and ownership lifecycle as automatic
activation. The tray tooltip and status menu use actual concise timing values such
as `Running (0.500 ms)`, `Running (0.497 ms, finer)`,
`Stopped (current: 0.497 ms)`, `Starting (0.500 ms)`,
`Stopping (current: 0.497 ms)`, and `Error (invalid interval)`. They use
`unknown` when no observation exists. After a successful request, the returned
verified effective observation replaces the preflight current value in the
controller and app snapshot. Release observations update that same snapshot. A
remaining finer or different value is displayed as external effective state and
does not claim Tick ownership. Query, request, release, and power
reconciliation observations are carried through controller and app state. Full
system reports, raw HNS values, power explanations, and full errors remain in the
diagnostic window. Its local session log records automatic selection, raw native
boundaries, selected HNS, requested HNS, effective HNS, and an `equal`, `finer`,
or `unverified` effective relation. The window shows a local in-memory session log
from process start through the current moment. It uses monotonic sequence numbers
and elapsed process time, has a default 512 event bound, retains newest events with
a truncation marker, and defers disk persistence. The diagnostic header uses the
latest verified effective observation and keeps requested HNS, selected HNS,
effective HNS, raw boundaries, raw status, and relation distinct. Fields are
sanitized and bounded so raw pointers, credentials, private tokens, arbitrary
secrets, and unbounded sensitive paths are not recorded.
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
Running and verified ownership is green. Starting, stopping, pending, degraded,
or unverified behavior is yellow. Stopped, blocked, unsupported, or error behavior
is red. Unsupported is produced when the native capability is unavailable. Error
is reserved for failed operations. Status icon canvases use current DPI where
available. The policy maps 100, 125, 150, 200, and 400 percent to 16, 20, 24, 32,
and 64 pixels, using the nearest supported canvas for intermediate values. The
tray shell may apply its own rendering scale. The calibration crate remains
future-only and is not a disconnected production path.

Intentionally absent:

- profiles, application detection, foreground hooks, or child process management beyond launcher handoff
- calibration loops, benchmark loops, real-time priority, affinity, QoS, execution-state requests
- NTP, True™ Time integration, network code, installer, secure updater, signing, or release package
- public compatibility, performance, energy, security, or release claims
- runtime registration validation against the development machine
- interactive runtime validation of launcher handoff and slot execution
- interactive runtime validation of the Windows diagnostic window appearance,
  taskbar presence, title-bar controls, restore, and close behavior

The Windows support matrix, exact native API behavior, and runtime confirmation of the loader fix remain bounded internal validation work. The exact tray target is built with `cargo build -p true-tick --bin true-tick` and inspected without launching it. Core Windows DLLs such as `kernel32.dll`, `user32.dll`, `ntdll.dll`, `shell32.dll`, `gdi32.dll`, and `comctl32.dll` are OS components and must not be copied or bundled. The embedded Common Controls v6 manifest is the compatibility mechanism. The current MSVC build has a non-system dependency on the Microsoft Visual C++ runtime and Universal CRT. A static binary scan found `VCRUNTIME140.dll` and `api-ms-win-crt-*` imports. The eventual distribution choice is a documented VC++ Redistributable prerequisite or a validated static CRT build. No installer or arbitrary DLL copy is added. The deterministic PE evidence check uses Visual Studio `dumpbin` for `/DEPENDENTS`, `/IMPORTS`, the `.rsrc` section headers, and `.rsrc` raw data. It must show the ordinal 345 import, a non-empty resource directory, and the embedded Common Controls dependency. No runtime Windows success is claimed here. The diagnostic window style contract is source-tested, but its actual appearance, taskbar registration, title-bar controls, restore, and close behavior remain runtime-unverified because the app is not launched. Power observation does not yet provide full Battery Saver,
session, lock, suspend, or resume notification coverage. Unknown observation is
reported as degraded and blocks acquisition. A/B selection is a safe local
scaffold. Missing or invalid active-slot metadata requires repair. It does not
verify a signed package or perform rollback. No package, installer, signed
artifact, tag, GitHub Release, or public publication is created here.
