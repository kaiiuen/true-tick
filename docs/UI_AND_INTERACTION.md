# User Interface, Shell Integration, and Interaction Contract

## 1. Notification Area Hierarchy
True Tick provides a non-intrusive, native Windows notification area interface. The context menu follows a strict, single-surface hierarchy:

```
True™ Tick v<version>          [Clickable package identity header]
-----------------------------
Start                         [Direct manual timing acquisition]
Stop                          [Guarded timing release]
Schedule >                    [Duration scheduling and pause controls]
Settings >                    [Atomic configuration toggles]
Status >                      [Read-only telemetry rows]
-----------------------------
Logs                          [Opens diagnostic report window]
-----------------------------
Quit                          [Guarded exit with release confirmation]
```

## 2. Persistent Settings Submenu
The `Settings >` submenu aggregates configuration controls without cluttering the root menu:
1. `Auto-start: On/Off`: Configures Windows login launch via the CurrentUser Run key.
2. `Auto-time: On/Off`: Controls automatic timer acquisition upon AC power detection.
3. `Auto-resume on AC: On/Off`: Reacquires timing when returning to AC power after a restrictive power state released an active session.

### 2.1 Interaction Persistence Loop
Clicking any toggle inside `Settings >` executes an atomic configuration update to `true-tick.toml` and immediately re-opens the `Settings >` submenu at the original cursor anchor coordinates. The menu remains open until the user explicitly clicks outside or presses Escape, enabling rapid multi-setting adjustments.

## 3. Duration Scheduling and Pause Controls
The `Schedule >` submenu provides bounded timer controls:
- `Start in >`: Schedules future acquisition after a monotonic delay.
- `Stop in >`: Schedules restorative release after a monotonic delay.
- `Pause for >`: Immediately releases timing and suppresses re-acquisition until expiration.
- `Interval presets...`: Opens the native Time Manager dialog window (`TrueTickPresetsClass`) to add, edit, or remove custom preset durations (10 seconds to 24 hours).
- `Cancel scheduled action`: Cancels any pending schedule and disarms coordinator timers.
- `Resume now`: Immediately terminates an active pause and re-evaluates power policy.

Only one scheduled action or pause may exist at any time. Selecting a new choice atomically replaces any prior deadline.

## 4. Status Telemetry Rows
The `Status >` submenu exposes five read-only telemetry rows with hover descriptions via `WM_MENUSELECT`:
1. `State`: Current lifecycle status (e.g. `Running`, `Stopped`, `Blocked (Battery)`).
2. `Timing`: Verified effective resolution with raw units (e.g. `0.4966 ms (4966 HNS)`).
3. `Running for`: Monotonic elapsed time since verified acquisition.
4. `Next action`: Pending scheduled deadline and remaining duration.
5. `Ownership`: Active True Tick ownership vs external system influence.

## 5. Shell Icon and Tooltip Contract
The notification area icon and tooltip strictly reflect live system truth:
- **Green Icon**: Verified running state under True Tick ownership.
- **Red Icon**: Stopped state, policy-blocked state, or unverified error state.
- **Yellow Icon**: Transient transition states (Starting, Stopping, Handoff watcher, Paused, Scheduled).
- **Branded Tooltip**: Formatted as `True™ Tick: <state> · <timing>`, for example `True™ Tick: Running · 0.4966 ms` or `True™ Tick: Blocked (Battery Saver) · 0.9999 ms`.
