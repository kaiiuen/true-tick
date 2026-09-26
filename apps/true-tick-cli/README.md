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
- `cancel` cancel a pending scheduled action
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
