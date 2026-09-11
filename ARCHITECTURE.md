# Architecture

This repository contains an internal v1 runtime, not a production release.

- `tick-core` contains platform-neutral HNS and lifecycle/status/event/error types.
- `tick-policy` contains pure policy decisions and deduplicated logical reasons.
- `tick-ownership` models one serialized runtime instance's tracked contribution and idempotent transitions.
- `tick-platform-windows` isolates the native timer request and release calls.
- `tick-observation-windows` is the event and power observation boundary.
- `tick-ownership` serializes preflight, request, verification, postcondition, and release.
- `tick-diagnostics` formats truthful status and evidence records.
- `tick-calibration` remains an explicit unsupported boundary.
- `tick-startup-windows` isolates opt-in current-user Run-key registration and removal.
- `apps/true-tick` owns the tray composition, startup configuration, and portable launcher framing.

An inconclusive adapter postcondition enters an unverified state and suppresses
repeat acquisition or guessed cleanup. The runtime does not use a busy loop,
high priority, affinity, QoS, execution-state requests, power-plan changes,
registry tuning beyond the explicit current-user startup boundary, driver,
hardware clock control, process detection, network services, installer, secure
updater, or release machinery.
Boot startup registration is a per-user Run-key operation controlled by
`startup_enabled` in the local config. It is not machine-wide installation and
it is not the same as `automatic`, which controls timer activation after launch.
True™ Time is not a dependency. Platform behavior that cannot be verified remains
yellow or red rather than being reported as green.
