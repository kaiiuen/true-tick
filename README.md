# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

## Development commands

The workspace has two app packages, so commands from the workspace root select
the package and target explicitly. These checks do not launch an app or change
Windows state:

```text
cargo metadata --no-deps --format-version 1
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

To launch the tray app during an explicitly authorized development run, use:

```text
cargo run -p true-tick --bin true-tick
```

The portable Launcher target is separate:

```text
cargo run -p true-tick-launcher --bin Launcher
```

Within `apps/true-tick`, `cargo run` defaults to the `true-tick` tray target.
The workspace-root command remains explicit because both app targets are
intended development targets. The commands above are not runtime validation of
tray behavior, startup registration, or launcher handoff.

The v1 uses a narrow native timer adapter with raw status preservation, an
explicit released, owned, or uncertain ownership lifecycle, controlled recovery,
conservative power policy, a native tray
surface, atomically replaced local configuration, per-user boot startup
registration, and a bounded portable A/B launcher scaffold. The compact tray
menu exposes `Start`, `Stop`, `Auto-start: On/Off`, `Auto-time: On/Off`, a clickable short status item, and `Quit`. Auto-start controls launch at Windows login. Auto-time controls automatic timer acquisition after launch. The new defaults are `startup_enabled = true` and `automatic = false`, so login launch does not acquire timing until the user manually starts it. Status opens a normal taskbar
diagnostic window titled `True Tick Status and Diagnostics` without changing timer
state. It is a normal taskbar window with standard title-bar controls, a resizable
read-only status and session log view, and snapshot refresh on reopen. The native
menu keeps the two setting toggles open after each toggle and closes for other
commands. Start and Stop remain
manual controls and use the same guarded policy and ownership lifecycle as automatic
activation. Quit requires a safe stop when timing is active or ownership is uncertain. The warning offers `Cancel` and `Stop and Quit`. The app exits only after owned-request release is verified. A failed or uncertain release keeps the app alive and records the retryable warning. The diagnostic window shows a bounded, local in-memory session log.
It excludes raw pointers, private tokens, credentials, arbitrary secrets, and
unbounded sensitive paths. The tooltip uses short runtime wording: `Running
(current timing)`, `Stopped (current timing)`, or a concise transition or warning
label. Unsupported native timer capability is surfaced as `Unsupported`, while
error, blocked, degraded, unverified, starting, stopping, running, and stopped
retain consistent icon colors and menu meanings. Calibration remains an explicit
future-only boundary and no profiles are added. All activation paths use
the same conservative power policy, so battery, Battery Saver, and unknown power
states do not acquire. If a native request may have succeeded but its postcondition
is inconclusive, the runtime records uncertain ownership, blocks duplicate acquire,
and attempts only a controlled matching release. Normal message-loop shutdown
attempts this cleanup once and preserves an unverified warning when it fails. Events use monotonic sequence numbers and elapsed time
from process start. The default bound is 512 events. Newest events are retained
with a truncation marker when the bound is reached. Disk persistence for full
session logs is deferred.

Per-user boot registration targets the buildable portable `Launcher.exe` entry
point, never a slot payload. The launcher requires `active-slot.txt`, selects only
the named A or B slot, validates the expected `true-tick.exe` file, and launches
that slot with forwarded arguments. Missing or invalid metadata reports repair
required and never silently selects A. Secure signatures and rollback are not
implemented and remain deferred. Boot registration is opt-in through local
config and is distinct from automatic timer activation after launch. The path resolver only derives `Launcher.exe` from a `Slots\A` or `Slots\B`
executable shape. The launcher performs the separate runtime metadata and file
checks before handoff. It does not
include profiles, process detection, True™ Time, NTP, an installer, a secure
updater, or release signing.

Unsupported platform behavior remains explicit. Native API acceptance and the
adapter postcondition are not claims about a universal effective system value.
Power observation currently verifies only the available system power query and
one power broadcast path. Battery Saver, session, lock, suspend, and resume
notification coverage remains incomplete and is shown as unknown or degraded
rather than claimed as fully observed.

Startup registration validates that the target exists and is exactly
`Launcher.exe` before writing the current-user value. Config persistence and the
registry operation are kept transactionally consistent with a rollback attempt or
an explicit repair-needed status. Initial power observation records success or the
failure reason in the diagnostic log. A failed observation remains unknown and
blocks acquisition.

Active documentation is checked with `scripts/check_doc_punctuation.py`. The
checker rejects em dash and semicolon characters and skips historical archive
material.

## Native loader diagnosis

The confirmed pre-main failure was in the tray PE. `target/debug/true-tick.exe`
imported ordinal 345 from `COMCTL32.dll`, which is `TaskDialogIndirect`, but
had no embedded application manifest. Windows therefore loaded legacy
Common Controls without version 6 activation and failed with
`STATUS_ORDINAL_NOT_FOUND` before `main`.

The tray target now embeds `apps/true-tick/windows/true-tick.manifest`. The
package-local `build.rs` passes `/MANIFEST:EMBED` and `/MANIFESTINPUT` only for
MSVC Windows builds. The manifest requests Common Controls version 6, declares
Windows 10 and later compatibility through the Windows 10 supported-OS
identifier, and requests normal user execution with `asInvoker`. It makes no
administrator, UI access, legacy Windows, performance, or runtime success
claim. The Launcher does not import `TaskDialogIndirect` or use common
controls version 6 APIs, so it does not need this target-specific manifest.

The Windows timer adapter links `NtQueryTimerResolution` and
`NtSetTimerResolution` explicitly from `ntdll`. Its `NtSetTimerResolution`
BOOLEAN argument crosses the Rust boundary as an explicit `u8` value of `1` or
`0`. Native NTSTATUS values cross the boundary as signed `i32` values and are
preserved in timer observations and errors. Manually declared kernel32 APIs
for configuration replacement, power observation, last-error retrieval, and
module-path lookup also have explicit `kernel32` links.

Use this deterministic non-running PE check after building the exact tray
binary:

```text
cargo build -p true-tick --bin true-tick
dumpbin /DEPENDENTS target/debug/true-tick.exe
dumpbin /IMPORTS target/debug/true-tick.exe
dumpbin /HEADERS /SECTION:.rsrc target/debug/true-tick.exe
dumpbin /RAWDATA /SECTION:.rsrc target/debug/true-tick.exe
```

The evidence should show `COMCTL32.dll` with ordinal 345, a non-empty resource
directory and `.rsrc` section, and the embedded Common Controls dependency in
resource data. These checks do not launch the executable. They establish
embedding and static dependencies only. Runtime Windows resolution remains
unverified until the user runs the rebuilt tray app.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).
