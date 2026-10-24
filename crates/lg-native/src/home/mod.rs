//! Fail-closed, root-only native Home overrides.
//!
//! Eight exact QML/JS files of `com.webos.app.home` are replaced by read-only
//! bind mounts of immutable, content-addressed copies. The same Home app,
//! LaunchPoints and native actions remain in place; nothing else is touched.

pub mod controller;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod model;

pub use controller::{Controller, HomeOps, HomeReader, Outcome};

use crate::error::{Error, Result};
use std::borrow::BorrowMut;
use std::io::Write;

/// What the readiness wait needs from the device (fakeable in tests).
pub trait ReadinessProbe {
    /// Monotonic clock in seconds.
    fn now(&self) -> f64;
    /// Sleeps (may be cut short by SIGTERM).
    fn sleep(&mut self, seconds: f64) -> Result<()>;
    /// True while CRIU is restoring a checkpoint.
    fn criu_restoring(&self) -> bool;
    /// One LS2 call with a timeout in seconds.
    fn call(&mut self, method: &str, timeout: f64) -> Result<()>;
}

/// Readiness services that must answer before Home may be touched.
const READINESS_METHODS: [&str; 2] = [
    "com.webos.applicationManager/getForegroundAppInfo",
    "com.webos.service.preloadmanager/getPreloadPolicy",
];

/// Waits until CRIU is idle and both services answer, within `budget` seconds
/// (clamped to 0–45). Nothing is mutated while waiting.
pub fn wait_ready(probe: &mut impl ReadinessProbe, budget: f64) -> Result<()> {
    let deadline = probe.now() + budget.clamp(0.0, 45.0);
    let mut last = String::from("CRIU restore active");
    while probe.now() < deadline {
        if !probe.criu_restoring() {
            match all_services_answer(probe, deadline) {
                Ok(()) => return Ok(()),
                Err(Error::Interrupted) => return Err(Error::Interrupted),
                Err(error) => last = error.to_string(),
            }
        }
        let remaining = deadline - probe.now();
        if remaining > 0.0 {
            probe.sleep(remaining.min(0.25))?;
        }
    }
    Err(Error::Timeout(format!(
        "Home readiness budget exhausted: {last}"
    )))
}

fn all_services_answer(probe: &mut impl ReadinessProbe, deadline: f64) -> Result<()> {
    for method in READINESS_METHODS {
        let remaining = deadline - probe.now();
        if remaining <= 0.0 {
            return Err(Error::Timeout("Readiness deadline".into()));
        }
        probe.call(method, remaining.min(1.5))?;
    }
    Ok(())
}

/// The process-level operations around a transaction (lock, boot claim,
/// readiness), separated so the command flow is testable.
pub trait Runtime: HomeOps {
    /// Takes the exclusive transaction lock.
    fn lock(&mut self) -> Result<()>;
    /// Releases the lock.
    fn unlock(&mut self);
    /// Records one attempt per boot; `false` if this boot already tried.
    fn claim_boot(&mut self) -> Result<bool>;
    /// Bounded readiness wait.
    fn wait_ready(&mut self, budget: f64) -> Result<()>;
    /// Verifies the operator's checkpoint backup record before any change.
    fn verify_checkpoint_backup(&mut self) -> Result<()>;
}

#[cfg(target_os = "linux")]
impl Runtime for linux::LinuxOps {
    fn lock(&mut self) -> Result<()> {
        linux::LinuxOps::lock(self)
    }
    fn unlock(&mut self) {
        linux::LinuxOps::unlock(self);
    }
    fn claim_boot(&mut self) -> Result<bool> {
        linux::LinuxOps::claim_boot(self)
    }
    fn wait_ready(&mut self, budget: f64) -> Result<()> {
        linux::LinuxOps::wait_ready(self, budget)
    }
    fn verify_checkpoint_backup(&mut self) -> Result<()> {
        self.trusted_file(model::BACKUP_PATH, None).map(drop)
    }
}

/// Which transaction to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Apply the manifest.
    Apply {
        /// One attempt per boot with the long (45 s) readiness budget.
        boot: bool,
        /// Permit 3–8 targets instead of 6–8.
        staging: bool,
    },
    /// Restore stock Home.
    Rollback {
        /// Always relaunch Home with `preload: full`.
        background: bool,
    },
}

const FAILURE_PREFIX: &str = "Native Home failed (inspect journal before retry): ";

/// `home rollback`: release the presentation gate FIRST, then restore stock
/// Home. An owned gate would keep stock Home transparent forever, so the
/// overrides are never removed while the gate might still hold Home windows.
pub fn rollback_releasing_gate<R: Runtime, B: BorrowMut<R>>(
    release_gate: impl FnOnce() -> Result<()>,
    background: bool,
    factory: impl FnOnce() -> Result<B>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> i32 {
    if let Err(error) = release_gate() {
        let _ = writeln!(
            err,
            "Native Home failed (gate release before rollback): {error}"
        );
        return 1;
    }
    run::<R, B>(Action::Rollback { background }, factory, out, err)
}

/// True when `saved` (the boot marker) already records `boot_id`.
pub fn boot_already_claimed(saved: Option<&crate::json::Value>, boot_id: &str) -> bool {
    saved
        .and_then(|s| s.get("bootId"))
        .and_then(crate::json::Value::as_str)
        == Some(boot_id)
}

/// Runs one Home command: lock, readiness, transaction, unlock. Returns the
/// process exit code. SIGTERM unwinds the transaction through its recovery.
/// `factory` may return the runtime itself or a mutable borrow of it.
pub fn run<R: Runtime, B: BorrowMut<R>>(
    action: Action,
    factory: impl FnOnce() -> Result<B>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> i32 {
    let _sigterm = crate::signals::install();
    let mut holder = match factory() {
        Ok(holder) => holder,
        Err(error) => {
            let _ = writeln!(err, "{FAILURE_PREFIX}{error}");
            return 1;
        }
    };
    let runtime: &mut R = holder.borrow_mut();
    let result = execute(action, runtime);
    runtime.unlock();
    match result {
        Ok(message) => {
            let _ = writeln!(out, "{message}");
            0
        }
        Err(error) => {
            let _ = writeln!(err, "{FAILURE_PREFIX}{error}");
            1
        }
    }
}

fn execute<R: Runtime>(action: Action, runtime: &mut R) -> Result<&'static str> {
    runtime.lock()?;
    match action {
        Action::Apply { boot, staging } => {
            if runtime.is_disabled() {
                return Ok("Home overrides disabled; no changes");
            }
            if boot && !runtime.claim_boot()? {
                return Ok("Home boot attempt already recorded; no retry");
            }
            runtime.wait_ready(if boot { 45.0 } else { 8.0 })?;
            runtime.verify_checkpoint_backup()?;
            Controller::new(runtime).apply(staging).map(Outcome::as_str)
        }
        Action::Rollback { background } => {
            // The persistent disable marker comes first, even if LS2 or CRIU
            // are unavailable.
            runtime.disable()?;
            runtime.wait_ready(8.0)?;
            Controller::new(runtime)
                .rollback(background)
                .map(Outcome::as_str)
        }
    }
}
