# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

The v1 uses a narrow native timer adapter with raw status preservation, an
explicit released, owned, or uncertain ownership lifecycle, controlled recovery,
conservative power policy, a native tray
surface, atomically replaced local configuration, per-user boot startup
registration, and a bounded portable A/B launcher scaffold. The compact tray
menu exposes `Start`, `Stop`, current Auto-start and automatic timing activation
toggles, a clickable short status item, and `Quit`. Status opens a normal taskbar
diagnostic window titled `True Tick Status and Diagnostics` without changing timer
state. It is a normal taskbar window with standard title-bar controls, a resizable
read-only status and session log view, and snapshot refresh on reopen. The native
menu keeps the two setting toggles open after each toggle and closes for other
commands. Start and Stop remain
manual controls and use the same guarded policy and ownership lifecycle as automatic
activation. The diagnostic window shows a bounded, local in-memory session log.
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

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).
