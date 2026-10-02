# True Tick Source Diagrams

Every production source file in True Tick has exactly one architecture diagram written in Mermaid, which renders natively on GitHub. Each diagram describes the implemented control flow of a single source file, including decision points, error paths, and platform calls. The diagram tree under `docs/diagrams/` mirrors the repository source layout exactly, so the diagram for any source file can be found by replacing its extension with `.md` at the matching path.

## Layout

Diagrams follow a strict path mirror convention. A diagram for a source file at `apps/x/src/y.rs` lives at `docs/diagrams/apps/x/src/y.md`. Crate sources follow the same rule, so `crates/tick-core/src/lib.rs` maps to `docs/diagrams/crates/tick-core/src/lib.md`. Build scripts are mirrored the same way, so `apps/true-tick/build.rs` maps to `docs/diagrams/apps/true-tick/build.rs.md`.

## Index

| Diagram | Source file | Type |
|---------|-------------|------|
| [`apps/launcher/src/main.md`](apps/launcher/src/main.md) | `apps/launcher/src/main.rs` | flowchart |
| [`apps/manifest-tool/src/main.md`](apps/manifest-tool/src/main.md) | `apps/manifest-tool/src/main.rs` | flowchart |
| [`apps/true-tick/build.rs.md`](apps/true-tick/build.rs.md) | `apps/true-tick/build.rs` | flowchart |
| [`apps/true-tick/src/config.md`](apps/true-tick/src/config.md) | `apps/true-tick/src/config.rs` | flowchart |
| [`apps/true-tick/src/emergency.md`](apps/true-tick/src/emergency.md) | `apps/true-tick/src/emergency.rs` | flowchart |
| [`apps/true-tick/src/environment.md`](apps/true-tick/src/environment.md) | `apps/true-tick/src/environment.rs` | flowchart |
| [`apps/true-tick/src/logging/mod.md`](apps/true-tick/src/logging/mod.md) | `apps/true-tick/src/logging/mod.rs` | flowchart |
| [`apps/true-tick/src/main.md`](apps/true-tick/src/main.md) | `apps/true-tick/src/main.rs` | flowchart |
| [`apps/true-tick/src/pause.md`](apps/true-tick/src/pause.md) | `apps/true-tick/src/pause.rs` | flowchart |
| [`apps/true-tick/src/portable.md`](apps/true-tick/src/portable.md) | `apps/true-tick/src/portable.rs` | flowchart |
| [`apps/true-tick/src/session.md`](apps/true-tick/src/session.md) | `apps/true-tick/src/session.rs` | flowchart |
| [`apps/true-tick/src/shutdown.md`](apps/true-tick/src/shutdown.md) | `apps/true-tick/src/shutdown.rs` | flowchart |
| [`apps/true-tick/src/tray/controller.md`](apps/true-tick/src/tray/controller.md) | `apps/true-tick/src/tray/controller.rs` | flowchart |
| [`apps/true-tick/src/tray/icon.md`](apps/true-tick/src/tray/icon.md) | `apps/true-tick/src/tray/icon.rs` | flowchart |
| [`apps/true-tick/src/tray/ipc.md`](apps/true-tick/src/tray/ipc.md) | `apps/true-tick/src/tray/ipc.rs` | flowchart |
| [`apps/true-tick/src/tray/menu.md`](apps/true-tick/src/tray/menu.md) | `apps/true-tick/src/tray/menu.rs` | flowchart |
| [`apps/true-tick/src/tray/mod.md`](apps/true-tick/src/tray/mod.md) | `apps/true-tick/src/tray/mod.rs` | flowchart |
| [`apps/true-tick/src/tray_surface.md`](apps/true-tick/src/tray_surface.md) | `apps/true-tick/src/tray_surface.rs` | flowchart |
| [`apps/true-tick/src/ui/diagnostic_window.md`](apps/true-tick/src/ui/diagnostic_window.md) | `apps/true-tick/src/ui/diagnostic_window.rs` | flowchart |
| [`apps/true-tick/src/ui/marquee.md`](apps/true-tick/src/ui/marquee.md) | `apps/true-tick/src/ui/marquee.rs` | flowchart |
| [`apps/true-tick/src/ui/mod.md`](apps/true-tick/src/ui/mod.md) | `apps/true-tick/src/ui/mod.rs` | flowchart |
| [`apps/true-tick/src/ui/presets_window.md`](apps/true-tick/src/ui/presets_window.md) | `apps/true-tick/src/ui/presets_window.rs` | flowchart |
| [`apps/true-tick-cli/src/main.md`](apps/true-tick-cli/src/main.md) | `apps/true-tick-cli/src/main.rs` | flowchart |
| [`crates/tick-core/src/lib.md`](crates/tick-core/src/lib.md) | `crates/tick-core/src/lib.rs` | flowchart |
| [`crates/tick-crypto/src/lib.md`](crates/tick-crypto/src/lib.md) | `crates/tick-crypto/src/lib.rs` | flowchart |
| [`crates/tick-diagnostics/src/lib.md`](crates/tick-diagnostics/src/lib.md) | `crates/tick-diagnostics/src/lib.rs` | flowchart |
| [`crates/tick-diagnostics/src/privacy.md`](crates/tick-diagnostics/src/privacy.md) | `crates/tick-diagnostics/src/privacy.rs` | flowchart |
| [`crates/tick-diagnostics/src/recorder.md`](crates/tick-diagnostics/src/recorder.md) | `crates/tick-diagnostics/src/recorder.rs` | flowchart |
| [`crates/tick-diagnostics/src/retention.md`](crates/tick-diagnostics/src/retention.md) | `crates/tick-diagnostics/src/retention.rs` | flowchart |
| [`crates/tick-ipc/src/lib.md`](crates/tick-ipc/src/lib.md) | `crates/tick-ipc/src/lib.rs` | flowchart |
| [`crates/tick-multiclient/src/lib.md`](crates/tick-multiclient/src/lib.md) | `crates/tick-multiclient/src/lib.rs` | flowchart |
| [`crates/tick-multiclient/src/main.md`](crates/tick-multiclient/src/main.md) | `crates/tick-multiclient/src/main.rs` | flowchart |
| [`crates/tick-observation-windows/src/lib.md`](crates/tick-observation-windows/src/lib.md) | `crates/tick-observation-windows/src/lib.rs` | flowchart |
| [`crates/tick-ownership/src/lib.md`](crates/tick-ownership/src/lib.md) | `crates/tick-ownership/src/lib.rs` | stateDiagram-v2 |
| [`crates/tick-platform-windows/src/lib.md`](crates/tick-platform-windows/src/lib.md) | `crates/tick-platform-windows/src/lib.rs` | flowchart |
| [`crates/tick-policy/src/lib.md`](crates/tick-policy/src/lib.md) | `crates/tick-policy/src/lib.rs` | flowchart |
| [`crates/tick-startup-windows/src/lib.md`](crates/tick-startup-windows/src/lib.md) | `crates/tick-startup-windows/src/lib.rs` | flowchart |
| [`crates/tick-watchdog/src/lib.md`](crates/tick-watchdog/src/lib.md) | `crates/tick-watchdog/src/lib.rs` | flowchart |

## Conventions

- One diagram per production source file, no more and no fewer.
- Each diagram is accurate to the implemented code as it exists in the repository.
- No aspirational content. Diagrams describe what is built, not what is planned.
