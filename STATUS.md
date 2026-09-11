# Status

**Phase:** internal v1 implementation, no release

The workspace builds an internal tray-only v1 with focused deterministic tests.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior. Opt-in current-user boot startup
registration is isolated behind its own Windows adapter and targets the portable
`Launcher.exe` entry point. The launcher remains the component that selects and
validates A/B before activation. Registration is distinct from automatic timer
activation after the application has launched.

The tray has compact `Start` and `Stop` manual controls, current-state Auto-start
and automatic timing activation items, one disabled short status item, and Quit.
Start and Stop use the same guarded policy and ownership lifecycle as automatic
activation. The tray tooltip uses `Running (current timing)`, `Stopped (current
timing)`, and concise transition or warning labels. Full system reports, config
paths, raw HNS values, power explanations, and full errors are intentionally
excluded from the tray surface and reserved for logs or future diagnostics.
Configuration writes use a flushed temporary file replacement. Parse and write
errors remain visible. Manual and automatic activation share the same policy.
Battery, Battery Saver, and unknown power states remain non-acquiring. A
restrictive power transition or normal Quit attempts to release owned state and
reports release failure as warning or stopped according to the truthful state map.
Running and verified ownership is green. Starting, stopping, pending, degraded,
or unverified behavior is yellow. Stopped, blocked, unsupported, or error behavior
is red.

Intentionally absent:

- profiles, application detection, foreground hooks, launcher runtime validation or child processes
- calibration loops, benchmark loops, real-time priority, affinity, QoS, execution-state requests
- NTP, True™ Time integration, network code, installer, secure updater, signing, or release package
- public compatibility, performance, energy, security, or release claims
- runtime registration validation against the development machine
- runtime validation of the resolved portable launcher path

The Windows support matrix and exact native API behavior remain bounded internal
validation work. Power observation does not yet provide full Battery Saver,
session, lock, suspend, or resume notification coverage. Unknown observation is
reported as degraded and blocks acquisition. A/B selection is a safe local
scaffold. Missing or invalid active-slot metadata requires repair. It does not
verify a signed package or perform rollback. No package, installer, signed
artifact, tag, GitHub Release, or public publication is created here.
