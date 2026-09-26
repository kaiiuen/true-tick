//! Independent heartbeat watchdog.
//!
//! A `Heartbeat` records the latest kick timestamp as a monotonic
//! millisecond count relative to process start. A `WatchdogHandle`
//! runs a bounded monitor thread that periodically compares the
//! elapsed monotonic count against the last kick and invokes a stall
//! callback once per stall episode. A recovery between episodes resets
//! the episode state so a later stall invokes the callback again. The
//! monitor exits after `MAX_STALL_EPISODES` invocations or when the
//! shared stop flag is set.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Millisecond interval between heartbeat kicks under normal operation.
///
/// The heartbeat is kicked by a recurring UI thread timer in the tray
/// application, so this cadence sets both the kick interval and the
/// watchdog evaluation interval. A 1000 ms cadence against a 4000 ms
/// threshold keeps a 4x margin while halving periodic wakeups for a
/// low overhead tray application.
pub const HEARTBEAT_INTERVAL_MS: u64 = 1000;

/// Millisecond threshold after which a missing heartbeat is a stall.
///
/// Under normal operation the heartbeat is kicked every 1000 ms. A 4000 ms
/// threshold provides a 4x margin before declaring a stall.
pub const STALL_THRESHOLD_MS: u64 = 4000;

/// Maximum number of stall episodes the monitor reports before exiting.
///
/// Once this many stall callbacks have been invoked the monitor thread
/// stops evaluating and terminates so a persistently broken process
/// cannot spin callbacks indefinitely.
pub const MAX_STALL_EPISODES: u32 = 8;

static PROCESS_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Returns the monotonic millisecond count relative to process start.
fn monotonic_ms() -> u64 {
    let epoch = PROCESS_EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Shared record of the latest heartbeat kick.
///
/// The inner atomic stores the monotonic millisecond count observed at
/// the most recent call to `kick`.
pub struct Heartbeat {
    last_kick_ms: Arc<AtomicU64>,
}

impl Heartbeat {
    /// Creates a heartbeat stamped with the current monotonic count.
    pub fn new() -> Self {
        Self {
            last_kick_ms: Arc::new(AtomicU64::new(monotonic_ms())),
        }
    }

    /// Records the current monotonic millisecond count.
    pub fn kick(&self) {
        self.last_kick_ms.store(monotonic_ms(), Ordering::Relaxed);
    }

    /// Returns the millisecond count recorded by the most recent kick.
    pub fn last_kick_ms(&self) -> u64 {
        self.last_kick_ms.load(Ordering::Relaxed)
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of comparing elapsed time against the last kick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogVerdict {
    /// Elapsed time is within the stall threshold.
    Alive,
    /// Elapsed time exceeded the stall threshold.
    Stalled {
        /// Milliseconds elapsed since the last kick.
        stall_ms: u64,
    },
}

/// Compares `now_ms` against `last_kick_ms` under `stall_threshold_ms`.
///
/// Returns `WatchdogVerdict::Stalled` when the saturating difference
/// exceeds the threshold and `WatchdogVerdict::Alive` otherwise.
pub fn evaluate(now_ms: u64, last_kick_ms: u64, stall_threshold_ms: u64) -> WatchdogVerdict {
    let stall_ms = now_ms.saturating_sub(last_kick_ms);
    if stall_ms > stall_threshold_ms {
        WatchdogVerdict::Stalled { stall_ms }
    } else {
        WatchdogVerdict::Alive
    }
}

/// Configuration for a spawned watchdog monitor.
///
/// The stall response lives entirely in the callback the integrator
/// supplies to `WatchdogHandle::spawn`, so the config carries only
/// detection tuning.
#[derive(Debug, Clone, Copy)]
pub struct WatchdogConfig {
    /// Milliseconds between successive evaluations.
    pub interval_ms: u64,
    /// Milliseconds without a kick that constitute a stall.
    pub stall_threshold_ms: u64,
}

/// Paired stop flag and wakeup channel for the monitor thread.
///
/// The monitor waits on the condvar for the full evaluation interval,
/// so `shutdown` wakes it instantly instead of waiting out the
/// interval. Spurious wakeups are absorbed by looping until either the
/// flag is set or the interval has actually elapsed.
struct ShutdownLatch {
    stop: AtomicBool,
    flag: Mutex<bool>,
    condvar: Condvar,
}

impl ShutdownLatch {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            flag: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    /// Sets the stop flag and wakes the monitor immediately.
    fn request_stop(&self) {
        *self.flag.lock().expect("watchdog flag poisoned") = true;
        self.stop.store(true, Ordering::Release);
        self.condvar.notify_one();
    }

    /// Blocks for `interval` or until `request_stop` fires.
    ///
    /// Returns `true` when a stop was requested, `false` when the
    /// interval elapsed. Spurious wakeups recompute the remaining time
    /// from `Instant` so a full interval is always waited unless a
    /// stop arrives.
    fn wait_interval(&self, interval: Duration) -> bool {
        let deadline = Instant::now() + interval;
        let mut flag = self.flag.lock().expect("watchdog flag poisoned");
        loop {
            if *flag {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline - now;
            let (guard, _timeout) = self
                .condvar
                .wait_timeout(flag, remaining)
                .expect("watchdog flag poisoned");
            flag = guard;
        }
    }

    /// Returns the atomic stop flag for cheap checks outside the wait.
    fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

/// Handle to a running watchdog monitor thread.
pub struct WatchdogHandle {
    latch: Arc<ShutdownLatch>,
    stall_episodes: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
}

impl WatchdogHandle {
    /// Spawns a bounded monitor thread.
    ///
    /// The thread blocks on a condvar for `config.interval_ms` per
    /// cycle, so it wakes once per interval plus once per `shutdown`
    /// call, then evaluates the heartbeat against the monotonic count.
    /// `on_stall` is invoked once on the first evaluation of each stall
    /// episode. An `Alive` verdict resets the episode state so a
    /// subsequent stall invokes the callback again. After
    /// `MAX_STALL_EPISODES` invocations the thread stops monitoring and
    /// exits. The thread performs no heap allocation after spawn.
    pub fn spawn(
        heartbeat: Arc<Heartbeat>,
        config: WatchdogConfig,
        on_stall: Box<dyn FnMut(u64) + Send>,
    ) -> WatchdogHandle {
        let latch = Arc::new(ShutdownLatch::new());
        let stall_episodes = Arc::new(AtomicU32::new(0));
        let thread_latch = Arc::clone(&latch);
        let thread_episodes = Arc::clone(&stall_episodes);
        let interval = Duration::from_millis(config.interval_ms);
        let thread = std::thread::spawn(move || {
            let mut on_stall = on_stall;
            let mut stalled = false;
            loop {
                if thread_latch.is_stopped() {
                    break;
                }
                if thread_latch.wait_interval(interval) {
                    break;
                }
                let now_ms = monotonic_ms();
                match evaluate(now_ms, heartbeat.last_kick_ms(), config.stall_threshold_ms) {
                    WatchdogVerdict::Alive => {
                        stalled = false;
                    }
                    WatchdogVerdict::Stalled { stall_ms } => {
                        if stalled {
                            continue;
                        }
                        stalled = true;
                        let episodes = thread_episodes.load(Ordering::Relaxed);
                        if episodes >= MAX_STALL_EPISODES {
                            break;
                        }
                        thread_episodes.store(episodes + 1, Ordering::Relaxed);
                        on_stall(stall_ms);
                    }
                }
            }
        });
        WatchdogHandle {
            latch,
            stall_episodes,
            thread: Some(thread),
        }
    }

    /// Sets the shared stop flag and wakes the monitor so it exits
    /// promptly without waiting out the current interval.
    pub fn shutdown(&self) {
        self.latch.request_stop();
    }

    /// Returns the number of stall episodes reported so far.
    ///
    /// The count is incremented once per stall callback invocation and
    /// is capped at `MAX_STALL_EPISODES`.
    pub fn stall_episodes(&self) -> u32 {
        self.stall_episodes.load(Ordering::Relaxed)
    }

    /// Returns true when the monitor thread has terminated.
    pub fn is_finished(&self) -> bool {
        match &self.thread {
            Some(thread) => thread.is_finished(),
            None => true,
        }
    }

    /// Blocks until the monitor thread terminates.
    pub fn join(mut self) -> std::thread::Result<()> {
        match self.thread.take() {
            Some(thread) => thread.join(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn wait_until<F: FnMut() -> bool>(mut condition: F, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn constants_maintain_four_fold_stall_margin() {
        const { assert!(STALL_THRESHOLD_MS >= 4 * HEARTBEAT_INTERVAL_MS) }
    }

    #[test]
    fn evaluate_reports_alive_within_threshold() {
        assert_eq!(evaluate(2000, 1000, 1200), WatchdogVerdict::Alive);
        assert_eq!(evaluate(2200, 1000, 1200), WatchdogVerdict::Alive);
    }

    #[test]
    fn evaluate_reports_stalled_beyond_threshold() {
        assert_eq!(
            evaluate(5001, 1000, 4000),
            WatchdogVerdict::Stalled { stall_ms: 4001 }
        );
    }

    #[test]
    fn evaluate_handles_counter_wraparound_via_saturating_sub() {
        assert_eq!(evaluate(10, 5000, 4000), WatchdogVerdict::Alive);
    }

    #[test]
    fn kick_updates_last_kick() {
        let heartbeat = Heartbeat::new();
        let initial = heartbeat.last_kick_ms();
        std::thread::sleep(Duration::from_millis(20));
        heartbeat.kick();
        assert!(heartbeat.last_kick_ms() >= initial + 20);
    }

    #[test]
    fn callback_fires_once_per_stall_episode() {
        let heartbeat = Arc::new(Heartbeat {
            last_kick_ms: Arc::new(AtomicU64::new(0)),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let handle = WatchdogHandle::spawn(
            heartbeat,
            WatchdogConfig {
                interval_ms: 10,
                stall_threshold_ms: 0,
            },
            Box::new(move |stall_ms| {
                assert!(stall_ms > 0);
                calls_clone.fetch_add(1, Ordering::Relaxed);
            }),
        );
        wait_until(
            || calls.load(Ordering::Relaxed) == 1,
            "first stall callback",
        );
        assert_eq!(handle.stall_episodes(), 1);
        handle.shutdown();
        wait_until(|| handle.is_finished(), "watchdog thread exit");
        handle.join().expect("watchdog thread panicked");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn callback_does_not_fire_again_within_same_episode() {
        let heartbeat = Arc::new(Heartbeat {
            last_kick_ms: Arc::new(AtomicU64::new(0)),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let handle = WatchdogHandle::spawn(
            heartbeat,
            WatchdogConfig {
                interval_ms: 10,
                stall_threshold_ms: 0,
            },
            Box::new(move |_stall_ms| {
                calls_clone.fetch_add(1, Ordering::Relaxed);
            }),
        );
        wait_until(
            || calls.load(Ordering::Relaxed) == 1,
            "first stall callback",
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(handle.stall_episodes(), 1);
        assert!(!handle.is_finished());
        handle.shutdown();
        wait_until(|| handle.is_finished(), "watchdog thread exit");
        handle.join().expect("watchdog thread panicked");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn recovery_resets_episode_state_and_allows_new_callback() {
        let heartbeat = Arc::new(Heartbeat {
            last_kick_ms: Arc::new(AtomicU64::new(0)),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let thread_heartbeat = Arc::clone(&heartbeat);
        let handle = WatchdogHandle::spawn(
            heartbeat,
            WatchdogConfig {
                interval_ms: 10,
                stall_threshold_ms: 50,
            },
            Box::new(move |_stall_ms| {
                calls_clone.fetch_add(1, Ordering::Relaxed);
            }),
        );
        wait_until(
            || calls.load(Ordering::Relaxed) == 1,
            "first stall callback",
        );
        thread_heartbeat.kick();
        wait_until(
            || calls.load(Ordering::Relaxed) == 2,
            "second stall callback",
        );
        assert_eq!(handle.stall_episodes(), 2);
        handle.shutdown();
        wait_until(|| handle.is_finished(), "watchdog thread exit");
        handle.join().expect("watchdog thread panicked");
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn episode_cap_stops_monitoring() {
        let heartbeat = Arc::new(Heartbeat {
            last_kick_ms: Arc::new(AtomicU64::new(0)),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let thread_heartbeat = Arc::clone(&heartbeat);
        let handle = WatchdogHandle::spawn(
            heartbeat,
            WatchdogConfig {
                interval_ms: 10,
                stall_threshold_ms: 50,
            },
            Box::new(move |_stall_ms| {
                calls_clone.fetch_add(1, Ordering::Relaxed);
            }),
        );
        for episode in 1..=MAX_STALL_EPISODES {
            let calls_wait = Arc::clone(&calls);
            wait_until(
                || calls_wait.load(Ordering::Relaxed) == episode as usize,
                "stall episode callback",
            );
            thread_heartbeat.kick();
        }
        wait_until(|| handle.is_finished(), "watchdog thread exit");
        assert_eq!(handle.stall_episodes(), MAX_STALL_EPISODES);
        handle.join().expect("watchdog thread panicked");
        assert_eq!(calls.load(Ordering::Relaxed), MAX_STALL_EPISODES as usize);
    }

    #[test]
    fn shutdown_stops_thread_without_callback() {
        let heartbeat = Arc::new(Heartbeat::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let handle = WatchdogHandle::spawn(
            heartbeat,
            WatchdogConfig {
                interval_ms: 20,
                stall_threshold_ms: 60_000,
            },
            Box::new(move |_stall_ms| {
                calls_clone.fetch_add(1, Ordering::Relaxed);
            }),
        );
        handle.shutdown();
        wait_until(|| handle.is_finished(), "watchdog thread exit");
        handle.join().expect("watchdog thread panicked");
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn shutdown_mid_interval_returns_well_before_interval() {
        let heartbeat = Arc::new(Heartbeat::new());
        let handle = WatchdogHandle::spawn(
            heartbeat,
            WatchdogConfig {
                interval_ms: 60_000,
                stall_threshold_ms: 120_000,
            },
            Box::new(|_stall_ms| {}),
        );
        // Give the monitor a moment to enter the condvar wait.
        std::thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        handle.shutdown();
        handle.join().expect("watchdog thread panicked");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "shutdown took {elapsed:?}, expected well under the 60 s interval"
        );
    }
}
