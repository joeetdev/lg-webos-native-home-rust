//! Gate ownership, verified removal, read-only readiness proof and recovery.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lg_native::gate::install::{build_chain, check_package_paths, install, next_state, InstallIo};
use lg_native::gate::{
    disable_gate, native_ready, recover_if_unready, remove_owned_filter, run, validate_state,
    Command, Disabled, KEY,
};
use lg_native::home::model::{FileInfo, ALLOWED};
use lg_native::home::Controller;
use lg_native::json::{self, Value};
use lg_native::{json_object, sha, Error, Result};
use support::{h, observed, ConfigService, FakeOps, ROOT};

fn entry(seed: &str) -> Value {
    json_object! {
        "file" => format!("{ROOT}/native-home-gate-loader-{}.js", sha::hex(seed)),
        "handler" => "handleNativeHomeGate",
    }
}

fn state(seed: &str) -> Value {
    json_object! {
        "version" => 1u64,
        "owner" => "lg-native-home-gate-v1",
        "entry" => entry(seed),
        "originalKeyFilters17" => Value::Array(vec![json_object! {"handler" => "old"}]),
        "files" => json_object! {format!("{ROOT}/native-home-gate-controller.qml") => h('a')},
        "parentExtension" => json_object! {"preserve" => true},
    }
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value.as_object_mut().unwrap().insert(key, field);
    value
}

fn native() -> Value {
    json_object! {"handler" => "native"}
}

#[test]
fn exact_owned_entry_only_is_removed() {
    let s = state("generated loader bytes");
    let e = entry("generated loader bytes");
    let foreign = vec![
        with(
            e.clone(),
            "file",
            entry("other loader").get("file").cloned().unwrap(),
        ),
        with(e.clone(), "handler", Value::from("handleSomeoneElse")),
        with(e.clone(), "extra", Value::Bool(true)),
        json_object! {"handler" => "handleSystemKeys"},
        Value::Null,
    ];
    let mut current = vec![foreign[0].clone(), e.clone()];
    current.extend(foreign[1..].iter().cloned());
    current.push(e.clone());
    assert_eq!(
        remove_owned_filter(&Value::Array(current), &s).unwrap(),
        foreign
    );
    assert_eq!(
        remove_owned_filter(&Value::Array(foreign.clone()), &s).unwrap(),
        foreign
    );
    assert!(validate_state(&s).is_ok());
    assert!(remove_owned_filter(&json_object! {}, &s).is_err());
}

#[test]
fn state_refuses_noncanonical_or_foreign_ownership_and_bad_file_maps() {
    let s = state("generated loader bytes");
    let file = s
        .get("entry")
        .and_then(|e| e.get("file"))
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let mut bad = vec![
        Value::Null,
        json_object! {},
        with(s.clone(), "owner", Value::from("foreign")),
        with(s.clone(), "version", Value::Bool(true)),
        with(s.clone(), "version", Value::from(2u64)),
        with(s.clone(), "version", json::parse("1.0").unwrap()),
    ];
    for path in [
        Value::from(format!("{ROOT}/native-home-gate-loader.js")),
        Value::from(file.replace("loader-", "loader-../")),
        Value::from(file.replace("/lg-native/", "/lg-native//")),
        Value::from(file.replace("lg-native/", "lg-native/../lg-native/")),
        Value::from(file.replace(".js", ".js\n")),
        Value::from(file.replace("loader-", "loader-A")),
        Value::from(file.replace("/var/", "/tmp/")),
        Value::Null,
    ] {
        bad.push(with(
            s.clone(),
            "entry",
            with(entry("generated loader bytes"), "file", path),
        ));
    }
    bad.push(with(
        s.clone(),
        "entry",
        with(
            entry("generated loader bytes"),
            "handler",
            Value::from("handleSystemKeys"),
        ),
    ));
    bad.push(with(
        s.clone(),
        "entry",
        with(entry("generated loader bytes"), "extra", Value::Bool(true)),
    ));
    for files in [
        Value::Array(vec![]),
        json_object! {"relative" => h('a')},
        json_object! {"/var/lib/../evil" => h('a')},
        json_object! {"/tmp/a\\b" => h('a')},
        json_object! {"/tmp/a" => h('A')},
        json_object! {"/tmp/a" => false},
    ] {
        bad.push(with(s.clone(), "files", files));
    }
    for case in bad {
        assert!(
            validate_state(&case).is_err(),
            "accepted {}",
            json::compact(&case)
        );
    }
}

#[test]
fn every_recorded_generation_is_removed_and_foreign_entries_survive() {
    let old = entry("trial generation");
    let new = entry("final generation");
    let s = with(
        state("final generation"),
        "ownedEntries",
        Value::Array(vec![old.clone(), new.clone()]),
    );
    let foreign = vec![
        entry("foreign generation"),
        with(old.clone(), "extra", Value::from("foreign")),
        json_object! {"file" => old.get("file").cloned().unwrap(), "handler" => "handleSystemKeys"},
    ];
    assert!(validate_state(&s).is_ok());
    for installed in [
        vec![old.clone()],
        vec![new.clone()],
        vec![old.clone(), new.clone()],
        vec![],
    ] {
        let mut chain = vec![foreign[0].clone()];
        chain.extend(installed.iter().cloned());
        chain.extend(foreign[1..].iter().cloned());
        let mut service = ConfigService::new(chain);
        let result = disable_gate(Some(&s), &mut service).unwrap();
        assert_eq!(service.filters, foreign);
        assert_eq!(
            result,
            Disabled {
                disabled: true,
                changed: !installed.is_empty()
            }
        );
    }
    let mut four = vec![new.clone()];
    four.extend((0..4).map(|i| entry(&i.to_string())));
    for entries in [
        Value::Null,
        json_object! {},
        Value::Array(vec![]),
        Value::Array(vec![old.clone()]),
        Value::Array(vec![new.clone(), new.clone()]),
        Value::Array(vec![
            new.clone(),
            with(old.clone(), "handler", Value::from("foreign")),
        ]),
        Value::Array(four),
    ] {
        assert!(validate_state(&with(s.clone(), "ownedEntries", entries)).is_err());
    }
    let mut max = vec![new];
    max.extend((0..3).map(|i| entry(&i.to_string())));
    assert!(validate_state(&with(s, "ownedEntries", Value::Array(max))).is_ok());
}

#[test]
fn disable_preserves_live_foreign_entries_verifies_the_write_and_is_idempotent() {
    let s = state("generated loader bytes");
    let e = entry("generated loader bytes");
    let foreign = vec![
        json_object! {"file" => "/firmware/native.js", "handler" => "handleSystemKeys"},
        entry("another generation"),
        with(e.clone(), "extra", Value::from("foreign")),
    ];
    let mut service = ConfigService::new(vec![
        foreign[0].clone(),
        e.clone(),
        foreign[1].clone(),
        foreign[2].clone(),
    ]);
    assert_eq!(
        disable_gate(Some(&s), &mut service).unwrap(),
        Disabled {
            disabled: true,
            changed: true
        }
    );
    assert_eq!(service.filters, foreign);
    assert_eq!(
        service.method_names(),
        ["getConfigs", "setConfigs", "getConfigs"]
    );
    service.calls.clear();
    assert_eq!(
        disable_gate(Some(&s), &mut service).unwrap(),
        Disabled {
            disabled: true,
            changed: false
        }
    );
    assert_eq!(service.calls.len(), 1);
    service.calls.clear();
    assert_eq!(
        disable_gate(None, &mut service).unwrap(),
        Disabled {
            disabled: false,
            changed: false
        }
    );
    assert!(service.calls.is_empty());
    let mut service = ConfigService::new(vec![e.clone()]);
    service.bad_readback = true;
    assert!(disable_gate(Some(&s), &mut service)
        .unwrap_err()
        .to_string()
        .contains("readback"));
    for method in ["getConfigs", "setConfigs"] {
        let mut service = ConfigService::new(vec![e.clone()]);
        service.fail = Some(format!("com.webos.service.config/{method}"));
        assert!(disable_gate(Some(&s), &mut service).is_err(), "{method}");
    }
    for response in [
        json_object! {},
        json_object! {"returnValue" => true},
        json_object! {"returnValue" => true, "configs" => json_object! {KEY => json_object! {}}},
    ] {
        let mut call = |_: &str, _: &Value| -> Result<Value> { Ok(response.clone()) };
        assert!(disable_gate(Some(&s), &mut call).is_err());
    }
}

#[test]
fn readback_detects_type_or_order_drift_in_foreign_filters() {
    let s = state("generated loader bytes");
    let before = Value::Array(vec![
        entry("generated loader bytes"),
        json_object! {"foreignVersion" => 0u64},
        native(),
    ]);
    for drifted in [
        Value::Array(vec![json_object! {"foreignVersion" => false}, native()]),
        Value::Array(vec![native(), json_object! {"foreignVersion" => 0u64}]),
        Value::Array(vec![
            json::parse(r#"{"foreignVersion":0.0}"#).unwrap(),
            native(),
        ]),
    ] {
        let mut responses = vec![
            json_object! {"returnValue" => true, "configs" => json_object! {KEY => before.clone()}},
            json_object! {"returnValue" => true},
            json_object! {"returnValue" => true, "configs" => json_object! {KEY => drifted}},
        ]
        .into_iter();
        let mut call = |_: &str, _: &Value| -> Result<Value> { Ok(responses.next().unwrap()) };
        assert!(disable_gate(Some(&s), &mut call)
            .unwrap_err()
            .to_string()
            .contains("readback"));
    }
}

#[test]
fn recovery_keeps_a_ready_gate_but_checker_errors_still_remove_only_the_gate() {
    let s = state("generated loader bytes");
    let foreign = json_object! {"file" => "/native.js", "handler" => "handleSystemKeys"};
    let mut service = ConfigService::new(vec![entry("generated loader bytes"), foreign.clone()]);
    let result = recover_if_unready(Some(&s), &mut service, || Ok(true)).unwrap();
    assert!(result.ready && !result.disabled);
    assert!(service.calls.is_empty());
    let checkers: Vec<Box<dyn Fn() -> Result<bool>>> = vec![
        Box::new(|| Ok(false)),
        Box::new(|| Err(Error::Policy("native Home runtime cannot be loaded".into()))),
        Box::new(|| Err(Error::Timeout("readiness".into()))),
    ];
    for checker in checkers {
        let mut service =
            ConfigService::new(vec![entry("generated loader bytes"), foreign.clone()]);
        let result = recover_if_unready(Some(&s), &mut service, checker).unwrap();
        assert!(!result.ready && result.disabled);
        assert_eq!(service.filters, vec![foreign.clone()]);
    }
    let mut service = ConfigService::new(vec![entry("generated loader bytes")]);
    service.bad_readback = true;
    assert!(
        recover_if_unready(Some(&s), &mut service, || Err(Error::Policy(
            "broken".into()
        )))
        .is_err()
    );
    let mut service = ConfigService::new(vec![]);
    let result =
        recover_if_unready(None, &mut service, || Err(Error::Policy("broken".into()))).unwrap();
    assert!(!result.ready && !result.disabled);
    assert!(service.calls.is_empty());
}

/// Journal and mounts produced by the ACTUAL apply controller over all eight targets.
fn applied_eight() -> FakeOps {
    let mut ops = FakeOps::new();
    let mut targets: Vec<&str> = ALLOWED.to_vec();
    targets.sort_unstable();
    let manifest: Vec<Value> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            json_object! {
                "source" => format!("{ROOT}/candidate/file{i}"), "target" => *t,
                "originalHash" => h('a'), "newHash" => sha::hex(t),
            }
        })
        .collect();
    ops.files.clear();
    ops.stock.clear();
    for (i, e) in manifest.iter().enumerate() {
        let source = support::text(e, "source");
        let target = support::text(e, "target");
        ops.files.insert(
            source.clone(),
            FileInfo {
                mode: 493,
                ..observed(&source, &support::text(e, "newHash"), i as u64 + 20)
            },
        );
        ops.stock.insert(
            target.clone(),
            FileInfo {
                mode: 493,
                ..observed(&target, &h('a'), i as u64 + 100)
            },
        );
    }
    ops.files.extend(ops.stock.clone());
    ops.manifest = Some(manifest);
    Controller::new(&mut ops).apply(false).unwrap();
    ops.clear_events();
    ops
}

fn only_reads(ops: &FakeOps) -> bool {
    ops.events
        .borrow()
        .iter()
        .all(|(a, _)| a == "inspect" || a == "load_manifest")
}

type Snapshot = (
    Vec<(String, FileInfo)>,
    Vec<lg_native::home::model::MountInfo>,
    Option<Value>,
    bool,
    bool,
);

fn snapshot(ops: &FakeOps) -> Snapshot {
    (
        ops.files.clone().into_iter().collect(),
        ops.live.clone(),
        ops.journal.clone(),
        ops.enabled,
        ops.disabled,
    )
}

#[test]
fn controller_generated_eight_readonly_owned_overlays_are_ready_without_writes() {
    let ops = applied_eight();
    let before = snapshot(&ops);
    assert!(native_ready(&ops).unwrap());
    assert_eq!(before, snapshot(&ops));
    assert!(only_reads(&ops));
}

fn edit_mount(ops: &mut FakeOps, index: usize, edit: impl FnOnce(&mut lg_native::json::Map)) {
    let journal = ops.journal.as_mut().unwrap().as_object_mut().unwrap();
    let mounts = journal.get_mut("mounts").unwrap();
    let Value::Array(items) = mounts else {
        panic!("mounts")
    };
    edit(items[index].as_object_mut().unwrap());
}

fn edit_journal(ops: &mut FakeOps, edit: impl FnOnce(&mut lg_native::json::Map)) {
    edit(ops.journal.as_mut().unwrap().as_object_mut().unwrap());
}

fn edit_preload(ops: &mut FakeOps, key: &str, value: Value) {
    edit_journal(ops, |j| {
        j.get_mut("preload")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key, value);
    });
}

fn mount_field(ops: &FakeOps, key: &str) -> String {
    support::text(&ops.journal_mounts()[0], key)
}

#[test]
fn unhealthy_or_incoherent_proofs_release_the_gate_without_touching_home() {
    type Mutation = Box<dyn Fn(&mut FakeOps)>;
    let mutations: Vec<(&str, Mutation)> = vec![
        ("disabled", Box::new(|o| o.disabled = true)),
        (
            "six manifest",
            Box::new(|o| o.manifest.as_mut().unwrap().truncate(6)),
        ),
        ("no journal", Box::new(|o| o.journal = None)),
        (
            "legacy journal",
            Box::new(|o| o.journal = Some(Value::Array(o.journal_mounts()))),
        ),
        (
            "seven journal",
            Box::new(|o| {
                edit_journal(o, |j| {
                    if let Some(Value::Array(m)) = j.get_mut("mounts") {
                        m.pop();
                    }
                })
            }),
        ),
        (
            "no preload",
            Box::new(|o| {
                edit_journal(o, |j| {
                    j.remove("preload");
                })
            }),
        ),
        (
            "pending preload",
            Box::new(|o| edit_preload(o, "pending", Value::Bool(true))),
        ),
        (
            "old preload boot",
            Box::new(|o| edit_preload(o, "bootId", Value::from("old"))),
        ),
        (
            "wrong journal owner",
            Box::new(|o| edit_journal(o, |j| j.insert("owner", "foreign"))),
        ),
        (
            "old mount boot",
            Box::new(|o| edit_mount(o, 0, |m| m.insert("bootId", "old"))),
        ),
        (
            "missing mount boot",
            Box::new(|o| {
                edit_mount(o, 0, |m| {
                    m.remove("bootId");
                })
            }),
        ),
        (
            "missing mount id",
            Box::new(|o| {
                edit_mount(o, 0, |m| {
                    m.remove("mountId");
                })
            }),
        ),
        (
            "missing source inode",
            Box::new(|o| {
                edit_mount(o, 0, |m| {
                    m.remove("sourceIno");
                })
            }),
        ),
        (
            "missing source device",
            Box::new(|o| {
                edit_mount(o, 0, |m| {
                    m.remove("sourceDev");
                })
            }),
        ),
        (
            "wrong mount id",
            Box::new(|o| edit_mount(o, 0, |m| m.insert("mountId", 999u64))),
        ),
        (
            "not mounted state",
            Box::new(|o| edit_mount(o, 0, |m| m.insert("state", "bind"))),
        ),
        (
            "not readonly",
            Box::new(|o| o.live[0].options = vec!["rw".into()]),
        ),
        (
            "stacked bind",
            Box::new(|o| {
                let m = o.live[0].clone();
                o.live.push(m);
            }),
        ),
        (
            "missing bind",
            Box::new(|o| {
                o.live.pop();
            }),
        ),
        (
            "foreign inode",
            Box::new(|o| {
                let t = mount_field(o, "target");
                o.files.get_mut(&t).unwrap().ino = 99999;
            }),
        ),
        (
            "bad target digest",
            Box::new(|o| {
                let t = mount_field(o, "target");
                o.files.get_mut(&t).unwrap().digest = h('c');
            }),
        ),
        (
            "bad copy digest",
            Box::new(|o| {
                let s = mount_field(o, "source");
                o.files.get_mut(&s).unwrap().digest = h('c');
            }),
        ),
        (
            "bad input digest",
            Box::new(|o| {
                let s = support::text(&o.manifest.as_ref().unwrap()[0], "source");
                o.files.get_mut(&s).unwrap().digest = h('c');
            }),
        ),
        (
            "source symlink",
            Box::new(|o| {
                let s = mount_field(o, "source");
                o.files.get_mut(&s).unwrap().symlink = true;
            }),
        ),
        (
            "source nonroot",
            Box::new(|o| {
                let s = mount_field(o, "source");
                o.files.get_mut(&s).unwrap().uid = 1000;
            }),
        ),
        (
            "target writable",
            Box::new(|o| {
                let t = mount_field(o, "target");
                o.files.get_mut(&t).unwrap().mode = 0o777;
            }),
        ),
        (
            "journal metadata drift",
            Box::new(|o| edit_mount(o, 0, |m| m.insert("mode", 0o644u64))),
        ),
        (
            "manifest newHash drift",
            Box::new(|o| {
                o.manifest.as_mut().unwrap()[0]
                    .as_object_mut()
                    .unwrap()
                    .insert("newHash", h('f'));
            }),
        ),
        (
            "manifest originalHash drift",
            Box::new(|o| {
                o.manifest.as_mut().unwrap()[0]
                    .as_object_mut()
                    .unwrap()
                    .insert("originalHash", h('f'));
            }),
        ),
        (
            "manifest inputSource drift",
            Box::new(|o| {
                let first = o.manifest.as_mut().unwrap()[0].as_object_mut().unwrap();
                let source = format!(
                    "{}-other",
                    first.get("source").and_then(Value::as_str).unwrap()
                );
                first.insert("source", source);
            }),
        ),
    ];
    let mut s = state("generated loader bytes");
    s.as_object_mut().unwrap().remove("files");
    for (label, mutate) in mutations {
        let mut ops = applied_eight();
        mutate(&mut ops);
        let before = snapshot(&ops);
        let mut service = ConfigService::new(vec![native(), entry("generated loader bytes")]);
        let result = recover_if_unready(Some(&s), &mut service, || native_ready(&ops)).unwrap();
        assert!(!result.ready, "{label}");
        assert!(result.disabled, "{label}");
        assert_eq!(service.filters, vec![native()], "{label}");
        assert_eq!(before, snapshot(&ops), "{label}");
        assert!(only_reads(&ops), "{label}");
    }
}

#[test]
fn command_line_disable_never_needs_home_and_recovery_errors_do_not_block_removal() {
    let s = state("generated loader bytes");
    let run_cli = |command: Command,
                   saved: Result<Option<Value>>,
                   checker: Result<bool>,
                   service: &mut ConfigService| {
        let mut out = Vec::new();
        let mut checked = false;
        let code = run(
            command,
            || saved,
            service,
            || {
                checked = true;
                checker
            },
            &mut out,
        );
        let report = json::parse_bytes(&out).unwrap();
        (code, report, checked)
    };
    let fresh = || ConfigService::new(vec![entry("generated loader bytes"), native()]);

    let mut service = fresh();
    let (code, report, checked) = run_cli(
        Command::Disable,
        Ok(Some(s.clone())),
        Ok(true),
        &mut service,
    );
    assert_eq!(
        (code, report.get("disabled"), checked),
        (0, Some(&Value::Bool(true)), false)
    );
    assert_eq!(service.filters, vec![native()]);

    let mut service = fresh();
    let (code, report, _) = run_cli(
        Command::RecoverIfUnready,
        Ok(Some(s.clone())),
        Err(Error::Policy("x".into())),
        &mut service,
    );
    assert_eq!(code, 0);
    assert_eq!(report.get("ready"), Some(&Value::Bool(false)));
    assert_eq!(report.get("disabled"), Some(&Value::Bool(true)));
    assert_eq!(report.get("reason"), Some(&Value::from("PolicyError: x")));

    let mut service = fresh();
    let (code, report, _) = run_cli(
        Command::RecoverIfUnready,
        Ok(Some(s.clone())),
        Ok(true),
        &mut service,
    );
    assert_eq!(
        (code, report.get("ready"), report.get("disabled")),
        (0, Some(&Value::Bool(true)), Some(&Value::Bool(false)))
    );
    assert!(service.calls.is_empty());

    for command in [Command::Disable, Command::RecoverIfUnready] {
        let mut service = fresh();
        let (code, report, checked) = run_cli(command, Ok(None), Ok(true), &mut service);
        assert_eq!(
            (code, report.get("disabled"), report.get("state")),
            (0, Some(&Value::Bool(false)), Some(&Value::from("absent")))
        );
        assert_eq!(
            report.get("ready").is_some(),
            command == Command::RecoverIfUnready
        );
        assert!(service.calls.is_empty() && !checked);
    }

    let mut service = fresh();
    let refused = Err(Error::Policy(
        "Requires root on Linux; not a host installer".into(),
    ));
    let (code, report, _) = run_cli(Command::Disable, refused, Ok(true), &mut service);
    assert_eq!(code, 1);
    assert!(report
        .get("error")
        .and_then(Value::as_str)
        .unwrap()
        .contains("Requires root"));
    assert!(service.calls.is_empty());

    let mut service = ConfigService::new(vec![entry("generated loader bytes")]);
    service.bad_readback = true;
    let (code, report, _) = run_cli(Command::Disable, Ok(Some(s)), Ok(true), &mut service);
    assert_eq!(code, 1);
    assert!(report
        .get("error")
        .and_then(Value::as_str)
        .unwrap()
        .contains("readback"));
}

#[cfg(unix)]
#[test]
fn luna_boundary_refuses_unapproved_methods_before_spawning() {
    let error = lg_native::gate::luna_call("com.webos.applicationManager/launch", &json_object! {})
        .unwrap_err();
    assert!(
        matches!(error, Error::Policy(ref m) if m == "Unapproved Luna method"),
        "{error}"
    );
}

// --- installer -------------------------------------------------------------

fn system_keys() -> Value {
    json_object! {"file" => "/usr/lib/qt5/qml/KeyFilters/systemUi.js", "handler" => "handleSystemKeys"}
}

#[test]
fn the_gate_is_inserted_before_system_keys_preserving_every_other_handler() {
    let before = vec![
        json_object! {"file" => "/sys/a.js", "handler" => "safety"},
        system_keys(),
        json_object! {"file" => "/sys/z.js", "handler" => "later"},
    ];
    let e = entry("a");
    assert_eq!(
        build_chain(&Value::Array(before.clone()), &e, None).unwrap(),
        vec![
            before[0].clone(),
            e.clone(),
            before[1].clone(),
            before[2].clone()
        ]
    );
    assert!(build_chain(&Value::Array(vec![]), &e, None).is_err());
    assert!(build_chain(&Value::Array(vec![system_keys(), system_keys()]), &e, None).is_err());
    let unknown = with(entry("foreign"), "file", Value::from("/elsewhere.js"));
    assert!(build_chain(&Value::Array(vec![unknown, system_keys()]), &e, None).is_err());
}

#[test]
fn next_state_records_both_generations_before_the_config_transition() {
    let a = entry("a");
    let b = entry("b");
    let old = json_object! {
        "version" => 1u64, "owner" => "lg-native-home-gate-v1", "entry" => a.clone(),
        "ownedEntries" => Value::Array(vec![a.clone()]),
        "originalKeyFilters17" => Value::Array(vec![Value::from("original")]),
        "originalHook" => "hook", "originalRollback" => "rollback",
    };
    let package = json_object! {
        "version" => 1u64, "owner" => "lg-native-home-gate-v1", "entry" => b.clone(),
        "files" => json_object! {}, "mode" => "final",
    };
    let next = next_state(
        &package,
        Some(&old),
        &Value::Array(vec![Value::from("current")]),
        "unused",
        None,
    )
    .unwrap();
    assert_eq!(next.get("ownedEntries"), Some(&Value::Array(vec![a, b])));
    assert_eq!(
        next.get("originalKeyFilters17"),
        Some(&Value::Array(vec![Value::from("original")]))
    );
    assert_eq!(next.get("originalHook"), Some(&Value::from("hook")));
    assert_eq!(next.get("originalRollback"), Some(&Value::from("rollback")));
    let fresh = next_state(&package, None, &Value::Array(vec![]), "hook text", None).unwrap();
    assert_eq!(fresh.get("originalHook"), Some(&Value::from("hook text")));
    assert!(
        fresh.get("originalRollback").is_none(),
        "never invents a legacy rollback record"
    );
}

#[derive(Default)]
struct FakeInstall {
    events: Vec<&'static str>,
    filters: Vec<Value>,
}

impl InstallIo for FakeInstall {
    fn check_files(&mut self, _package: &Value) -> Result<()> {
        self.events.push("files");
        Ok(())
    }
    fn load_state(&mut self) -> Result<Option<Value>> {
        Ok(None)
    }
    fn get_filters(&mut self) -> Result<Value> {
        Ok(Value::Array(self.filters.clone()))
    }
    fn read_originals(&mut self) -> Result<(String, Option<String>)> {
        Ok(("hook".into(), None))
    }
    fn install_safety(&mut self) -> Result<()> {
        self.events.push("safety");
        Ok(())
    }
    fn save_state(&mut self, _state: &Value) -> Result<()> {
        self.events.push("state");
        Ok(())
    }
    fn arm_guard(&mut self) -> Result<()> {
        self.events.push("guard");
        Ok(())
    }
    fn set_filters(&mut self, chain: &[Value]) -> Result<()> {
        self.events.push("config");
        self.filters = chain.to_vec();
        Ok(())
    }
}

#[test]
fn installation_arms_recovery_and_records_ownership_before_activation() {
    let e = entry("a");
    let package = json_object! {
        "version" => 1u64, "owner" => "lg-native-home-gate-v1", "entry" => e.clone(),
        "files" => json_object! {}, "mode" => "trial",
    };
    let mut io = FakeInstall {
        filters: vec![system_keys()],
        ..Default::default()
    };
    install(&package, &mut io).unwrap();
    assert_eq!(io.events, ["files", "safety", "state", "guard", "config"]);
    assert_eq!(io.filters[0], e);
}

#[test]
fn package_assets_must_be_content_addressed() {
    let loader = format!("{ROOT}/native-home-gate-loader-{}.js", h('1'));
    let package = |files: Value, entry_file: &str| {
        json_object! {
            "version" => 1u64, "owner" => "lg-native-home-gate-v1", "mode" => "final",
            "entry" => json_object! {"file" => entry_file, "handler" => "handleNativeHomeGate"},
            "files" => files,
        }
    };
    let good = json_object! {
        format!("{ROOT}/gate-proof-{}.js", h('2')) => h('2'),
        format!("{ROOT}/native-home-gate-{}.qml", h('3')) => h('3'),
        loader.clone() => h('1'),
    };
    assert_eq!(
        check_package_paths(&package(good.clone(), &loader))
            .unwrap()
            .len(),
        3
    );
    let mismatched = json_object! {
        format!("{ROOT}/gate-proof-{}.js", h('2')) => h('4'),
        format!("{ROOT}/native-home-gate-{}.qml", h('3')) => h('3'),
        loader.clone() => h('1'),
    };
    assert!(check_package_paths(&package(mismatched, &loader)).is_err());
    let two = json_object! {loader.clone() => h('1'), format!("{ROOT}/gate-proof-{}.js", h('2')) => h('2')};
    assert!(check_package_paths(&package(two, &loader)).is_err());
    let other_loader = format!("{ROOT}/native-home-gate-loader-{}.js", h('9'));
    assert!(check_package_paths(&package(good, &other_loader)).is_err());
}

#[test]
fn trial_hooks_from_both_implementations_are_recognised_as_owned() {
    use lg_native::gate::install::{is_owned_trial_hook, LEGACY_TRIAL_HOOK, TRIAL_HOOK};
    assert!(is_owned_trial_hook(TRIAL_HOOK.as_bytes()));
    assert!(is_owned_trial_hook(LEGACY_TRIAL_HOOK.as_bytes()));
    assert!(!is_owned_trial_hook(b"#!/bin/sh\nrm -rf /\n"));
    assert!(TRIAL_HOOK.contains("exec /var/lib/lg-native/lg-native gate guard --spawn"));
}
