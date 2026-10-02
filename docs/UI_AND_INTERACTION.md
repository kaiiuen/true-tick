# True Tick UI and Interaction Guide

This guide describes the user-facing surfaces of True Tick: the system tray
application `true-tick.exe` and the companion command line tool
`true-tick-cli.exe`. It covers the tray menu structure, icon states, tooltip
and status readouts, multi-monitor menu behavior, the full CLI subcommand
surface, and the daemon launch path.

For a per-file architectural view of the same code, see the Mermaid diagrams
under `docs/diagrams/`. That tree mirrors the repository source layout one
diagram per source file and is the alternate visual view of everything
described here.

## Tray interface

The tray icon lives in the Windows notification area. Clicking it with either
mouse button opens the root menu once. The menu is tracked with
`TPM_RETURNCMD`, so commands dispatch through the application rather than
through accelerator handling.

### Root menu

The root menu contains eight entries in this order, with separators between
the groups:

1. **True™ Tick vX.Y.Z** - version header showing the running package
   version. Clicking it opens the True Tick page on GitHub.
2. **Start** - requests the best supported timer resolution. Disabled while
   already running or starting, and while a pause is active.
3. **Stop** - releases True Tick timing. Disabled while stopped, stopping, or
   paused.
4. **Schedule >** - opens the Schedule submenu.
5. **Settings >** - opens the Settings submenu.
6. **Status >** - opens the read-only Status submenu.
7. **Logs** - opens the diagnostic window with status and session logs.
8. **Quit** - stops safely and exits. If timing is still owned or ownership
   is uncertain, a warning dialog asks for confirmation first.

Start, Stop, and the Settings toggles keep the menu open after activation so
several adjustments can be made in one pass. After a Settings toggle the menu
reopens directly on the Settings submenu with updated labels. Logs, the
version header link, and Quit close the menu.

### Schedule submenu

The Schedule submenu contains six entries in this order:

1. **Start in >** - submenu of configured presets that schedule a guarded
   acquire at a future time.
2. **Stop in >** - submenu of configured presets that schedule a guarded
   release at a future time.
3. **Pause for >** - submenu of configured presets that suppress acquisition
   for a fixed duration.
4. **Interval presets...** - opens the preset manager window where the preset
   list is edited.
5. **Cancel scheduled action** - clears the pending scheduled start or stop.
   Enabled only while such an action is armed.
6. **Resume now** - cancels an active pause and resumes timing. Enabled only
   while a pause is active.

The Start in, Stop in, and Pause for lists are populated from the configured
interval presets rather than fixed entries. The factory defaults are 1 minute,
5 minutes, 15 minutes, 30 minutes, and 1 hour. Pauses offer 5 minutes through
1 hour by default. Presets are stored as seconds between 10 and 86400, at most
twelve entries, and each label is rendered in a natural unit such as
"5 minutes" or "1h 30m".

While a pause is active, Start and every Start in preset are disabled, since
acquisition is suppressed until the pause ends or Resume now is used.

### Settings submenu

The Settings submenu contains four toggles and one action in this order:

1. **Auto-start: On|Off** - launch True Tick when you sign in.
2. **Auto-time: On|Off** - request timing automatically on AC power.
3. **Auto-resume on AC: On|Off** - resume timing when returning to AC power
   after a battery release.
4. **Battery lockout: On|Off** - restrict timing requests on DC power.
5. **Reset to factory defaults...** - separated by a divider, restores the
   default configuration after a Yes/No confirmation. A confirmed reset stops
   any active contribution, clears pending scheduled actions, writes the
   default config, restores the default preset list, reapplies the startup
   registration, and republishes state.

Each toggle label reflects the current state, so the menu shows the value you
have, not the value you would switch to.

## Tray icon states

The icon is drawn in one of three colors that map directly to the internal
lifecycle status:

- **Green** - verified running. True Tick owns the timer resolution and the
  effective timing has been confirmed.
- **Yellow** - transitional or degraded. This covers starting, stopping,
  scheduled start, scheduled stop, paused, pending handoff to a finer client,
  degraded observation, and unverified timing. The icon stays yellow until the
  lifecycle settles into a verified or terminal state.
- **Red** - stopped or error. This covers stopped, blocked by power policy,
  unsupported hardware, and error states.

## Tooltip and Status submenu

Hovering the icon shows a tooltip built from the same snapshot that drives
the menus, so the two never disagree. The general format is:

`True™ Tick: <State> · <Timing>`

Examples:

- `True™ Tick: Running · 0.500 ms`
- `True™ Tick: Stopped` (no timing suffix when nothing is verified)
- `True™ Tick: Starting in 4m 32s · 1.000 ms`
- `True™ Tick: Paused for 14m 05s · Timing unknown`
- `True™ Tick: Blocked (Battery) · Timing unknown`
- `True™ Tick: Warning` for blocked states without a named power reason
- `True™ Tick: Timing unknown` when observation is unverified
- `True™ Tick: Error` for a hard error

The `Blocked` suffix names the restricting power state: `Battery`,
`Battery Saver`, or `Power unknown`.

The Status submenu shows five read-only rows in this order. All are grayed
out because they are informational:

1. `State: <state>` - current lifecycle state, or a live countdown such as
   `Starting in 2m 10s` while a schedule is armed.
2. `Timing: <value>` - latest verified effective timing, or `Timing unknown`.
3. `Running for: <duration>` - elapsed time since verified running, or
   `Not running`.
4. `Next action: <action>` - the armed action and remaining time, or `none`.
5. `Ownership: <owner>` - `True™ Tick`, `Released`, `External timing`, or
   `Uncertain`.

## Multi-monitor menu anchoring

Menu placement uses the actual notification icon rectangle when Windows can
report it. `Shell_NotifyIconGetRect` returns the icon bounds, and the menu
anchors at the icon top-left corner with bottom and left alignment, so the
popup grows upward and rightward from the icon regardless of which monitor
or taskbar edge hosts it.

If the icon rectangle is unavailable, the menu falls back to the cursor
position with alignment flags chosen for the detected taskbar edge, so the
popup still grows into the screen on all four taskbar orientations. The
anchor point is clamped inside the work area of the monitor containing the
cursor, resolved with `MonitorFromPoint` and `GetMonitorInfoW`. When the
per-monitor query fails, the primary work area from `GetSystemMetrics` is
used as the bound. This keeps the menu on the same screen as the click on
multi-monitor setups instead of drifting back to the primary display.

## CLI surface

`true-tick-cli` is the authorized command line control surface for a running
tray instance. It speaks to the tray over the named pipe using the session
token stored under the application data root.

```
true-tick-cli [--root <path>] <subcommand> [options]
```

`--root <path>` selects the application data root that holds the `Data`
directory. It defaults to the directory containing the executable, which sits
beside `Data` in a slot layout.

### Subcommands

- `status` - query tray status and print the reported fields.
- `status --json` - print the same status as a JSON object. Numeric values
  are emitted unquoted, unknown values are omitted.
- `start [--interval <hns>]` - acquire the timer. The optional interval is in
  100-nanosecond units. Omit it for automatic selection.
- `stop` - release the timer.
- `schedule <start-in|stop-in|pause> <seconds>` - arm a timed start, stop, or
  pause action. Seconds must be within [10, 86400]. On success the CLI prints
  the assigned action id.
- `cancel [action-id]` - cancel a pending scheduled action. The optional id
  is encoded as four little-endian bytes to target one action. Without an id,
  every pending action is cleared.
- `logs [--tail N] [--date YYYY-MM-DD]` - print the last N lines of the daily
  CSV log. Defaults to N=20 and the current UTC date. Reads the file
  directly, so the tray does not need to be running.
- `daemon` - launch the headless background engine. Prints
  `daemon already running` and exits quietly when an instance is up.
  Otherwise it spawns the engine detached and waits up to 10 seconds for the
  session token.
- `help` - print usage text.
- `version` - print the CLI version.

### Exit codes

- `0` - success.
- `1` - command, usage, parse, or protocol error. A missing or unreadable log
  file for `logs` also reports exit code 1.
- `2` - tray unreachable. The named pipe cannot be opened or the session
  token file cannot be read, meaning the application is not running.

## Daemon behavior and the output-capture caveat

`daemon` resolves the engine beside the CLI executable, preferring a sibling
`true-tick.exe` and falling back to the slot loader `Launcher.exe`. It clears
any stale session token and server pid marker left by a crashed engine,
spawns the target detached with null stdin, stdout, and stderr handles, then
polls for the token file for up to 10 seconds and verifies the serving
process id before reporting success.

Known caveat: because the engine is spawned as a detached child, a caller
that captures the CLI output can inherit the capture pipe into the engine.
The CLI process exits, but the caller can block reading its capture pipe
until the pipe closes. Python `subprocess.run(..., capture_output=True)` and
shell command substitution are the usual cases. Interactive use and scripts
that let output flow to a terminal are unaffected. The workaround is to
invoke `daemon` without capturing its output, or to redirect its output to a
file instead of a pipe. The engine itself starts correctly in all cases.

## Alternate visual view

Every production source file in the repository has a matching Mermaid diagram
under `docs/diagrams/`. The diagram for `apps/true-tick/src/tray/menu.rs`
lives at `docs/diagrams/apps/true-tick/src/tray/menu.md`, and the same path
mirror convention applies throughout. Use the diagrams when you want the
control flow view, and this guide when you want the user-facing behavior.
