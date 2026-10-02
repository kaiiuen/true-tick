# true-tick-cli

Command line control surface for a running True Tick tray instance, the
authorized local command line interface over the named pipe protocol.

## Usage

```
true-tick-cli [--root <path>] <subcommand> [options]
```

`--root` selects the application data root holding the `Data` directory. By
default the directory containing this executable is used, which sits beside
`Data` in a slot layout.

## Subcommands

- `status [--json]` query tray status, `--json` prints `{"status": "..."}`
- `start [--interval <hns>]` acquire the timer, interval in 100-nanosecond
  units, omitted means automatic selection
- `stop` release the timer
- `schedule <start-in|stop-in|pause> <seconds>` arm a timed action
- `cancel [action-id]` cancel a pending scheduled action, with an optional
  4 byte little-endian action id to target a specific action, omitted means
  legacy clear-anything behavior
- `daemon` launch the headless background engine. Exits quietly when an
  instance is already running, otherwise spawns `true-tick.exe` (or the
  slot loader `Launcher.exe`) detached and waits for the session token
- `logs [--tail N] [--date YYYY-MM-DD]` print the last N lines of the daily
  CSV log, defaults to N=20 and the current UTC date, reads the file
  directly so the tray does not need to be running
- `help` usage text
- `version` print the version

## Exit codes

- `0` success
- `1` command, usage, parse, or protocol error
- `2` tray cannot be reached or the session token file cannot be read, the
  application is not running

## Notes

The `daemon` subcommand spawns the engine as a detached process, so a caller
that captures CLI output, such as Python `subprocess.run(..., capture_output=True)`
or a shell command substitution, can have its capture pipe inherited by the
engine and block until that pipe closes even though the CLI already exited.
Interactive use and scripts that let output flow to a terminal are unaffected,
and the engine starts correctly in all cases. The workaround is to invoke
`daemon` without capturing its output.
