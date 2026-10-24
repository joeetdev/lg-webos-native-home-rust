//! Command flow around the transaction: lock, boot claim, readiness, output.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lg_native::home::model::HOME;
use lg_native::home::{rollback_releasing_gate, run, Action};
use lg_native::json::Value;
use lg_native::{json_object, Error};
use support::FakeOps;

fn run_with(action: Action, ops: &mut FakeOps) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run::<FakeOps, _>(action, || Ok(&mut *ops), &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn boot_apply_is_once_per_boot_and_disabled_or_unready_runs_change_nothing() {
    for scenario in ["disabled", "already-run", "not-ready", "ready"] {
        let mut ops = FakeOps::new();
        ops.disabled = scenario == "disabled";
        ops.claim = scenario != "already-run";
        ops.not_ready = scenario == "not-ready";
        let (code, out, err) = run_with(
            Action::Apply {
                boot: true,
                staging: false,
            },
            &mut ops,
        );
        assert_eq!(
            code,
            if scenario == "not-ready" { 1 } else { 0 },
            "{scenario}: {err}"
        );
        let applied = ops.actions().contains(&"bind".to_owned());
        assert_eq!(applied, scenario == "ready", "{scenario}");
        assert_eq!((ops.locks, ops.unlocks), (1, 1), "{scenario}");
        match scenario {
            "disabled" => assert_eq!(out, "Home overrides disabled; no changes\n"),
            "already-run" => assert_eq!(out, "Home boot attempt already recorded; no retry\n"),
            "not-ready" => {
                assert!(err.starts_with("Native Home failed (inspect journal before retry): busy"))
            }
            _ => assert_eq!(out, "applied\n"),
        }
        if scenario == "ready" || scenario == "not-ready" {
            assert_eq!(
                ops.budgets,
                vec![45.0],
                "boot uses the bounded 45 s startup budget"
            );
        }
    }
    let mut ops = FakeOps::new();
    run_with(
        Action::Apply {
            boot: false,
            staging: false,
        },
        &mut ops,
    );
    assert_eq!(ops.budgets, vec![8.0]);
}

#[test]
fn rollback_dispatch_is_manifest_independent_and_background() {
    let mut ops = FakeOps::new();
    ops.manifest = None;
    let (code, out, err) = run_with(Action::Rollback { background: true }, &mut ops);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "rolled-back\n");
    assert!(ops.disabled, "persistent disable is written first");
    assert_eq!(
        ops.values("launch"),
        vec![json_object! {"id" => HOME, "preload" => "full"}]
    );
    assert!(!ops.actions().contains(&"load_manifest".to_owned()));
    assert_eq!(ops.budgets, vec![8.0]);
}

#[test]
fn rollback_releases_the_gate_before_touching_home() {
    let mut ops = FakeOps::new();
    let mut created = false;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = rollback_releasing_gate::<FakeOps, _>(
        || Err(Error::Policy("Gate removal readback mismatch".into())),
        false,
        || {
            created = true;
            Ok(&mut ops)
        },
        &mut out,
        &mut err,
    );
    assert_eq!(code, 1);
    assert!(
        !created,
        "Home must not be touched while the gate may still hold it"
    );
    assert!(String::from_utf8_lossy(&err).contains("gate release before rollback"));

    let order = std::cell::RefCell::new(Vec::new());
    let mut ops = FakeOps::new();
    let code = rollback_releasing_gate::<FakeOps, _>(
        || {
            order.borrow_mut().push("gate");
            Ok(())
        },
        false,
        || {
            order.borrow_mut().push("home");
            Ok(&mut ops)
        },
        &mut out,
        &mut err,
    );
    assert_eq!(code, 0);
    assert_eq!(*order.borrow(), ["gate", "home"]);
}

#[test]
fn factory_failure_is_reported_without_side_effects() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run::<FakeOps, FakeOps>(
        Action::Apply {
            boot: true,
            staging: false,
        },
        || {
            Err(Error::Policy(
                "Requires root on Linux; not a host installer".into(),
            ))
        },
        &mut out,
        &mut err,
    );
    assert_eq!(code, 1);
    assert_eq!(
        String::from_utf8_lossy(&err),
        "Native Home failed (inspect journal before retry): Requires root on Linux; not a host installer\n"
    );
    assert!(out.is_empty());
}

#[test]
fn boot_hook_is_bounded_and_matches_the_installer_copy() {
    let hook = include_str!("../../../runtime/55-lg-native");
    assert_eq!(hook, lg_native::gate::install::SAFE_HOOK);
    assert!(hook.starts_with("#!/bin/sh\n"));
    assert!(hook.contains("home apply --boot"));
    assert!(hook.contains("gate recover-if-unready"));
    assert!(hook.contains("gate disable"));
    assert!(!hook.contains("while"));
    assert!(!hook.replace("2>&1", "").contains('&'), "no backgrounding");
    assert!(!hook.contains("python"));
    let status = std::process::Command::new("sh")
        .args(["-n", "../../runtime/55-lg-native"])
        .status()
        .unwrap();
    assert!(status.success(), "hook must be valid POSIX sh");
}

#[test]
fn binary_help_is_safe_on_any_host_and_device_commands_refuse_off_device() {
    let exe = env!("CARGO_BIN_EXE_lg-native");
    let help = std::process::Command::new(exe)
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("usage: lg-native"));
    let sub = std::process::Command::new(exe)
        .args(["home", "apply", "--help"])
        .output()
        .unwrap();
    assert!(sub.status.success());
    let bad = std::process::Command::new(exe)
        .args(["home", "apply", "--force"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    // Not the installed binary: Home commands refuse before touching anything.
    let refused = std::process::Command::new(exe)
        .args(["home", "apply", "--boot"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("installed binary")
            || String::from_utf8_lossy(&refused.stderr).contains("Requires root")
    );
    // Gate release is the safety net and needs only root. Never run it where
    // a real gate state could exist.
    let root = lg_native::fsx::euid() == 0;
    if !root || !std::path::Path::new("/var/lib/lg-native").exists() {
        let gate = std::process::Command::new(exe)
            .args(["gate", "disable"])
            .output()
            .unwrap();
        let report = lg_native::json::parse_bytes(&gate.stdout).unwrap();
        if root {
            assert_eq!(gate.status.code(), Some(0));
            assert_eq!(report.get("state"), Some(&Value::from("absent")));
        } else {
            assert_eq!(gate.status.code(), Some(1));
            assert!(report.get("error").and_then(Value::as_str).is_some());
        }
    }
    let remote = std::process::Command::new(exe)
        .args(["remote", "check"])
        .output()
        .unwrap();
    assert_eq!(remote.status.code(), Some(1));
    let report = lg_native::json::parse_bytes(&remote.stdout).unwrap();
    assert_eq!(report.get("ok"), Some(&Value::Bool(false)));
}
