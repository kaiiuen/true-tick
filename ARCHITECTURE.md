# Architecture

This repository is a compile-oriented skeleton, not a production runtime.

- `tick-core` contains platform-neutral HNS and lifecycle/status/event/error types.
- `tick-policy` contains pure policy decisions and deduplicated logical reasons.
- `tick-ownership` models one serialized runtime instance's tracked contribution and idempotent transitions.
- `tick-platform-windows` defines the future timer boundary and returns `Unsupported`.
- `tick-observation-windows` defines the future observation boundary and returns `Unsupported`.
- `tick-diagnostics` formats in-memory truthful status/evidence records only.
- `tick-calibration` defines a bounded future result model and refuses execution.
- `apps/true-tick` is a no-side-effect composition placeholder.

No crate currently calls Windows APIs, native FFI, power APIs, event hooks,
process detection, tray/UI libraries, network services, or release machinery.
True™ Time is not a dependency. API, lifecycle, support, policy, measurement,
security, installation, and integration decisions remain unresolved.
