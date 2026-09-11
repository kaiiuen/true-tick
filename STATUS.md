# Status

**Phase:** compile-oriented skeleton only

The workspace builds a standard-library-only model of future boundaries. Pure
state transitions, policy decisions, formatting, and explicit unsupported paths
have focused deterministic tests.

Intentionally absent:

- Windows timer, power, notification, process, tray, launcher, installer, updater, and FFI code
- real calibration or benchmark loops, real-time priority, affinity, QoS, MSR/TSC/HPET/APIC control
- network code, machine/user state, disk logging, manifest verification, A/B runtime, and True™ Time integration
- compatibility, performance, power, security, or release claims

The final timer API, Windows support matrix, lifecycle semantics, measurement
protocol, policy constants, integration boundaries, release design, licensing,
and publication authority remain open. No production implementation or release
is authorized by this repository. No package, installer, signed artifact, tag,
GitHub Release, or public publication is created here.
