# Status

**Phase:** internal v1 implementation, no release

The workspace builds an internal tray-only v1 with focused deterministic tests.
Timer ownership is isolated behind a Windows adapter and remains explicit about
API acceptance versus effective system behavior.

Intentionally absent:

- profiles, application detection, foreground hooks, launchers or child processes
- calibration loops, benchmark loops, real-time priority, affinity, QoS, execution-state requests
- NTP, True™ Time integration, network code, installer, secure updater, signing, or release package
- public compatibility, performance, energy, security, or release claims

The Windows support matrix and exact native API behavior remain bounded internal
validation work. A/B selection is a safe local scaffold and does not verify a
signed package or perform rollback. No package, installer, signed artifact, tag,
GitHub Release, or public publication is created here.
