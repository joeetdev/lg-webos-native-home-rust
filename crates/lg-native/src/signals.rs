//! Cooperative handling of termination signals (SIGTERM and SIGINT) for Home
//! transactions.
//!
//! The handler only sets a flag (async-signal-safe). Transactions consume the
//! flag at their checkpoints — before each recorded side effect, inside child
//! process waits and during sleeps — and unwind through their normal recovery
//! path. Consuming the flag means one signal interrupts the transaction once.
//!
//! Recovery and the restore of the preload policy run under a [`Deferral`]:
//! while one is alive, a signal stays pending instead of interrupting, so a
//! signal can never cancel the steps that return the TV to a usable state.

use crate::error::{Error, Result};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static TERMINATED: AtomicBool = AtomicBool::new(false);
static DEFERRALS: AtomicUsize = AtomicUsize::new(0);

/// Consumes a pending termination request, returning whether one was pending.
/// While a [`Deferral`] is alive, nothing is consumed and `false` is returned.
pub fn take() -> bool {
    take_from(&TERMINATED, &DEFERRALS)
}

fn take_from(flag: &AtomicBool, deferrals: &AtomicUsize) -> bool {
    deferrals.load(Ordering::SeqCst) == 0 && flag.swap(false, Ordering::SeqCst)
}

/// Fails with [`Error::Interrupted`] if a termination request is pending (and
/// consumes it), unless a [`Deferral`] is alive.
pub fn check() -> Result<()> {
    if take() {
        Err(Error::Interrupted)
    } else {
        Ok(())
    }
}

/// While alive, termination requests stay pending instead of interrupting.
#[derive(Debug)]
pub struct Deferral(());

/// Defers termination until the returned guard is dropped.
pub fn defer() -> Deferral {
    DEFERRALS.fetch_add(1, Ordering::SeqCst);
    Deferral(())
}

impl Drop for Deferral {
    fn drop(&mut self) {
        DEFERRALS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Sleeps for `duration`, waking early (with an error) on a termination request.
pub fn sleep(duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;
    loop {
        check()?;
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
    }
}

/// Restores the previous signal dispositions when dropped.
#[derive(Debug)]
pub struct SigtermGuard {
    #[cfg(unix)]
    previous: Vec<(libc::c_int, libc::sigaction)>,
}

/// Signals that request an orderly unwind: service-manager termination and
/// an operator's Ctrl-C.
#[cfg(unix)]
const SIGNALS: [libc::c_int; 2] = [libc::SIGTERM, libc::SIGINT];

#[cfg(unix)]
extern "C" fn on_termination(_signal: libc::c_int) {
    TERMINATED.store(true, Ordering::SeqCst);
}

/// Installs the flag-setting handler for SIGTERM and SIGINT.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn install() -> Result<SigtermGuard> {
    let mut guard = SigtermGuard {
        previous: Vec::with_capacity(SIGNALS.len()),
    };
    for signal in SIGNALS {
        // SAFETY: `sigaction` structures are plain data; zero is a valid
        // initial state. The handler only performs an atomic store, which is
        // async-signal-safe. SA_RESTART is deliberately not set so blocking
        // waits return EINTR promptly.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction =
                on_termination as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, &action, &mut previous) != 0 {
                // `guard` restores whatever was already installed.
                return Err(Error::io(
                    "install termination handler",
                    std::io::Error::last_os_error(),
                ));
            }
            guard.previous.push((signal, previous));
        }
    }
    Ok(guard)
}

#[cfg(unix)]
impl Drop for SigtermGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        for (signal, previous) in &self.previous {
            // SAFETY: restores a disposition saved by `install`.
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}

/// Non-Unix hosts never run device transactions.
#[cfg(not(unix))]
pub fn install() -> Result<SigtermGuard> {
    Ok(SigtermGuard {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_termination_is_consumed_exactly_once_and_deferred_while_held() {
        // Local state: the process-wide flag is shared with concurrently running tests.
        let flag = AtomicBool::new(true);
        let deferrals = AtomicUsize::new(1);
        assert!(!take_from(&flag, &deferrals), "deferred: stays pending");
        assert!(flag.load(Ordering::SeqCst));
        deferrals.store(0, Ordering::SeqCst);
        assert!(take_from(&flag, &deferrals));
        assert!(!take_from(&flag, &deferrals), "consumed exactly once");
        assert!(sleep(Duration::from_millis(1)).is_ok());
    }
}
