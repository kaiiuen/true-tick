# Architecture

This repository contains an internal v1 runtime, not a production release.

- `tick-core` contains platform-neutral HNS and lifecycle/status/event/error types.
- `tick-policy` contains pure policy decisions and deduplicated logical reasons.
- `tick-ownership` models one serialized runtime instance's tracked contribution and idempotent transitions.
- `tick-platform-windows` isolates the native timer request and release calls.
- `tick-observation-windows` is the event and power observation boundary.
- `tick-ownership` serializes preflight, request, verification, postcondition, and release.
- `tick-diagnostics` owns truthful status formatting and a bounded synchronized in-memory session event store.
- `tick-calibration` remains an explicit unsupported future boundary. It is not a
  production control path.
- `tick-startup-windows` isolates opt-in current-user Run-key registration and removal.
- `apps/true-tick` owns the tray composition, startup configuration, and portable launcher framing.
- `apps/true-tick/src/tray_surface.rs` owns pure compact labels, tooltip text, status-to-color mapping, and status command mapping.

The v1 tray menu is intentionally compact. It contains `Start`, `Stop`, current
Auto-start and automatic timing activation toggles, a disabled short status item,
and Quit. Start and Stop remain the core manual controls and use the same guarded
policy and ownership lifecycle as automatic activation. The clickable status row
opens a normal overlapped taskbar diagnostic window and never changes timer state.
The two setting toggles use a non recursive return-command loop so the menu stays
open after a toggle. The diagnostic window is a normal taskbar window titled
`True Tick Status and Diagnostics`. It shows current status, power observation,
startup result, and the bounded read-only session snapshot. Closing it destroys
only the window and does not affect timer ownership. Full system reports, config
paths, raw HNS values, power explanations, and full errors remain excluded from
the compact menu and tooltip.

Detailed evidence is recorded in a local bounded in-memory session log. Events have
monotonic sequence numbers and elapsed process time. The default limit is 512
events, with newest-event retention and one truncation marker. The store sanitizes
control layout and bounds each field. Disk persistence for a full session log is
deferred. The UI refreshes from a current snapshot and shows the current tray state.

An inconclusive adapter postcondition enters an explicit uncertain ownership state
and suppresses repeat acquisition. The adapter retains enough request identity for
a controlled matching release or recovery attempt. Normal message-loop shutdown
uses one centralized cleanup guard, attempts release exactly once, and preserves an
unverified warning when cleanup cannot be confirmed. The runtime does not use a busy loop,
high priority, affinity, QoS, execution-state requests, power-plan changes,
registry tuning beyond the explicit current-user startup boundary, driver,
hardware clock control, process detection, network services, installer, secure
updater, or release machinery.
Boot startup registration is a per-user Run-key operation controlled by
`startup_enabled` in the local config. In portable A/B mode it registers the
buildable `Launcher.exe` entry point, not `true-tick.exe` from either slot. The
launcher owns slot selection and validation before activation, and launches only
the active slot with forwarded arguments. Missing or invalid metadata fails with
repair required. Secure signatures and rollback are not implemented. The bounded
path resolver derives the launcher path from a `Slots\A` or `Slots\B` executable
shape without claiming runtime filesystem validation. Registration is non-elevated,
idempotent, removable, and not performed by tests. It is not machine-wide
installation and it is not the same as `automatic`, which controls timer
activation after launch. Both settings are persisted through a flushed temporary
file replacement. Parse and write failures are surfaced in the tray status.
Missing or invalid A/B metadata requires repair and never defaults to slot A.
Startup registration validates launcher existence and executable identity before
writing. If config persistence fails after a registry change, the inverse
operation is attempted and failure is marked repair required. Initial power query
errors are recorded with their native status and remain conservative unknown
observation. True™ Time is not a dependency. Platform behavior that cannot be verified remains
yellow or red rather than being reported as green. The tray maps running and verified ownership to green, starting, stopping,
pending, degraded, or unverified behavior to yellow, and stopped, blocked,
unsupported, or error behavior to red. Each public tray status is reachable from
an explicit runtime path or is covered by a deterministic boundary test.
The tooltip uses `Running (current timing)` and `Stopped (current timing)` for
steady states, with concise transition labels such as `Starting...` and
`Stopping...`. Current observation verifies
only the available system power query and one power broadcast path. Full Battery
Saver, session, lock, suspend, and resume notification support is not claimed.
