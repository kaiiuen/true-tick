# tick-policy lib architecture

Source: `crates/tick-policy/src/lib.rs`

```mermaid
flowchart TD
    IN["Policy Input, enabled, eligible_profile, power, battery_lockout_enabled"] --> EN{"enabled?"}
    EN -- "No" --> GD["Blocked, GloballyDisabled"]
    EN -- "Yes" --> EP{"eligible_profile?"}
    EP -- "No" --> NP["Released, NoEligibleProfile"]
    EP -- "Yes" --> PW{"power"}
    PW -- "Ac" --> RQAC["Requested, EligibleProfile"]
    PW -- "Battery" --> BK1{"battery_lockout_enabled?"}
    BK1 -- "Yes" --> BT["Blocked, BatteryRestricted"]
    BK1 -- "No" --> RQB1["Requested, EligibleProfile"]
    PW -- "BatterySaver" --> BK2{"battery_lockout_enabled?"}
    BK2 -- "Yes" --> BS["Blocked, BatterySaverRestricted"]
    BK2 -- "No" --> RQB2["Requested, EligibleProfile"]
    PW -- "Unknown" --> UN["Blocked, PowerUnknown"]

    subgraph RS["ResponsivenessTracker latency classifier"]
        OB["observe(lat_ms)"] --> SD{"seeded?"}
        SD -- "No" --> SB["baseline = latency, seeded = true"]
        SD -- "Yes" --> TH
        SB --> TH["threshold = max(baseline x 3, 250 ms)"]
        TH --> AN{"latency > threshold?"}
        AN -- "Anomaly" --> W{"worsening at 2x worst?"}
        W -- "Yes" --> DG["Degraded"]
        W -- "No" --> CT{"streak >= 3?"}
        CT -- "Yes" --> DG
        CT -- "No" --> EV["Elevated"]
        DG --> U1["streak += 1, clean = 0, worst = max"]
        EV --> U1
        AN -- "Clean" --> EW["baseline EWMA, weight 7 to 1"]
        EW --> CL{"clean streak >= 5?"}
        CL -- "Yes" --> NR["Normal, worst = 0"]
        CL -- "No" --> ST["Keep state"]
    end

    subgraph TR["Tray reconciliation names, not crate outputs"]
        AQ["Acquire, DesiredIntent"]
        RE["Release, DesiredIntent"]
        RB["ReleaseBlocked, PowerReconciliation"]
        PO["PreserveOwned, PowerReconciliation"]
        PU["PreserveUncertain, PowerReconciliation"]
        SS["ShowStopped, PowerReconciliation"]
    end

    RQAC -.->|"status Requested"| AQ
    NP -.->|"status Released"| RE
    BT -.->|"status Blocked"| RB
    BS -.->|"status Blocked"| RB
    UN -.->|"status Blocked"| RB
```

## Notes

* This crate is pure decision logic only, it never observes platform state
* decide evaluates guards in a fixed order, global disable first
* A disabled policy always returns Blocked with GloballyDisabled
* No eligible profile returns Released with NoEligibleProfile
* Ac power returns Requested with EligibleProfile
* Battery and BatterySaver honor battery_lockout_enabled, enabled gives Blocked and disabled gives Requested
* Unknown power is conservatively Blocked with PowerUnknown
* The raw statuses inside decide are Blocked, Released and Requested
* Acquire, Release, ReleaseBlocked, PreserveOwned, PreserveUncertain and ShowStopped are tray layer names, they are not produced by this crate
* The anomaly threshold is the larger of 3 times the baseline and 250 ms
* One anomalous sample gives Elevated, three consecutive samples or a 2x episode worst gives Degraded
* A clean sample moves the baseline with a 7 to 1 EWMA and five clean samples recover Normal