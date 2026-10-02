# tick-ownership lib architecture

Source: `crates/tick-ownership/src/lib.rs`

```mermaid
flowchart TD
    subgraph Lifecycle["Ownership lifecycle"]
        direction TB
        Released["Released"]
        Owned["Owned"]
        Uncertain["Uncertain"]

        Released -->|"Acquire success"| Owned
        Released -->|"Acquire failed PostconditionUnverified"| Uncertain
        Released -->|"Acquire failed other error"| Released
        Owned -->|"Release success"| Released
        Owned -->|"Release error"| Uncertain
        Owned -->|"Idempotent acquire"| Owned
        Uncertain -->|"Release success"| Released
        Uncertain -->|"Acquire blocked or release error"| Uncertain
    end

    subgraph Probe["Settle probe"]
        direction TB
        Start["attempt_startup_kernel_settle_probe(attempts_used)"] --> CheckSnapshot{"Snapshot has effective observation?"}
        CheckSnapshot -->|"No"| NotApp1["NotApplicable"]
        CheckSnapshot -->|"Yes"| CheckOwnership{"ownership == Released?"}
        CheckOwnership -->|"No"| NotApp2["NotApplicable"]
        CheckOwnership -->|"Yes"| CheckThreshold{"effective > KERNEL_SETTLE_HIGH_RESOLUTION_THRESHOLD_HNS?"}
        CheckThreshold -->|"Yes"| ConfirmedAbove["Settled ExternalClientConfirmed"]
        CheckThreshold -->|"No"| CheckBudget{"settle_probe_budget_remaining == 0?"}
        CheckBudget -->|"Yes"| ExhaustedZero["Settled Exhausted"]
        CheckBudget -->|"No"| CallPlatform["platform.attempt_kernel_settle_probe"]
        CallPlatform -->|"Err"| BudgetAfterErr{"budget_remaining == 0?"}
        CallPlatform -->|"Ok probe"| InspectOutcome{"probe.outcome"}
        InspectOutcome -->|"Restored and after > before"| Resolved["Settled Resolved"]
        InspectOutcome -->|"ExternalTiming"| ConfirmedTiming["Settled ExternalClientConfirmed"]
        InspectOutcome -->|"Other outcome"| BudgetAfterOther{"budget_remaining == 0?"}
        BudgetAfterErr -->|"Yes"| ExhaustedErr["Settled Exhausted"]
        BudgetAfterErr -->|"No"| RetryErr["Retry budget_remaining"]
        BudgetAfterOther -->|"Yes"| ExhaustedOther["Settled Exhausted"]
        BudgetAfterOther -->|"No"| RetryOther["Retry budget_remaining"]
    end
```

## Notes

* Startup controller assumes zero prior ownership and starts in Released state
* Controller only records explicit successful requests and never writes guessed global defaults
* Acquire preflights the resolved interval then requests the timer resolution
* If acquire fails with PostconditionUnverified ownership transitions to Uncertain
* Non postcondition acquisition errors keep ownership in Released state
* Release requires active or uncertain ownership and calls platform release
* Successful release transitions state to Released and establishes release boundary
* Release handoff observation queries use the existing boundary without resolving new intervals
* Failed release leaves ownership tracked and marks verification as Unverified
* Settle probe runs only when ownership is Released and effective snapshot exists
* Settle probe budget allows at most three attempts spaced by caller timing
* Coarser resolution after restorative probe proves an orphaned token was dropped
