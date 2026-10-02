//! Coordinator and helper binary for the multi-client ownership harness.

use std::env;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use tick_core::Hns;
use tick_multiclient::{classify_release_outcome, ClientRole, ClientSpec};
use tick_ownership::TimerController;
use tick_platform_windows::WindowsTimerPlatform;

const TICK_REQUEST_HNS: u64 = 5_000;
const FINER_REQUEST_HNS: u64 = 5_000;
const COARSER_REQUEST_HNS: u64 = 10_000;
const MAX_ITERATIONS: usize = 100;

#[cfg(windows)]
fn set_timer_resolution(desired: u32, set: bool) -> i32 {
    use std::os::raw::c_int;
    extern "system" {
        fn NtSetTimerResolution(
            desired_resolution: u32,
            set_resolution: u8,
            current_resolution: *mut u32,
        ) -> c_int;
    }
    let mut current = 0u32;
    unsafe { NtSetTimerResolution(desired, u8::from(set), &mut current) }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let role = parse_role(&args);
    let iterations = parse_iterations(&args);
    let spec = parse_client_spec(&args);
    let helper_role_flag = match spec.role {
        ClientRole::Finer => "--role finer",
        ClientRole::Coarser => "--role coarser",
    };

    if let Some(role) = role {
        run_helper(role);
    } else {
        run_coordinator(helper_role_flag, spec, iterations);
    }
}

fn parse_role(args: &[String]) -> Option<ClientRole> {
    let role_index = args.iter().position(|arg| arg == "--role")?;
    args.get(role_index + 1)
        .and_then(|value| match value.as_str() {
            "finer" => Some(ClientRole::Finer),
            "coarser" => Some(ClientRole::Coarser),
            _ => None,
        })
}

fn parse_iterations(args: &[String]) -> usize {
    let iterations_index = args.iter().position(|arg| arg == "--iterations");
    let parsed = iterations_index
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    parsed.min(MAX_ITERATIONS)
}

fn parse_client_spec(args: &[String]) -> ClientSpec {
    let role = if args.iter().any(|arg| arg == "--finer") {
        ClientRole::Finer
    } else {
        ClientRole::Coarser
    };
    let interval_hns = match role {
        ClientRole::Finer => FINER_REQUEST_HNS,
        ClientRole::Coarser => COARSER_REQUEST_HNS,
    };
    ClientSpec { role, interval_hns }
}

fn run_helper(role: ClientRole) {
    let interval_hns = match role {
        ClientRole::Finer => FINER_REQUEST_HNS,
        ClientRole::Coarser => COARSER_REQUEST_HNS,
    };
    #[cfg(windows)]
    {
        let status = set_timer_resolution(interval_hns as u32, true);
        println!(
            "helper role={:?} interval_hns={} raw_status={}",
            role, interval_hns, status
        );
        loop {
            thread::sleep(Duration::from_millis(1_000));
        }
    }
    #[cfg(not(windows))]
    {
        println!(
            "helper role={:?} interval_hns={} raw_status=unsupported",
            role, interval_hns
        );
        loop {
            thread::sleep(Duration::from_millis(1_000));
        }
    }
}

fn spawn_helper(role_flag: &str) -> Option<Child> {
    let current_exe = env::current_exe().ok()?;
    let mut command = Command::new(current_exe);
    command.args(role_flag.split_whitespace());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.spawn().ok()
}

fn run_coordinator(helper_role_flag: &str, spec: ClientSpec, iterations: usize) {
    for iteration in 0..iterations {
        let mut helper = spawn_helper(helper_role_flag);
        thread::sleep(Duration::from_millis(50));

        let mut controller =
            TimerController::new(WindowsTimerPlatform::default(), Hns::new(TICK_REQUEST_HNS));

        let tick_owned = controller.start().is_ok();
        let tick_released = controller.stop().is_ok();

        let observed = match controller.query() {
            Ok(query) => query.reported_current.value(),
            Err(_) => 0,
        };

        if let Some(mut child) = helper.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        let finer_active = matches!(spec.role, ClientRole::Finer) && tick_released;
        let outcome = classify_release_outcome(
            tick_owned,
            finer_active,
            observed,
            TICK_REQUEST_HNS,
            spec.interval_hns,
        );
        let name = match outcome {
            tick_multiclient::ReleaseOutcome::TickOnlyReleased => "tick_only_released",
            tick_multiclient::ReleaseOutcome::FinerClientRetained => "finer_client_retained",
            tick_multiclient::ReleaseOutcome::UnknownState => "unknown_state",
        };
        println!(
            "iteration={} outcome={} observed_hns={}",
            iteration, name, observed
        );
    }
}
