# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

The v1 uses a narrow native timer adapter with raw status preservation, a
serialized owned-request lifecycle, conservative power policy, a native tray
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
label. All activation paths use
the same conservative power policy, so battery, Battery Saver, and unknown power
states do not acquire. Events use monotonic sequence numbers and elapsed time
from process start. The default bound is 512 events. Newest events are retained
with a truncation marker when the bound is reached. Disk persistence for full
session logs is deferred.

Per-user boot registration targets the portable `Launcher.exe` entry point, never
a slot payload. The launcher remains responsible for selecting and validating
A/B before activation. Missing or invalid active-slot metadata reports repair
required and never silently selects A. Boot registration is opt-in through local
config and is distinct from automatic timer activation after launch. The current
path resolver only derives `Launcher.exe` from a `Slots\A` or `Slots\B`
executable shape. It does not claim runtime path or file validation. It does not
include profiles, process detection, True™ Time, NTP, an installer, a secure
updater, or release signing.

Unsupported platform behavior remains explicit. Native API acceptance and the
adapter postcondition are not claims about a universal effective system value.
Power observation currently verifies only the available system power query and
one power broadcast path. Battery Saver, session, lock, suspend, and resume
notification coverage remains incomplete and is shown as unknown or degraded
rather than claimed as fully observed.

Active documentation is checked with `scripts/check_doc_punctuation.py`. The
checker rejects em dash and semicolon characters and skips historical archive
material.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).
