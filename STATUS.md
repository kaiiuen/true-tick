# Status

**Phase:** internal v1 implementation, no release

The workspace builds an internal tray-only v1 with focused deterministic tests.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior. Opt-in current-user boot startup
registration is isolated behind its own Windows adapter and targets the portable
`Launcher.exe` entry point. The launcher remains the component that selects and
validates A/B before activation. Registration is distinct from automatic timer
activation after the application has launched.

Intentionally absent:

- profiles, application detection, foreground hooks, launcher runtime validation or child processes
- calibration loops, benchmark loops, real-time priority, affinity, QoS, execution-state requests
- NTP, True™ Time integration, network code, installer, secure updater, signing, or release package
- public compatibility, performance, energy, security, or release claims
- runtime registration validation against the development machine
- runtime validation of the resolved portable launcher path

The Windows support matrix and exact native API behavior remain bounded internal
validation work. A/B selection is a safe local scaffold and does not verify a
signed package or perform rollback. No package, installer, signed artifact, tag,
GitHub Release, or public publication is created here.
