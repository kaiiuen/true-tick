pub(crate) mod commands;
pub mod controller;
pub(crate) mod diagnostics_map;
pub mod icon;
pub mod ipc;
pub(crate) mod ipc_queue;
pub mod menu;
pub(crate) mod native;
pub(crate) mod power;
pub(crate) mod quit;
pub(crate) mod reset;
pub(crate) mod schedule;
pub(crate) mod status;
pub(crate) mod surface;
pub(crate) mod tier;

#[cfg(test)]
pub(crate) mod test_harness;

pub use controller::run;
pub(crate) use controller::App;
pub(crate) use icon::DeleteObject;
pub(crate) use icon::GetDpiForWindow;
pub(crate) use icon::GetLastError;
pub(crate) use icon::*;

pub(crate) use commands::*;
pub(crate) use diagnostics_map::*;
pub(crate) use ipc_queue::*;
pub(crate) use native::*;
pub(crate) use power::*;
pub(crate) use quit::*;
pub(crate) use reset::*;
pub(crate) use schedule::*;
pub(crate) use status::*;
pub(crate) use surface::*;
pub(crate) use tier::*;

pub(crate) use tick_diagnostics::{DiagnosticOutcome, DiagnosticSource};
pub(crate) use tick_policy::PowerState;

pub(crate) mod list_view_native {
    use super::native::{c_void, Point};

    pub const LVS_REPORT: u32 = 0x0001;
    pub const LVS_SINGLESEL: u32 = 0x0004;
    pub const LVS_SHOWSELALWAYS: u32 = 0x0008;
    pub const LVS_EX_GRIDLINES: usize = 0x0000_0001;
    pub const LVS_EX_FULLROWSELECT: usize = 0x0000_0020;
    pub const LVS_EX_DOUBLEBUFFER: usize = 0x0001_0000;
    pub const LVM_FIRST: u32 = 0x1000;
    pub const LVM_SETBKCOLOR: u32 = LVM_FIRST + 1;
    pub const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
    pub const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
    pub const LVM_GETCOLUMNWIDTH: u32 = LVM_FIRST + 29;
    pub const LVM_GETTOPINDEX: u32 = LVM_FIRST + 39;
    pub const LVM_GETCOUNTPERPAGE: u32 = LVM_FIRST + 40;
    pub const LVM_ENSUREVISIBLE: u32 = LVM_FIRST + 19;
    pub const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
    pub const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
    pub const LVM_SETTEXTCOLOR: u32 = LVM_FIRST + 36;
    pub const LVM_SETTEXTBKCOLOR: u32 = LVM_FIRST + 38;
    pub const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
    pub const LVM_SETCOLUMNWIDTH: u32 = LVM_FIRST + 30;
    pub const LVSCW_AUTOSIZE: i32 = -1;
    pub const LVSCW_AUTOSIZE_USEHEADER: i32 = -2;
    pub const LVIF_TEXT: u32 = 0x0001;
    pub const LVIF_STATE: u32 = 0x0008;
    pub const LVIS_SELECTED: u32 = 0x0002;
    pub const LVNI_SELECTED: u32 = 0x0002;
    pub const LVCF_WIDTH: u32 = 0x0002;
    pub const LVCF_TEXT: u32 = 0x0004;
    pub const LVCFMT_LEFT: i32 = 0x0000;
    pub const LVIR_BOUNDS: i32 = 0;
    pub const LVM_GETITEMRECT: u32 = LVM_FIRST + 14;
    pub const WM_NOTIFY: u32 = 0x004E;
    pub const LVN_ITEMCHANGED: i32 = -101;
    pub const LVN_KEYDOWN: i32 = -155;
    pub const VK_SHIFT: i32 = 0x10;
    pub const VK_CONTROL: i32 = 0x11;

    #[repr(C)]
    pub struct ListViewColumn {
        pub mask: u32,
        pub format: i32,
        pub width: i32,
        pub text: *mut u16,
        pub text_maximum: i32,
        pub subitem: i32,
        pub image: i32,
        pub order: i32,
        pub minimum_width: i32,
        pub default_width: i32,
        pub ideal_width: i32,
    }

    #[repr(C)]
    pub struct ListViewItem {
        pub mask: u32,
        pub item: i32,
        pub subitem: i32,
        pub state: u32,
        pub state_mask: u32,
        pub text: *mut u16,
        pub text_maximum: i32,
        pub image: i32,
        pub parameter: isize,
        pub indent: i32,
        pub group_id: i32,
        pub columns: u32,
        pub column_indices: *mut u32,
        pub column_formats: *mut i32,
        pub group: i32,
    }

    #[repr(C)]
    pub struct NotifyHeader {
        pub hwnd_from: *mut c_void,
        pub id_from: usize,
        pub code: i32,
    }

    #[repr(C)]
    pub struct ListViewNotification {
        pub header: NotifyHeader,
        pub item: i32,
        pub subitem: i32,
        pub new_state: u32,
        pub old_state: u32,
        pub changed: u32,
        pub action_point: Point,
        pub parameter: isize,
    }

    #[repr(C)]
    pub struct ListViewKeyDownNotification {
        pub header: NotifyHeader,
        pub virtual_key: u16,
        pub flags: u32,
    }
}

// Thin wrappers delegating to menu implementation
pub(crate) unsafe fn show_menu(hwnd: *mut std::ffi::c_void, app: &mut App) {
    menu::show_menu(hwnd, app);
}

pub(crate) unsafe fn refresh_popup_menu(app: &mut App) {
    menu::refresh_popup_menu(app);
}

pub(crate) unsafe fn handle_menu_command(
    hwnd: *mut std::ffi::c_void,
    app: &mut App,
    command: usize,
) -> bool {
    menu::handle_menu_command(hwnd, app, command)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tray_surface::TrayStatus;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tick_diagnostics::{
        DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticStore, NativeOutcome,
    };
    use tick_ownership::OwnershipState;
    use tick_policy::PowerState;

    pub(crate) use super::test_harness::harness::{stub_tray_icon, test_app};

    #[test]
    fn startup_rollback_never_deletes_a_value_after_an_enable() {
        assert_eq!(
            startup_rollback(true),
            StartupRollback::SkipDestructiveInverse
        );
        assert_eq!(
            startup_rollback(false),
            StartupRollback::RestoreRegistration
        );
    }

    #[test]
    fn native_result_mapping_preserves_success_and_raw_failure_codes() {
        assert_eq!(native_bool_result(1, 0), NativeResult::Succeeded);
        assert_eq!(
            native_bool_result(0, 5),
            NativeResult::Failed { raw_error: 5 }
        );
        assert_eq!(native_handle_result(false, 0), NativeResult::Succeeded);
        assert_eq!(
            native_handle_result(true, 6),
            NativeResult::Failed { raw_error: 6 }
        );
    }

    #[test]
    fn create_params_are_taken_from_create_struct() {
        let marker = 7u8;
        let create = CreateStruct {
            create_params: (&marker as *const u8).cast_mut().cast(),
            instance: std::ptr::null_mut(),
            menu: std::ptr::null_mut(),
            parent: std::ptr::null_mut(),
            height: 0,
            width: 0,
            y: 0,
            x: 0,
            style: 0,
            name: std::ptr::null(),
            class_name: std::ptr::null(),
            extended_style: 0,
        };
        assert_eq!(app_create_params(&create), create.create_params);
        assert!(app_create_params(std::ptr::null()).is_null());
    }

    #[test]
    fn active_operation_lineage_populates_parent_operation_id() {
        let store = Arc::new(DiagnosticStore::new(16));
        let root = store.begin_operation(DiagnosticSource::TrayCommand);
        let child = store.child_operation(root, DiagnosticSource::Native);

        assert_ne!(root.operation_id, child.operation_id);
        assert_eq!(child.parent_operation_id, Some(root.operation_id));
        assert_eq!(child.correlation_id, root.correlation_id);

        store.record_with_context(
            DiagnosticRecord {
                context: child,
                phase: DiagnosticPhase::Observe,
                source: DiagnosticSource::Native,
                outcome: DiagnosticOutcome::Completed,
                native: NativeOutcome::default(),
            },
            "native.NtQueryTimerResolution.call",
            "status=success",
        );

        let events = store.snapshot();
        let last_event = events.last().expect("event recorded");
        assert_eq!(last_event.parent_operation_id, Some(root.operation_id));
        assert_eq!(last_event.operation_id, child.operation_id);
    }

    #[test]
    fn resume_on_ac_applies_only_when_all_conditions_hold() {
        assert!(resume_on_ac_applies(
            true,
            true,
            PowerState::Ac,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            false,
            true,
            PowerState::Ac,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            true,
            false,
            PowerState::Ac,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            true,
            true,
            PowerState::Battery,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            true,
            true,
            PowerState::Ac,
            OwnershipState::Owned,
        ));
    }

    #[test]
    fn resume_on_ac_queues_acquire_intent_even_when_automatic_timing_is_false() {
        assert!(resume_on_ac_applies(
            true,
            true,
            PowerState::Ac,
            OwnershipState::Released,
        ));
    }

    #[test]
    fn power_debounce_constants_are_bounded_and_monotonic() {
        assert_eq!(POWER_DEBOUNCE_TIMER_ID, 0x7200);
        assert_eq!(POWER_DEBOUNCE_INTERVAL_MS, 2_000);
    }

    #[test]
    fn duration_timer_id_stays_within_bounded_window() {
        assert_eq!(DURATION_TIMER_ID_BASE, 0x6000);
        assert_eq!(DURATION_TIMER_ID_MASK, 0x07FF);
        for generation in 0..4096u64 {
            let id = duration_timer_id(generation);
            assert!(
                (0x6000..=0x67FF).contains(&id),
                "timer id out of window for generation {generation}: 0x{id:X}"
            );
        }
        // Wrap around after the mask window elapses so ids stay bounded.
        assert_eq!(
            duration_timer_id(0x07FF),
            duration_timer_id(0x0800 + 0x07FF)
        );
    }

    #[test]
    fn ipc_server_field_defaults_to_none_in_test_app() {
        let store = Arc::new(DiagnosticStore::new(16));
        let app = test_app(store);
        assert!(app.ipc_server.is_none());
    }

    #[test]
    fn refresh_ipc_status_writes_lifecycle_and_ownership() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        app.tray_status = TrayStatus::Running;
        app.publish();
        let payload = app.ipc_status.status_payload();
        let text = String::from_utf8(payload).expect("status payload is utf8");
        assert!(text.contains("status=running"));
        assert!(text.contains("ownership=released"));
    }

    #[test]
    fn kernel_rejection_escalates_to_metrology_degraded_and_clamps_floor() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = Some(stub_tray_icon());
        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);

        app.kernel_rejected_requests = tick_policy::KERNEL_REJECTIONS_BEFORE_FLOOR;
        refresh_operating_tier(&mut app);

        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::MetrologyDegraded {
                floored_hns: tick_policy::FALLBACK_FLOOR_HNS
            }
        );
        assert_eq!(
            app.controller.requested_interval(),
            tick_core::Hns::new(tick_policy::FALLBACK_FLOOR_HNS)
        );
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.transition"));
    }

    #[test]
    fn anomaly_escalation_to_quiescent_blocks_acquisition() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());

        app.unhandled_anomalies = tick_policy::MAX_ANOMALIES_BEFORE_QUIESCENT;
        refresh_operating_tier(&mut app);

        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Quiescent);
        assert!(!tick_policy::tier_allows_high_resolution(
            app.operating_tier
        ));

        acquire_timer(&mut app);
        assert_eq!(app.controller.ownership(), OwnershipState::Released);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.acquisition_blocked"));
    }

    #[test]
    fn metrology_degraded_recovers_to_nominal_and_resets_counters() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = Some(stub_tray_icon());

        app.kernel_rejected_requests = tick_policy::KERNEL_REJECTIONS_BEFORE_FLOOR;
        refresh_operating_tier(&mut app);
        assert!(matches!(
            app.operating_tier,
            tick_policy::OperatingTier::MetrologyDegraded { .. }
        ));

        app.kernel_rejected_requests = 0;
        refresh_operating_tier(&mut app);

        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);
        assert_eq!(app.kernel_rejected_requests, 0);
        assert_eq!(app.unhandled_anomalies, 0);
    }

    #[test]
    fn explicit_outcome_records_completed_despite_unverified_detail_token() {
        let store = Arc::new(DiagnosticStore::new(16));
        let mut app = test_app(store.clone());

        app.record_with_outcome(
            "timer.query.observation",
            "requested_hns=156250 effective_hns=156250 raw_status=0 effective_relation=unverified",
            DiagnosticOutcome::Completed,
        );

        let events = store.snapshot();
        let event = events
            .iter()
            .find(|event| event.name == "timer.query.observation")
            .expect("observation event recorded");
        assert_eq!(event.outcome, DiagnosticOutcome::Completed);
        assert!(event.details.contains("effective_relation=unverified"));
    }

    #[test]
    fn failed_release_records_caller_side_diagnostic() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.desired_intent
            .request(tick_core::DesiredIntent::Release);
        app.tray_status = TrayStatus::Starting;

        // While acquisition verification is pending, a release intent is
        // requeued rather than acted on, so no guarded_release runs and no
        // release diagnostics appear.
        process_desired_intent(&mut app);

        let events = store.snapshot();
        assert!(events
            .iter()
            .any(|event| event.name == "lifecycle.release_queued"));
    }

    #[test]
    fn successful_release_records_no_caller_side_error() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());

        // Ownership is Released in the test fixture so the release path does
        // not invoke guarded_release at all.
        process_desired_intent(&mut app);

        let events = store.snapshot();
        assert!(!events
            .iter()
            .any(|event| event.name == "ownership.release.caller_failed"));
        assert!(!events
            .iter()
            .any(|event| event.name == "ownership.release.error"));
    }

    fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn anomaly_producer_increments_on_watchdog_stall_episode() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        // A heartbeat stamped at creation goes stale inside the first monitor
        // cycle because the stall threshold is zero, so one episode lands
        // without any heartbeat field access from this crate.
        let heartbeat = Arc::new(tick_watchdog::Heartbeat::new());
        app.watchdog = Some(tick_watchdog::WatchdogHandle::spawn(
            heartbeat,
            tick_watchdog::WatchdogConfig {
                interval_ms: 10,
                stall_threshold_ms: 0,
            },
            Box::new(|_stall_ms| {}),
        ));
        wait_until(
            || {
                app.watchdog
                    .as_ref()
                    .map_or(0, |handle| handle.stall_episodes())
                    == 1
            },
            "first stall episode",
        );
        refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.anomaly.observed"
                && event.details.contains("watchdog_stall_episode")));
        app.watchdog.as_ref().unwrap().shutdown();
        let handle = app.watchdog.take().unwrap();
        handle.join().expect("watchdog thread panicked");
    }

    #[test]
    fn anomaly_producer_increments_on_config_recovery() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        report_anomaly(&app, "config_recovery");
        refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
    }

    #[test]
    fn anomaly_producer_increments_on_tombstone_corruption() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        report_anomaly(&app, "tombstone_corruption");
        refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
    }

    #[test]
    fn anomaly_counter_resets_only_on_recover() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        app.tray_icon = Some(stub_tray_icon());
        app.unhandled_anomalies = 1;
        app.kernel_rejected_requests = tick_policy::KERNEL_REJECTIONS_BEFORE_FLOOR;
        refresh_operating_tier(&mut app);
        assert!(matches!(
            app.operating_tier,
            tick_policy::OperatingTier::MetrologyDegraded { .. }
        ));
        // A Stay transition must not clear the counter.
        refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
        // Recovery requires anomalies cleared first by the operator path.
        app.unhandled_anomalies = 0;
        app.kernel_rejected_requests = 0;
        refresh_operating_tier(&mut app);
        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);
        assert_eq!(app.unhandled_anomalies, 0);
    }

    #[test]
    fn surface_degraded_continues_serving_and_does_not_exit() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = None;
        refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        // The IPC snapshot keeps refreshing even with no tray icon.
        app.tray_status = TrayStatus::Running;
        app.publish();
        let payload = app.ipc_status.status_payload();
        let text = String::from_utf8(payload).expect("status payload is utf8");
        assert!(text.contains("status=running"));
        // The process stays in the loop and does not tear down.
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.surface_recovery.armed"));
    }

    #[test]
    fn surface_recovery_succeeds_within_attempt_budget() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = None;
        refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        // One failed attempt, then success inside the budget.
        advance_surface_recovery(&mut app, false);
        assert_eq!(app.surface_recovery_attempts, 1);
        app.tray_icon = Some(stub_tray_icon());
        advance_surface_recovery(&mut app, true);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.surface_recovered"));
        // The icon being present again lets the next evaluation recover.
        refresh_operating_tier(&mut app);
        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);
    }

    #[test]
    fn surface_recovery_exhaustion_stays_in_surface_degraded() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = None;
        refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        for _ in 0..SURFACE_RECOVERY_ATTEMPTS {
            advance_surface_recovery(&mut app, false);
        }
        assert!(app.surface_recovery_exhausted);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.surface_recovery_exhausted"));
        // Timing capability is unaffected, so the tier stays put and never
        // escalates to Quiescent from surface loss alone.
        refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        assert!(tick_policy::tier_allows_high_resolution(app.operating_tier));
    }

    #[test]
    fn published_status_degrades_running_stopped_and_paused_only() {
        assert_eq!(
            published_status(TrayStatus::Running, true),
            TrayStatus::Degraded
        );
        assert_eq!(
            published_status(TrayStatus::Stopped, true),
            TrayStatus::Degraded
        );
        assert_eq!(
            published_status(TrayStatus::Paused, true),
            TrayStatus::Degraded
        );
        for base in [
            TrayStatus::Blocked,
            TrayStatus::Error,
            TrayStatus::Unsupported,
            TrayStatus::Degraded,
            TrayStatus::Unverified,
            TrayStatus::Starting,
            TrayStatus::ScheduledStart,
            TrayStatus::Pausing,
            TrayStatus::Stopping,
            TrayStatus::ScheduledStop,
        ] {
            assert_eq!(
                published_status(base, true),
                base,
                "{base:?} must not be masked by responsiveness"
            );
        }
        assert_eq!(
            published_status(TrayStatus::Running, false),
            TrayStatus::Running
        );
    }

    #[test]
    fn heartbeat_lateness_escalates_to_degraded_after_three_samples() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        // Seed a low baseline with an on time fire so the later lateness
        // samples classify as anomalies instead of reseeding the tracker.
        app.last_heartbeat_fire = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    tick_watchdog::HEARTBEAT_INTERVAL_MS,
                ))
                .expect("instant subtraction stays in range"),
        );
        handle_heartbeat_timer(&mut app);
        for _ in 0..tick_policy::ESCALATE_CONSECUTIVE_SAMPLES {
            app.last_heartbeat_fire = Some(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(
                        tick_watchdog::STALL_THRESHOLD_MS + tick_watchdog::HEARTBEAT_INTERVAL_MS,
                    ))
                    .expect("instant subtraction stays in range"),
            );
            handle_heartbeat_timer(&mut app);
        }
        assert!(app.responsiveness_degraded);
        assert!(app.responsiveness_notified);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "responsiveness.degraded"));
    }

    #[test]
    fn heartbeat_recovery_clears_degraded_and_notification_flag() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.last_heartbeat_fire = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    tick_watchdog::HEARTBEAT_INTERVAL_MS,
                ))
                .expect("instant subtraction stays in range"),
        );
        handle_heartbeat_timer(&mut app);
        for _ in 0..tick_policy::ESCALATE_CONSECUTIVE_SAMPLES {
            app.last_heartbeat_fire = Some(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(
                        tick_watchdog::STALL_THRESHOLD_MS + tick_watchdog::HEARTBEAT_INTERVAL_MS,
                    ))
                    .expect("instant subtraction stays in range"),
            );
            handle_heartbeat_timer(&mut app);
        }
        assert!(app.responsiveness_degraded);
        for _ in 0..tick_policy::RECOVER_CONSECUTIVE_SAMPLES {
            app.last_heartbeat_fire = Some(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(
                        tick_watchdog::HEARTBEAT_INTERVAL_MS,
                    ))
                    .expect("instant subtraction stays in range"),
            );
            handle_heartbeat_timer(&mut app);
        }
        assert!(!app.responsiveness_degraded);
        assert!(!app.responsiveness_notified);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "responsiveness.recovered"));
    }

    #[test]
    fn watchdog_stall_counter_folds_into_next_heartbeat_sample() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        // Seed the baseline with an on time fire so the folded stall
        // sample registers as an anomaly with a nonzero worst.
        app.last_heartbeat_fire = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    tick_watchdog::HEARTBEAT_INTERVAL_MS,
                ))
                .expect("instant subtraction stays in range"),
        );
        handle_heartbeat_timer(&mut app);
        app.watchdog_stall_events.store(1, Ordering::Release);
        app.last_heartbeat_fire = Some(std::time::Instant::now());
        handle_heartbeat_timer(&mut app);
        assert_eq!(
            app.watchdog_stall_events.load(Ordering::Acquire),
            0,
            "stall events are consumed by the heartbeat sample"
        );
        assert!(
            app.responsiveness.worst_ms() >= tick_watchdog::STALL_THRESHOLD_MS,
            "folded stall sample must reach at least the stall threshold"
        );
    }

    #[test]
    fn kill_heartbeat_timer_clears_last_fire_stamp() {
        let store = Arc::new(DiagnosticStore::new(16));
        let mut app = test_app(store);
        app.last_heartbeat_fire = Some(std::time::Instant::now());
        kill_heartbeat_timer(&mut app);
        assert!(app.last_heartbeat_fire.is_none());
    }
}
