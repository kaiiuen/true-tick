#[cfg(test)]
pub(crate) mod harness {
    use std::path::PathBuf;
    use std::sync::Arc;

    use tick_diagnostics::DiagnosticStore;

    use crate::config;
    use crate::pause::{DurationCoordinator, PresetsManager};
    use crate::tray::controller::App;
    use crate::tray::icon::NotifyIconData;
    use crate::tray::ipc;
    use crate::tray::ipc_queue::IpcCommandQueue;
    use crate::tray_surface::TrayStatus;
    use tick_observation_windows::WindowsObservation;
    use tick_ownership::{TimerController, TimingSnapshot};
    use tick_platform_windows::WindowsTimerPlatform;

    pub(crate) fn test_app(diagnostics: Arc<DiagnosticStore>) -> App {
        let config = config::Config::default();
        App {
            controller: TimerController::new(
                WindowsTimerPlatform::with_diagnostics(diagnostics.clone()),
                config.request_interval,
            ),
            observation: WindowsObservation::default(),
            config,
            tray_status: TrayStatus::Stopped,
            last_block_reason: None,
            operating_tier: tick_policy::OperatingTier::Nominal,
            kernel_rejected_requests: 0,
            unhandled_anomalies: 0,
            pending_anomalies: std::sync::atomic::AtomicU32::new(0),
            pending_anomaly_reasons: std::sync::Mutex::new(Vec::new()),
            last_watchdog_episodes: 0,
            consumed_log_write_failures: 0,
            consumed_dispatch_panics: 0,
            surface_recovery_attempts: 0,
            surface_recovery_due: None,
            surface_recovery_exhausted: false,
            tray_hwnd: std::ptr::null_mut(),
            config_path: PathBuf::new(),
            executable: PathBuf::new(),
            startup_status: String::new(),
            operating_slot_label: String::new(),
            portable_root_label: String::new(),
            taskbar_created_message: 0,
            another_instance_message: 0,
            tray_icon: None,
            timing_snapshot: TimingSnapshot::default(),
            timing_snapshot_valid: false,
            invalid_interval: false,
            external_timing: false,
            desired_intent: tick_core::DesiredIntentQueue::new(),
            diagnostics,
            diagnostic_ui: crate::ui::diagnostic_window::DiagnosticUiState::new(),
            menu_active: false,
            settings_submenu_open: false,
            popup_menus: None,
            popup_refresh_timer_active: false,
            schedule_display_timer_active: false,
            handoff: None,
            menu_help: None,
            menu_help_text: Vec::new(),
            pause: DurationCoordinator::new(),
            presets_manager: PresetsManager::new(),
            presets_window: None,
            presets_listbox: None,
            presets_input: None,
            duration_timer_id: None,
            duration_timer_generation: None,
            scheduled_operation: None,
            running_since: None,
            pending_resume_on_ac: false,
            shutdown_gate: crate::shutdown::ShutdownGate::new(),
            operation: None,
            operation_source: tick_diagnostics::DiagnosticSource::Internal,
            handoff_operation: None,
            last_publication: None,
            log_directory: PathBuf::new(),
            state_directory: PathBuf::new(),
            last_persisted_event_sequence: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
                0,
            )),
            power_debounce_active: false,
            power_debounce_target_state: None,
            heartbeat: Arc::new(tick_watchdog::Heartbeat::new()),
            watchdog: None,
            tracked_interval_hns: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ownership_flag: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            responsiveness: tick_policy::ResponsivenessTracker::new(),
            responsiveness_degraded: false,
            responsiveness_notified: false,
            last_heartbeat_fire: None,
            watchdog_stall_events: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            ipc_server: None,
            ipc_status: Arc::new(ipc::IpcStatusSnapshot::new()),
            ipc_schedule_id: 0,
            ipc_command_queue: Arc::new(IpcCommandQueue::new()),
        }
    }

    pub(crate) fn stub_tray_icon() -> NotifyIconData {
        NotifyIconData {
            cb_size: 0,
            h_wnd: std::ptr::null_mut(),
            u_id: 0,
            u_flags: 0,
            u_callback_message: 0,
            h_icon: std::ptr::null_mut(),
            sz_tip: [0; 128],
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0; 256],
            u_timeout_or_version: 0,
            sz_info_title: [0; 64],
            dw_info_flags: 0,
            guid: [0; 16],
            h_balloon_icon: std::ptr::null_mut(),
        }
    }
}
