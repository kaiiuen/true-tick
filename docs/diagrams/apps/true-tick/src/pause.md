# Pause and Duration Scheduling in pause.rs

Source path: `true-tick/apps/true-tick/src/pause.rs`

```mermaid
flowchart TD
    A["DurationPreset bounds"] --> B{"seconds in 10..86400"}
    B -- "no" --> ERR1["Err preset must be 10 seconds to 24 hours"]
    B -- "yes" --> C["DurationPreset { seconds }"]

    P["DurationPreset.parse natural text"] --> T["trim then ascii lowercase"]
    T --> ALL{"all digits and dots"}
    ALL -- "yes" --> DEC["parse_decimal_micros whole plus fraction"]
    ALL -- "no" --> SEG["scan segments number then unit"]
    SEG --> UN{"unit is s m h or secs mins hours"}
    UN -- "no" --> ERR2["Err expected unit or unknown unit"]
    UN -- "yes" --> ACC["sum segment seconds checked overflow"]
    DEC --> RND["round half up to nearest second add 500000 micros"]
    ACC --> BND{"total in 10..86400"}
    RND --> BND
    BND -- "no" --> ERR3["Err preset must be between 10 seconds and 24 hours"]
    BND -- "yes" --> NEW["DurationPreset::new"]

    NEW --> M["DurationAction Start Stop Pause"]
    M --> S["schedule action sets deadline now plus duration"]
    S --> SA["ScheduledAction action duration deadline generation"]
    SA --> PA{"pause_active matches Pause"}
    PA -- "yes" --> SUPP["acquisition_is_allowed false while paused"]
    PA -- "no" --> RUN["acquisition_is_allowed checks automatic power battery"]
    SA --> EV{"timer_event generation"}
    EV -- "mismatch" --> ST["CoordinatorTimerEvent Stale"]
    EV -- "remaining zero" --> EX["CoordinatorTimerEvent Expired"]
    EV -- "remaining above zero" --> EAR["CoordinatorTimerEvent Early with remaining"]
    EAR --> IV["timer_interval_ms as_millis max 1 min 3600000"]
    RUN --> DISP["remaining display uses saturating_duration_since"]
    DISP --> IV
```

## Notes

- `MIN_PRESET_SECONDS` is 10 and `MAX_PRESET_SECONDS` is 86400, which `DurationPreset::new` enforces on every construction.
- `MAX_DURATION` reuses the preset maximum, so a scheduled deadline is capped at 24 hours via `duration.duration().min(MAX_DURATION)`.
- `DurationPreset::parse` accepts whole numbers, decimal numbers, and natural segments such as "1.5 hours" and "1h 30m", rejecting empty text and unknown units.
- Decimal values are scaled to microseconds with `parse_decimal_micros`, which rejects more than six fraction digits and multiple dots.
- Each natural segment is rounded to whole seconds by adding 500000 microseconds before dividing, a round half up to the nearest second.
- The parsed total must land in the 10 to 86400 second range or parsing fails with the same bounds error as `new`.
- `DurationAction` has three variants, Start, Stop and Pause, each carrying a short label used by menu rows.
- `DurationCoordinator` holds a single `Option<ScheduledAction>` and a monotonic generation that increments on each schedule and on every cancel that clears a live action.
- A `Pause` action is treated as active while the coordinator holds an action whose variant is `Pause`, which drives `pause_active`.
- `acquisition_is_allowed` returns false whenever a pause is active, and otherwise requires automatic mode plus a policy decision of `Requested` across all power states.
- A timer event is `Stale` unless its generation matches both the coordinator generation and the action generation, which prevents an old replacement from firing.
- Remaining display time comes from `deadline.saturating_duration_since(now)`, and the refresh interval clamps that to between 1 millisecond and `MAX_TIMER_INTERVAL_MS` 3600000 milliseconds.
