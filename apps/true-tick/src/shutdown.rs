#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageLoopExit {
    NormalQuit,
    GetMessageFailed { raw_error: u32 },
}

pub(crate) const fn message_loop_exit(result: i32, raw_error: u32) -> Option<MessageLoopExit> {
    match result {
        0 => Some(MessageLoopExit::NormalQuit),
        -1 => Some(MessageLoopExit::GetMessageFailed { raw_error }),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShutdownDisposition {
    Complete,
    ExitAfterMessageLoopError,
    KeepAliveForRetry,
    ExitWithUnresolvedCleanup,
}

pub(crate) const fn shutdown_disposition(
    message_loop_exit: MessageLoopExit,
    cleanup_verified: bool,
    ui_usable: bool,
) -> ShutdownDisposition {
    if cleanup_verified {
        return match message_loop_exit {
            MessageLoopExit::NormalQuit => ShutdownDisposition::Complete,
            MessageLoopExit::GetMessageFailed { .. } => {
                ShutdownDisposition::ExitAfterMessageLoopError
            }
        };
    }
    if matches!(message_loop_exit, MessageLoopExit::NormalQuit) && ui_usable {
        ShutdownDisposition::KeepAliveForRetry
    } else {
        ShutdownDisposition::ExitWithUnresolvedCleanup
    }
}

#[derive(Debug, Default)]
pub(crate) struct ShutdownGate {
    verified: bool,
    attempts: u32,
}

impl ShutdownGate {
    pub(crate) const fn new() -> Self {
        Self {
            verified: false,
            attempts: 0,
        }
    }

    pub(crate) const fn verified(&self) -> bool {
        self.verified
    }

    pub(crate) const fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(crate) fn attempt<F, E>(&mut self, cleanup: F) -> Result<(), E>
    where
        F: FnOnce() -> Result<(), E>,
    {
        if self.verified {
            return Ok(());
        }
        self.attempts = self.attempts.saturating_add(1);
        match cleanup() {
            Ok(()) => {
                self.verified = true;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_loop_exit_models_normal_quit_and_native_failure_distinctly() {
        assert_eq!(message_loop_exit(0, 0), Some(MessageLoopExit::NormalQuit));
        assert_eq!(
            message_loop_exit(-1, 1234),
            Some(MessageLoopExit::GetMessageFailed { raw_error: 1234 })
        );
        assert_eq!(message_loop_exit(1, 0), None);
    }

    #[test]
    fn successful_cleanup_is_guarded_against_repeated_release() {
        let mut gate = ShutdownGate::new();
        let mut calls = 0;
        assert_eq!(
            gate.attempt(|| {
                calls += 1;
                Ok::<(), ()>(())
            }),
            Ok(())
        );
        assert_eq!(
            gate.attempt(|| {
                calls += 1;
                Ok::<(), ()>(())
            }),
            Ok(())
        );
        assert_eq!(calls, 1);
        assert_eq!(gate.attempts(), 1);
        assert!(gate.verified());
    }

    #[test]
    fn failed_cleanup_remains_retryable_until_verified() {
        let mut gate = ShutdownGate::new();
        let mut calls = 0;
        assert_eq!(
            gate.attempt(|| {
                calls += 1;
                Err::<(), _>("release failed")
            }),
            Err("release failed")
        );
        assert_eq!(
            gate.attempt(|| {
                calls += 1;
                Ok::<(), &str>(())
            }),
            Ok(())
        );
        assert_eq!(calls, 2);
        assert_eq!(gate.attempts(), 2);
        assert!(gate.verified());
    }

    #[test]
    fn unresolved_cleanup_is_kept_alive_only_with_a_usable_normal_loop() {
        assert_eq!(
            shutdown_disposition(MessageLoopExit::NormalQuit, false, true),
            ShutdownDisposition::KeepAliveForRetry
        );
        assert_eq!(
            shutdown_disposition(MessageLoopExit::NormalQuit, false, false),
            ShutdownDisposition::ExitWithUnresolvedCleanup
        );
    }

    #[test]
    fn uncertain_cleanup_after_message_loop_failure_exits_unresolved() {
        assert_eq!(
            shutdown_disposition(
                MessageLoopExit::GetMessageFailed { raw_error: 5 },
                false,
                true
            ),
            ShutdownDisposition::ExitWithUnresolvedCleanup
        );
    }

    #[test]
    fn cleanup_gate_distinguishes_normal_shutdown_from_message_loop_failure() {
        assert_eq!(
            shutdown_disposition(MessageLoopExit::NormalQuit, true, false),
            ShutdownDisposition::Complete
        );
        assert_eq!(
            shutdown_disposition(MessageLoopExit::NormalQuit, true, true),
            ShutdownDisposition::Complete
        );
        assert_eq!(
            shutdown_disposition(
                MessageLoopExit::GetMessageFailed { raw_error: 1 },
                true,
                false
            ),
            ShutdownDisposition::ExitAfterMessageLoopError
        );
        assert_eq!(
            shutdown_disposition(
                MessageLoopExit::GetMessageFailed { raw_error: 1 },
                true,
                true
            ),
            ShutdownDisposition::ExitAfterMessageLoopError
        );
    }

    #[test]
    fn message_loop_failure_never_reports_normal_shutdown_complete() {
        assert_eq!(
            shutdown_disposition(
                MessageLoopExit::GetMessageFailed { raw_error: 5 },
                true,
                true
            ),
            ShutdownDisposition::ExitAfterMessageLoopError
        );
    }
}
