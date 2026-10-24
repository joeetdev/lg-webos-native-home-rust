//! The real Home controller against the fake device boundary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lg_native::home::model::{HOME, OWNER};
use lg_native::home::{Controller, Outcome};
use lg_native::json::Value;
use lg_native::{json_object, Error};
use support::{h, items, text, FakeOps};

fn background_launch() -> Value {
    json_object! {"id" => HOME, "preload" => "full"}
}

const SIDE_EFFECTS: [&str; 8] = [
    "copy", "bind", "readonly", "unmount", "clear", "kill", "launch", "preload",
];

#[test]
fn success_journals_before_every_side_effect_and_preserves_original_metadata() {
    let mut o = FakeOps::new();
    assert_eq!(
        Controller::new(&mut o).apply(false).unwrap(),
        Outcome::Applied
    );
    assert_eq!(o.live.len(), 6);
    assert!(o.enabled);
    let events = o.events.borrow().clone();
    let first_bind = events.iter().position(|(a, _)| a == "bind").unwrap();
    let inspected: Vec<String> = events[..first_bind]
        .iter()
        .filter(|(a, _)| a == "inspect")
        .filter_map(|(_, v)| v.as_str().map(str::to_owned))
        .collect();
    for entry in o.manifest.clone().unwrap() {
        assert!(
            inspected.contains(&text(&entry, "target")),
            "all stock hashes before any bind"
        );
    }
    for (i, (action, value)) in events.iter().enumerate() {
        if SIDE_EFFECTS.contains(&action.as_str()) {
            assert_eq!(
                events[i - 1].0,
                "journal",
                "{action} {value:?} not preceded by its intent"
            );
        }
    }
    for entry in o.journal_mounts() {
        let source = text(&entry, "source");
        let target = text(&entry, "target");
        assert!(source.starts_with("/var/lib/lg-native/mount-copies/"));
        assert_eq!(o.files[&source].mode, o.stock[&target].mode);
        assert!(o
            .live
            .iter()
            .find(|m| m.target == target)
            .unwrap()
            .is_readonly());
        assert_eq!(text(&entry, "state"), "mounted");
        assert_eq!(text(&entry, "bootId"), "boot-a");
        for key in ["mountId", "sourceDev", "sourceIno"] {
            assert!(entry.get(key).is_some_and(Value::is_integer), "{key}");
        }
    }
    let actions = o.actions();
    let clear = actions.iter().position(|a| a == "clear").unwrap();
    let kill = actions.iter().position(|a| a == "kill").unwrap();
    assert!(clear < kill);
    assert_eq!(
        o.journal_preload().get("pending"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn failure_restores_both_original_preload_values_and_owned_mounts() {
    for enabled in [true, false] {
        for failure in [
            "preload", "copy", "bind", "readonly", "clear", "kill", "launch",
        ] {
            let mut o = FakeOps::with(enabled, Some("airplay"), Some(failure));
            let error = Controller::new(&mut o).apply(false).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("injected {failure}"),
                "the original error propagates"
            );
            assert_eq!(o.enabled, enabled, "{failure}");
            assert!(o.disabled);
            assert!(o.live.is_empty());
            assert!(o
                .values("launch")
                .iter()
                .all(|v| v.get("preload") == Some(&Value::from("full"))));
        }
    }
}

#[test]
fn hash_change_has_no_mount_and_leaves_disabled() {
    let mut o = FakeOps::new();
    let last = text(o.manifest.as_ref().unwrap().last().unwrap(), "target");
    o.stock.get_mut(&last).unwrap().digest = h('c');
    o.files.extend(o.stock.clone());
    let error = Controller::new(&mut o).apply(false).unwrap_err();
    assert!(matches!(error, Error::Policy(_)), "{error}");
    assert!(!o.actions().contains(&"bind".to_owned()));
    assert!(o.disabled);
    assert!(o.enabled);
}

#[test]
fn update_removes_only_owned_legacy_mounts_once_then_checks_stock() {
    let mut o = FakeOps::new();
    let old = items(3);
    for e in &old {
        o.bind_value(e);
    }
    o.journal = Some(Value::Array(old.clone()));
    o.clear_events();
    Controller::new(&mut o).apply(false).unwrap();
    let removed: Vec<Value> = o.values("unmount");
    let expected: Vec<Value> = old
        .iter()
        .rev()
        .map(|e| Value::from(text(e, "target")))
        .collect();
    assert_eq!(removed, expected);
    assert_eq!(o.live.len(), 6);
}

#[test]
fn unowned_mount_and_corrupt_journal_are_never_unmounted() {
    for corrupt in [false, true] {
        let mut o = FakeOps::new();
        o.live = vec![lg_native::home::model::MountInfo {
            id: 1,
            target: text(&items(1)[0], "target"),
            options: vec!["ro".into()],
        }];
        if corrupt {
            o.journal = Some(
                json_object! {"version" => 1u64, "owner" => "someone-else", "mounts" => Value::Array(items(3))},
            );
        }
        assert!(matches!(
            Controller::new(&mut o).apply(false),
            Err(Error::Policy(_))
        ));
        assert!(!o.actions().contains(&"unmount".to_owned()));
    }
}

#[test]
fn rollback_is_manifest_independent_background_and_persistent() {
    let mut o = FakeOps::with(false, Some("com.webos.app.home"), None);
    Controller::new(&mut o).apply(false).unwrap();
    o.manifest = None;
    o.clear_events();
    assert_eq!(
        Controller::new(&mut o).rollback(true).unwrap(),
        Outcome::RolledBack
    );
    assert!(o.disabled);
    assert!(!o.enabled);
    assert!(o.live.is_empty());
    assert_eq!(o.values("launch"), vec![background_launch()]);
    assert!(!o.actions().contains(&"load_manifest".to_owned()));
    o.clear_events();
    Controller::new(&mut o).rollback(true).unwrap();
    assert!(!o.actions().contains(&"unmount".to_owned()));
}

#[test]
fn disabled_is_a_noop_and_a_crashed_preload_intent_recovers() {
    let mut o = FakeOps::new();
    o.disabled = true;
    assert_eq!(
        Controller::new(&mut o).apply(false).unwrap(),
        Outcome::Disabled
    );
    assert!(o.events.borrow().is_empty());
    let mut o = FakeOps::with(false, Some(HOME), None);
    o.journal = Some(json_object! {
        "version" => 1u64, "owner" => OWNER, "mounts" => Value::Array(vec![]),
        "preload" => json_object! {"original" => true, "bootId" => "boot-a", "pending" => true},
    });
    Controller::new(&mut o).rollback(true).unwrap();
    assert!(o.enabled);
}

#[test]
fn repeated_apply_reuses_immutable_copy_paths_without_accumulation() {
    let mut o = FakeOps::new();
    Controller::new(&mut o).apply(false).unwrap();
    let first: Vec<String> = o
        .journal_mounts()
        .iter()
        .map(|e| text(e, "source"))
        .collect();
    Controller::new(&mut o).apply(false).unwrap();
    let second: Vec<String> = o
        .journal_mounts()
        .iter()
        .map(|e| text(e, "source"))
        .collect();
    assert_eq!(first, second);
    let mut unique = second.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 6);
}

#[test]
fn failed_restore_retains_the_original_for_the_next_invocation() {
    let mut o = FakeOps::new();
    o.restore_true_fails = true;
    let error = Controller::new(&mut o).apply(false).unwrap_err();
    assert_eq!(error.to_string(), "restore API unavailable");
    assert_eq!(o.journal_preload().get("pending"), Some(&Value::Bool(true)));
    assert_eq!(
        o.journal_preload().get("original"),
        Some(&Value::Bool(true))
    );
    o.restore_true_fails = false;
    Controller::new(&mut o).rollback(true).unwrap();
    assert!(o.enabled);
}

#[test]
fn old_boot_pending_never_restores_a_stale_value_when_policy_is_unavailable() {
    let mut o = FakeOps::with(false, Some(HOME), None);
    o.journal = Some(json_object! {
        "version" => 1u64, "owner" => OWNER, "mounts" => Value::Array(vec![]),
        "preload" => json_object! {"original" => true, "bootId" => "boot-old", "pending" => true},
    });
    o.preload_fails = true;
    assert!(Controller::new(&mut o).rollback(true).is_err());
    assert!(
        !o.enabled,
        "a previous boot's state must not overwrite the current policy"
    );
}

#[test]
fn firmware_drift_under_legacy_mounts_unmounts_owned_then_aborts() {
    let mut o = FakeOps::new();
    let old = items(3);
    for e in &old {
        o.bind_value(e);
    }
    o.journal = Some(Value::Array(old.clone()));
    o.stock.get_mut(&text(&old[0], "target")).unwrap().digest = h('c');
    o.clear_events();
    assert!(matches!(
        Controller::new(&mut o).apply(false),
        Err(Error::Policy(_))
    ));
    assert!(o.live.is_empty());
    assert!(!o.actions().contains(&"bind".to_owned()));
    assert!(o.disabled);
}

#[test]
fn a_foreground_change_mid_transaction_never_steals_focus() {
    let mut o = FakeOps::new();
    o.fg_after_copy = Some("airplay".into());
    Controller::new(&mut o).apply(false).unwrap();
    assert_eq!(o.values("launch"), vec![background_launch()]);
}

#[test]
fn foreground_api_failure_uses_background_and_restores_pending_state() {
    let mut o = FakeOps::with(false, Some(HOME), None);
    o.journal = Some(json_object! {
        "version" => 1u64, "owner" => OWNER, "mounts" => Value::Array(vec![]),
        "preload" => json_object! {"original" => true, "bootId" => "boot-a", "pending" => true},
    });
    o.fg_fails = true;
    Controller::new(&mut o).rollback(false).unwrap();
    assert!(o.enabled);
    assert_eq!(o.values("launch"), vec![background_launch()]);
}

#[test]
fn a_failed_write_ahead_record_prevents_the_mount_side_effect() {
    let mut o = FakeOps::new();
    o.fail_journal_action = Some("bind".into());
    let error = Controller::new(&mut o).apply(false).unwrap_err();
    assert!(matches!(error, Error::Io { .. }), "{error}");
    assert!(!o.actions().contains(&"bind".to_owned()));
    assert!(o.enabled);
    assert!(o.disabled);
}

#[test]
fn criu_clear_refusal_never_kills_home_or_retries_kill() {
    let mut o = FakeOps::new();
    o.refuse_clear = true;
    let error = Controller::new(&mut o).apply(false).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Apply failed: CRIU restoration active; rollback incomplete:"),
        "{error}"
    );
    assert!(!o.actions().contains(&"kill".to_owned()));
    assert!(o.live.is_empty());
    assert!(o.enabled);
}

#[test]
fn copy_names_are_identical_to_the_previous_implementation() {
    // Computed by the Python runtime:
    // sha256(json.dumps([target, "b"*64, 0, 0, 493], separators=(",", ":")))
    let id = lg_native::home::model::copy_id(
        "/usr/palm/applications/com.webos.app.home/qml/Model/ShelfList.js",
        &h('b'),
        0,
        0,
        0o755,
    );
    assert_eq!(
        id,
        "96a12df350765d83730b91c25e1213666eae0725cabbb5d3f759fb1457160d06"
    );
}
