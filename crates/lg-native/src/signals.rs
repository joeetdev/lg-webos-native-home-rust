//! Cooperative SIGTERM handling for Home transactions.
//!
//! The handler only sets a flag (async-signal-safe). Transactions consume the
//! flag at their checkpoints — before each recorded side effect, inside child
//! process waits and during sleeps — and unwind through their normal recovery
//! path. Consuming the flag means one SIGTERM interrupts the transaction once;
//! the recovery that follows is not itself cancelled unless another SIGTERM
//! arrives.

use crate::error::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static TERMINATED: AtomicBool = AtomicBool::new(false);

/// Consumes a pending SIGTERM, returning whether one was pending.
pub fn take() -> bool {
    TERMINATED.swap(false, Ordering::SeqCst)
}

/// Fails with [`Error::Interrupted`] if a SIGTERM is pending (and consumes it).
pub fn check() -> Result<()> {
    consume(&TERMINATED)
}

fn consume(flag: &AtomicBool) -> Result<()> {
    if flag.swap(false, Ordering::SeqCst) {
        Err(Error::Interrupted)
    } else {
        Ok(())
    }
}

/// Sleeps for `duration`, waking early (with an error) on SIGTERM.
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

/// Restores the previous SIGTERM disposition when dropped.
#[derive(Debug)]
pub struct SigtermGuard {
    #[cfg(unix)]
    previous: libc::sigaction,
}

#[cfg(unix)]
extern "C" fn on_sigterm(_signal: libc::c_int) {
    TERMINATED.store(true, Ordering::SeqCst);
}

/// Installs the flag-setting SIGTERM handler.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn install() -> Result<SigtermGuard> {
    // SAFETY: `sigaction` structures are plain data; zero is a valid initial
    // state. The handler only performs an atomic store, which is
    // async-signal-safe. SA_RESTART is deliberately not set so blocking waits
    // return EINTR promptly.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        let mut previous: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGTERM, &action, &mut previous) != 0 {
            return Err(Error::io(
                "install SIGTERM handler",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(SigtermGuard { previous })
    }
}

#[cfg(unix)]
impl Drop for SigtermGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: restores the disposition saved by `install`.
        unsafe {
            libc::sigaction(libc::SIGTERM, &self.previous, std::ptr::null_mut());
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
    fn a_pending_termination_is_consumed_exactly_once() {
        // A local flag: the process-wide one is shared with concurrently running tests.
        let flag = AtomicBool::new(true);
        assert!(matches!(consume(&flag), Err(Error::Interrupted)));
        assert!(consume(&flag).is_ok());
        assert!(sleep(Duration::from_millis(1)).is_ok());
    }
}
