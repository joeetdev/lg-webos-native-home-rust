//! Termination signals during Home transactions. Each scenario raises a real
//! signal (handled synchronously by the installed handler), so this file is
//! its own test binary with a single test: the flag is process-wide.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lg_native::home::model::HOME;
use lg_native::home::Controller;
use lg_native::json::Value;
use lg_native::{json_object, signals, Error};
use support::FakeOps;

fn scenario(signal: i32, raise_on: &str) -> (lg_native::Result<lg_native::home::Outcome>, FakeOps) {
    let mut ops = FakeOps::with(false, Some(HOME), None);
    ops.luna_checks_signals = true;
    ops.raise_on = Some(raise_on.to_owned());
    ops.raise_signal = signal;
    let result = Controller::new(&mut ops).apply(false);
    // Nothing may stay pending into the next scenario.
    let _ = signals::take();
    (result, ops)
}

fn recovered_to_stock(ops: &FakeOps) {
    assert!(ops.disabled, "persistent disable marker written");
    assert!(ops.live.is_empty(), "owned mounts removed");
    assert!(!ops.enabled, "original preload policy (false) restored");
    assert_eq!(
        ops.journal_preload().get("pending"),
        Some(&Value::Bool(false))
    );
    let launches = ops.values("launch");
    assert_eq!(
        launches.last(),
        Some(&json_object! {"id" => HOME, "preload" => "full"})
    );
}

#[test]
fn termination_signals_unwind_through_recovery_and_never_skip_the_preload_restore() {
    let _handler = signals::install().unwrap();

    // SIGTERM while Home is being killed: the next checkpoint (the foreground
    // re-check) unwinds, and recovery restores stock Home.
    let (result, ops) = scenario(libc::SIGTERM, "kill");
    assert!(matches!(result, Err(Error::Interrupted)), "{result:?}");
    recovered_to_stock(&ops);

    // Ctrl-C (SIGINT) behaves the same way.
    let (result, ops) = scenario(libc::SIGINT, "kill");
    assert!(matches!(result, Err(Error::Interrupted)), "{result:?}");
    recovered_to_stock(&ops);

    // SIGTERM while the restore intent is being journaled must not cancel the
    // restore's own LS2 calls.
    let (result, ops) = scenario(libc::SIGTERM, "journal:preload-restore");
    assert!(result.is_ok(), "{result:?}");
    assert!(!ops.enabled);
    assert_eq!(
        ops.journal_preload().get("pending"),
        Some(&Value::Bool(false))
    );

    // A second signal during recovery is deferred until recovery has finished.
    let mut ops = FakeOps::with(false, Some(HOME), Some("readonly"));
    ops.luna_checks_signals = true;
    ops.raise_on = Some("journal:disable".into());
    ops.raise_signal = libc::SIGTERM;
    let result = Controller::new(&mut ops).apply(false);
    let _ = signals::take();
    assert!(
        matches!(result, Err(Error::Data(ref m)) if m == "injected readonly"),
        "{result:?}"
    );
    recovered_to_stock(&ops);
}
