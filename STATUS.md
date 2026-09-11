# Status

**Phase:** internal v1 implementation, no release

The workspace builds an internal tray-only v1 with focused deterministic tests. The native loader diagnosis has been narrowed to likely manual FFI and library-link declarations. The timer adapter explicitly links `ntdll`, passes Windows BOOLEAN as `u8` values `1` or `0`, and preserves signed `i32` NTSTATUS values. Manual kernel32 declarations for file replacement, power observation, last-error retrieval, and module-path lookup explicitly link `kernel32`. Source inspection found no ordinal import mechanism, `GetProcAddress`, raw-dylib use, custom linker flags, or manifest import mechanism. This is not proof of the original loader cause until the built PE imports are inspected.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior. A postcondition mismatch preserves
uncertain ownership and suppresses duplicate acquisition until controlled cleanup
recovers or reports the state. Normal message-loop shutdown attempts centralized
cleanup once and keeps an unverified warning if release is not confirmed. Tray Quit now requires a native confirmation when timing is active or ownership is uncertain. `Stop and Quit` exits only after a guarded release is verified. Failed cleanup keeps the app alive, updates the icon and diagnostic log, and allows retry. Opt-in current-user boot startup registration is isolated behind its own Windows
adapter and targets the buildable portable `Launcher.exe` entry point. The launcher
requires valid active-slot metadata, validates the named A or B executable, and
launches that slot before activation. Registration is distinct from automatic timer
activation after the application has launched. Secure signatures and rollback are
not implemented.

The tray has compact `Start` and `Stop` manual controls, `Auto-start: On/Off` and
`Auto-time: On/Off` items, one clickable short status item, and Quit. Auto-start
launches the app at Windows login. Auto-time controls automatic timing acquisition
and defaults to off. The status row opens a normal taskbar diagnostic window titled `True Tick Status and Diagnostics` without changing timer state. It provides standard title-bar controls, a resizable read-only status and session log view, and a fresh snapshot each time it is reopened. The native menu keeps both setting toggles open after a toggle and closes for action commands.
Start and Stop use the same guarded policy and ownership lifecycle as automatic
activation. The tray tooltip uses `Running (current timing)`, `Stopped (current
timing)`, and concise transition or warning labels. Full system reports, config
paths, raw HNS values, power explanations, and full errors are intentionally
excluded from the tray surface and reserved for the diagnostic window. The window shows a local in-memory session log
from process start through the current moment. It uses monotonic sequence numbers
and elapsed process time, has a default 512 event bound, retains newest events with
a truncation marker, and defers disk persistence. Fields are sanitized and bounded
so raw pointers, credentials, private tokens, arbitrary secrets, and unbounded
sensitive paths are not recorded.
Configuration writes use a flushed temporary file replacement. Parse and write
errors remain visible. Startup registration validates the existing `Launcher.exe`
file before registry writes and attempts rollback if config persistence fails.
Initial power observation records success or the native failure reason, and failed
observation remains unknown and blocks acquisition. Manual and automatic activation share the same policy.
Battery, Battery Saver, and unknown power states remain non-acquiring. A
restrictive power transition attempts to release owned state. Normal Quit requires
a safe stop when timing is active or uncertain and reports failed cleanup as an
unverified warning rather than exiting.
Running and verified ownership is green. Starting, stopping, pending, degraded,
or unverified behavior is yellow. Stopped, blocked, unsupported, or error behavior
is red. Unsupported is produced when the native capability is unavailable. Error
is reserved for failed operations. The calibration crate remains future-only and
is not a disconnected production path.

Intentionally absent:

- profiles, application detection, foreground hooks, or child process management beyond launcher handoff
- calibration loops, benchmark loops, real-time priority, affinity, QoS, execution-state requests
- NTP, True™ Time integration, network code, installer, secure updater, signing, or release package
- public compatibility, performance, energy, security, or release claims
- runtime registration validation against the development machine
- interactive runtime validation of launcher handoff and slot execution
- interactive runtime validation of the Windows diagnostic window

The Windows support matrix, exact native API behavior, and runtime confirmation of the loader fix remain bounded internal validation work. The exact tray target must be built and its PE imports inspected without launching it. No runtime Windows success is claimed. Power observation does not yet provide full Battery Saver,
session, lock, suspend, or resume notification coverage. Unknown observation is
reported as degraded and blocks acquisition. A/B selection is a safe local
scaffold. Missing or invalid active-slot metadata requires repair. It does not
verify a signed package or perform rollback. No package, installer, signed
artifact, tag, GitHub Release, or public publication is created here.
