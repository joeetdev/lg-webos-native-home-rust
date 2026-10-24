//! Pure Home rules: allowlist, manifest/journal validation, ownership proofs,
//! launch decisions, mountinfo parsing, boot identity and readiness waiting.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lg_native::fsx::{Meta, S_IFLNK, S_IFREG};
use lg_native::home::model::*;
use lg_native::home::{boot_already_claimed, wait_ready, ReadinessProbe};
use lg_native::json::{self, Value};
use lg_native::{json_object, Error, Result};
use support::{h, items, observed, ROOT};

type Mutation = Box<dyn Fn(&mut FileInfo)>;

fn manifest(entries: Vec<Value>) -> Value {
    Value::Array(entries)
}

fn set(entries: &mut [Value], index: usize, key: &str, value: Value) {
    entries[index].as_object_mut().unwrap().insert(key, value);
}

#[test]
fn manifest_guard_exact_allowlist_hashes_and_staging() {
    assert_eq!(
        validate_manifest(&manifest(items(6)), false).unwrap().len(),
        6
    );
    for count in [3, 4, 5] {
        assert!(validate_manifest(&manifest(items(count)), false).is_err());
        assert_eq!(
            validate_manifest(&manifest(items(count)), true)
                .unwrap()
                .len(),
            count
        );
    }
    let mut bad: Vec<Value> = Vec::new();
    let rel0 = support::REL[0];
    for target in [
        "/etc/passwd",
        "/tmp/criu_enable",
        &format!("{QML}../qml/{rel0}"),
        "/mnt/lg/cmn_data/sdp/sdx/detailconfig.json",
    ] {
        let mut x = items(6);
        set(&mut x, 0, "target", Value::from(target));
        bad.push(manifest(x));
    }
    for source in [
        format!("{ROOT}/../evil"),
        format!("{ROOT}/a/../b"),
        format!("{ROOT}//b"),
        format!("{ROOT}/a\\b"),
        format!("{ROOT}/home-mounts.json"),
        format!("{ROOT}/native_home.py"),
        format!("{ROOT}/home-disabled"),
        format!("{ROOT}/candidate\0bad"),
        format!("{ROOT}-other/file"),
        format!("{ROOT}/lg-native"),
        format!("{ROOT}/x/remote-controls-state.json"),
    ] {
        let mut x = items(6);
        set(&mut x, 0, "source", Value::from(source));
        bad.push(manifest(x));
    }
    for hash in [
        Value::from(""),
        Value::from("a".repeat(40)),
        Value::from(h('G')),
        Value::from(h('A')),
        Value::Null,
    ] {
        let mut x = items(6);
        set(&mut x, 0, "newHash", hash);
        bad.push(manifest(x));
    }
    let mut x = items(6);
    let first_target = x[0].get("target").cloned().unwrap();
    set(&mut x, 1, "target", first_target);
    bad.push(manifest(x));
    let mut x = items(6);
    let first_source = x[0].get("source").cloned().unwrap();
    set(&mut x, 1, "source", first_source);
    bad.push(manifest(x));
    let mut long = items(6);
    long.push(items(1).remove(0));
    bad.extend([
        manifest(items(2)),
        manifest(long),
        json_object! {},
        Value::Null,
    ]);
    for case in bad {
        assert!(
            validate_manifest(&case, true).is_err(),
            "accepted {}",
            json::compact(&case)
        );
    }
}

#[test]
fn manifest_cannot_inject_journal_identity_or_metadata() {
    for extra in [
        "mountId",
        "bootId",
        "sourceIno",
        "sourceDev",
        "inputSource",
        "state",
        "mode",
    ] {
        let mut m = items(6);
        set(&mut m, 0, extra, Value::from("injected"));
        assert!(validate_manifest(&manifest(m), false).is_err(), "{extra}");
    }
}

#[test]
fn journal_rejects_protected_duplicate_and_traversal_targets_and_bad_preload() {
    let mut duplicate = items(3);
    duplicate.push(duplicate[0].clone());
    let mut protected = items(3);
    set(&mut protected, 0, "target", Value::from("/tmp/criu_enable"));
    let mut traversal = items(3);
    set(
        &mut traversal,
        0,
        "source",
        Value::from(format!("{ROOT}/a/../../evil")),
    );
    for case in [duplicate, protected, traversal] {
        assert!(validate_journal(Some(Value::Array(case))).is_err());
    }
    let bad_preload = json_object! {
        "owner" => OWNER, "version" => 1u64, "mounts" => Value::Array(vec![]),
        "preload" => json_object! {"original" => "true", "pending" => true, "bootId" => "a"},
    };
    assert!(validate_journal(Some(bad_preload)).is_err());
    assert_eq!(validate_journal(None).unwrap(), Journal::empty());
    let legacy = validate_journal(Some(Value::Array(items(3)))).unwrap();
    assert_eq!(legacy.mounts.len(), 3);
    assert_eq!(legacy.fields.get("owner"), Some(&Value::from(OWNER)));
}

fn fixture_manifest() -> Vec<Value> {
    let text = include_str!("../../../gate/fixtures/home-final-manifest.json");
    json::parse(text).unwrap().as_array().cloned().unwrap()
}

#[test]
fn seventh_dashboard_target_is_exact_and_six_remain_valid_for_recovery() {
    let entries: Vec<Value> = fixture_manifest()
        .into_iter()
        .filter(|e| {
            !support::text(e, "target").ends_with("/HomeDashboard.qml")
                && !support::text(e, "target").ends_with("/MobileComponent.qml")
        })
        .collect();
    assert_eq!(entries.len(), 6);
    let extra = json_object! {
        "source" => format!("{ROOT}/categorized-HomeDashboard.qml"),
        "target" => format!("{QML}UserInterfaceLayer/Containers/Main/HomeDashboard.qml"),
        "originalHash" => h('a'), "newHash" => h('b'),
    };
    let mut seven = entries.clone();
    seven.push(extra.clone());
    assert_eq!(
        validate_manifest(&manifest(seven.clone()), false)
            .unwrap()
            .len(),
        7
    );
    assert_eq!(
        validate_manifest(&manifest(entries.clone()), false)
            .unwrap()
            .len(),
        6
    );
    let foreign = json_object! {
        "source" => format!("{ROOT}/extra.qml"),
        "target" => format!("{QML}UserInterfaceLayer/Containers/Main/Other.qml"),
        "originalHash" => h('a'), "newHash" => h('b'),
    };
    let mut eight = seven.clone();
    eight.push(foreign.clone());
    assert!(validate_manifest(&manifest(eight), false).is_err());
    let mut with_foreign = entries;
    with_foreign.push(foreign);
    assert!(validate_manifest(&manifest(with_foreign), false).is_err());
}

#[test]
fn eighth_mobile_target_is_exact_and_duplicates_are_rejected() {
    let baseline: Vec<Value> = fixture_manifest()
        .into_iter()
        .filter(|e| !support::text(e, "target").ends_with("/MobileComponent.qml"))
        .collect();
    assert_eq!(baseline.len(), 7);
    let extra = json_object! {
        "source" => format!("{ROOT}/six-MobileComponent.qml"),
        "target" => format!("{QML}UserInterfaceLayer/Containers/Sub/MobileComponent.qml"),
        "originalHash" => h('a'), "newHash" => h('b'),
    };
    let mut eight = baseline.clone();
    eight.push(extra.clone());
    assert_eq!(
        validate_manifest(&manifest(eight.clone()), false)
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        validate_manifest(&manifest(baseline.clone()), false)
            .unwrap()
            .len(),
        7
    );
    let mut nine = eight.clone();
    nine.push(json_object! {
        "source" => format!("{ROOT}/bad.qml"),
        "target" => format!("{QML}UserInterfaceLayer/Containers/Sub/Other.qml"),
        "originalHash" => h('a'), "newHash" => h('b'),
    });
    assert!(validate_manifest(&manifest(nine), false).is_err());
    let mut duplicate = eight;
    duplicate.push(extra);
    assert!(validate_manifest(&manifest(duplicate), false).is_err());
}

fn planned_inputs() -> (Vec<ManifestEntry>, Vec<FileInfo>, Vec<FileInfo>) {
    let entries = validate_manifest(&manifest(items(6)), false).unwrap();
    let sources = entries
        .iter()
        .map(|e| observed(&e.source, &h('b'), 12))
        .collect();
    let targets = entries
        .iter()
        .map(|e| observed(&e.target, &h('a'), 99))
        .collect();
    (entries, sources, targets)
}

#[test]
fn all_hashes_and_metadata_are_checked_before_a_plan_exists() {
    let (entries, sources, targets) = planned_inputs();
    assert_eq!(plan_mounts(&entries, &sources, &targets).unwrap().len(), 6);
    let mutations: Vec<Mutation> = vec![
        Box::new(|f| f.digest = h('c')),
        Box::new(|f| f.symlink = true),
        Box::new(|f| f.uid = 1000),
        Box::new(|f| f.mode = 0o666),
        Box::new(|f| f.nlink = 2),
        Box::new(|f| f.regular = false),
        Box::new(|f| f.realpath = "/etc/passwd".into()),
        Box::new(|f| f.mode = 0o4755),
    ];
    for mutate in &mutations {
        let mut bad = sources.clone();
        mutate(bad.last_mut().unwrap());
        assert!(plan_mounts(&entries, &bad, &targets).is_err());
    }
    let mut bad = targets.clone();
    bad.last_mut().unwrap().digest = h('c');
    assert!(plan_mounts(&entries, &sources, &bad).is_err());
}

#[test]
fn owned_mount_requires_inode_hash_metadata_and_a_single_mount() {
    let entry = items(1).remove(0);
    let entry_map = entry.as_object().unwrap().clone();
    let source = observed(&support::text(&entry, "source"), &h('b'), 12);
    let target = observed(&support::text(&entry, "target"), &h('b'), 12);
    let mounts = vec![MountInfo {
        id: 55,
        target: target.path.clone(),
        options: vec!["ro".into()],
    }];
    assert!(owned_mount(&entry_map, &source, &target, &mounts, "boot-a").unwrap());
    assert!(!owned_mount(&entry_map, &source, &target, &[], "boot-a").unwrap());
    let changes: Vec<Mutation> = vec![
        Box::new(|f| f.ino = 13),
        Box::new(|f| f.dev = 3),
        Box::new(|f| f.digest = h('c')),
        Box::new(|f| f.uid = 1000),
    ];
    for change in &changes {
        let mut bad = target.clone();
        change(&mut bad);
        assert!(owned_mount(&entry_map, &source, &bad, &mounts, "boot-a").is_err());
    }
    let stacked = [mounts.clone(), mounts.clone()].concat();
    assert!(owned_mount(&entry_map, &source, &target, &stacked, "boot-a").is_err());
    let mut modern = entry_map.clone();
    for (key, value) in [
        ("mountId", Value::from(55u64)),
        ("bootId", Value::from("boot-a")),
        ("sourceDev", Value::from(2u64)),
        ("sourceIno", Value::from(12u64)),
    ] {
        modern.insert(key, value);
    }
    assert!(owned_mount(&modern, &source, &target, &mounts, "boot-a").unwrap());
    for (key, value) in [
        ("mountId", Value::from(56u64)),
        ("bootId", Value::from("boot-b")),
        ("sourceIno", Value::from(999u64)),
    ] {
        let mut bad = modern.clone();
        bad.insert(key, value);
        assert!(
            owned_mount(&bad, &source, &target, &mounts, "boot-a").is_err(),
            "{key}"
        );
    }
    let mut null_identity = entry_map;
    null_identity.insert("mountId", Value::Null);
    assert!(
        owned_mount(&null_identity, &source, &target, &mounts, "boot-a").unwrap(),
        "null means unrecorded"
    );
}

#[test]
fn foreground_guard_and_exact_process_selection() {
    for fg in ["", HOME, "com.webos.app.livetv", "com.webos.app.lgchannels"] {
        assert_eq!(launch_payload(Some(fg), false), json_object! {"id" => HOME});
        assert_eq!(
            launch_payload(Some(fg), true).get("preload"),
            Some(&Value::from("full"))
        );
    }
    for fg in [
        Some("airplay"),
        Some("com.webos.app.hdmi1"),
        Some("netflix"),
        None,
        Some("unknown"),
    ] {
        assert_eq!(
            launch_payload(fg, false).get("preload"),
            Some(&Value::from("full"))
        );
    }
    let processes = vec![
        (13, "/usr/sbin/sdx".to_owned()),
        (12, "/usr/bin/com.webos.app.home (deleted)".to_owned()),
        (11, "/usr/bin/com.webos.app.home.old".to_owned()),
        (10, "/usr/bin/com.webos.app.home".to_owned()),
        (9, EXE.to_owned()),
    ];
    assert_eq!(home_pids(&processes), vec![9, 10]);
}

#[test]
fn mountinfo_parser_decodes_paths_and_keeps_duplicate_mounts() {
    let raw = "10 1 8:1 /file /some\\040path ro,relatime - ext4 /dev/a rw\n11 1 8:1 /file /some\\040path ro - ext4 /dev/a rw\n";
    let mounts = parse_mountinfo(raw).unwrap();
    assert_eq!(
        mounts.iter().map(|m| m.target.as_str()).collect::<Vec<_>>(),
        ["/some path", "/some path"]
    );
    assert_eq!(mounts.iter().map(|m| m.id).collect::<Vec<_>>(), [10, 11]);
    assert!(mounts[0].is_readonly());
    assert!(parse_mountinfo("broken").is_err());
    assert!(parse_mountinfo("x 1 8:1 / / rw - ext4 /dev/a rw").is_err());
    assert_eq!(parse_mountinfo("").unwrap(), vec![]);
}

#[test]
fn readonly_remount_keeps_existing_per_mount_flags() {
    let options = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        preserved_mount_flags(&options(&["rw", "relatime"])),
        1 << 21
    );
    assert_eq!(
        preserved_mount_flags(&options(&["rw", "nosuid", "nodev", "noexec", "noatime"])),
        2 | 4 | 8 | 1024
    );
    assert_eq!(
        preserved_mount_flags(&options(&["ro", "nodiratime", "nosymfollow", "unknown"])),
        2048 | 256
    );
}

fn marker(mtime_ns: i128, mode: u32) -> Meta {
    Meta {
        mode,
        uid: 0,
        nlink: 1,
        mtime_ns,
        ..Meta::default()
    }
}

#[test]
fn boot_identity_changes_with_the_hbc_marker_under_fast_boot() {
    let uuid = "2bb46c05-edcf-4649-b8f1-8afe6be375a1\n";
    let a = composite_boot_id(uuid, &marker(1_792_458_000_000_000_000, S_IFREG | 0o644)).unwrap();
    let b = composite_boot_id(uuid, &marker(1_792_459_000_000_000_000, S_IFREG | 0o644)).unwrap();
    assert_ne!(a, b, "same kernel UUID, new HBC start = new boot");
    assert_eq!(
        a,
        composite_boot_id(uuid, &marker(1_792_458_000_000_000_000, S_IFREG | 0o644)).unwrap()
    );
    assert_eq!(
        a,
        "2bb46c05-edcf-4649-b8f1-8afe6be375a1:1792458000000000000"
    );
    assert!(composite_boot_id(uuid, &marker(1, S_IFLNK | 0o644)).is_err());
    assert!(composite_boot_id(
        uuid,
        &Meta {
            uid: 5,
            ..marker(1, S_IFREG | 0o644)
        }
    )
    .is_err());
    assert!(composite_boot_id(uuid, &marker(1, S_IFREG | 0o666)).is_err());
    assert!(composite_boot_id("not-a-uuid", &marker(1, S_IFREG | 0o644)).is_err());
}

#[test]
fn boot_claim_is_once_per_boot() {
    let saved = json_object! {"bootId" => "boot-a", "attempted" => true};
    assert!(boot_already_claimed(Some(&saved), "boot-a"));
    assert!(!boot_already_claimed(Some(&saved), "boot-b"));
    assert!(!boot_already_claimed(None, "boot-a"));
}

struct Probe {
    clock: f64,
    criu: bool,
    preload_ready_at: f64,
    calls: Vec<String>,
}

impl ReadinessProbe for Probe {
    fn now(&self) -> f64 {
        self.clock
    }
    fn sleep(&mut self, seconds: f64) -> Result<()> {
        self.clock += seconds;
        Ok(())
    }
    fn criu_restoring(&self) -> bool {
        self.criu
    }
    fn call(&mut self, method: &str, _timeout: f64) -> Result<()> {
        self.calls.push(method.to_owned());
        if method.contains("preloadmanager") && self.clock < self.preload_ready_at {
            return Err(Error::Timeout("controller still starting".into()));
        }
        Ok(())
    }
}

#[test]
fn readiness_deadline_with_criu_active_makes_no_calls() {
    let mut probe = Probe {
        clock: 0.0,
        criu: true,
        preload_ready_at: 0.0,
        calls: vec![],
    };
    assert!(matches!(
        wait_ready(&mut probe, 8.0),
        Err(Error::Timeout(_))
    ));
    assert!(probe.clock <= 8.0);
    assert!(probe.calls.is_empty());
}

#[test]
fn a_late_preload_controller_is_waited_for_within_the_boot_budget() {
    let mut probe = Probe {
        clock: 0.0,
        criu: false,
        preload_ready_at: 15.0,
        calls: vec![],
    };
    wait_ready(&mut probe, 45.0).unwrap();
    assert!(probe.clock >= 15.0 && probe.clock <= 45.0);
    let mut probe = Probe {
        clock: 0.0,
        criu: false,
        preload_ready_at: 99.0,
        calls: vec![],
    };
    assert!(matches!(
        wait_ready(&mut probe, 450.0),
        Err(Error::Timeout(_))
    ));
    assert!(probe.clock <= 45.0, "budget is clamped to 45 seconds");
}
