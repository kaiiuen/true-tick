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
- `apps/true-tick` owns the tray composition, startup configuration, and portable launcher framing.

The runtime does not use a busy loop, high priority, affinity, QoS, execution-state
requests, power-plan changes, registry tuning, driver, hardware clock control,
process detection, network services, installer, secure updater, or release machinery.
True™ Time is not a dependency. Platform behavior that cannot be verified remains
yellow or red rather than being reported as green.
