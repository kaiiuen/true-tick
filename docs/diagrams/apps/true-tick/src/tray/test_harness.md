# Tray test_harness App factory

Source path: `true-tick/apps/true-tick/src/tray/test_harness.rs`

```mermaid
flowchart TD
    subgraph TESTAPP["test_app diagnostics"]
        T0["test_app Arc DiagnosticStore"] --> T1["config = Config::default"]
        T1 --> T2["controller = TimerController::new WindowsTimerPlatform::with_diagnostics config.request_interval"]
        T2 --> T3["observation = WindowsObservation::default"]
        T3 --> T4["App struct literal with every field"]
        T4 --> T4A["status fields Stopped, tier Nominal, counters zeroed"]
        T4 --> T4B["handles tray_hwnd null, tray_icon None, hwnd fields None"]
        T4 --> T4C["paths config_path, executable, log and state dirs empty PathBuf"]
        T4 --> T4D["timing snapshot default, valid false, external_timing false"]
        T4 --> T4E["coordinators DesiredIntentQueue, DurationCoordinator, PresetsManager new"]
        T4 --> T4F["diagnostics store plus DiagnosticUiState new"]
        T4 --> T4G["flags menu_active false, timers inactive, handoff None"]
        T4 --> T4H["shutdown_gate new, operation None, handoff_operation None"]
        T4 --> T4I["heartbeat Arc new, watchdog None, stall events AtomicU32 0"]
        T4 --> T4J["tracked_interval_hns and ownership_flag AtomicU64 0"]
        T4 --> T4K["responsiveness tracker new, degraded and notified false"]
        T4 --> T4L["ipc_server None, ipc_status snapshot new, ipc_schedule_id 0, ipc_command_queue new"]
    end

    subgraph STUB["stub_tray_icon"]
        S0["stub_tray_icon"] --> S1["NotifyIconData all zero fields"]
        S1 --> S1A["h_wnd null, h_icon null, h_balloon_icon null"]
        S1 --> S1B["sz_tip 128, sz_info 256, sz_info_title 64, guid 16 zeroed arrays"]
    end
```

## Notes

- The module is `#[cfg(test)]` only, so `test_app` and `stub_tray_icon` never compile into the release binary.
- `test_app` wires the real `WindowsTimerPlatform` with diagnostics rather than a mock, so tests exercise the actual controller path while still capturing diagnostic events through the injected store.
- Every `App` field is initialized explicitly in one literal, so adding a field to `App` without updating the harness is a compile error rather than a silent default.
- `stub_tray_icon` returns an all-zero `NotifyIconData`, which is sufficient for code paths that only read `h_wnd` or store the struct, and deliberately avoids fabricating a live shell registration.
- Atomics (`pending_anomalies`, `watchdog_stall_events`, `tracked_interval_hns`, `ownership_flag`, `last_persisted_event_sequence`) are constructed fresh per call so tests cannot leak counter state into each other.
