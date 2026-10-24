//! Remote-control key-filter ownership.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lg_native::json::{self, Value};
use lg_native::remote::{
    self, build_chain, check, disable, install, remove_owned, validate_package, Command, Io, OWNER,
    ROOT, STATE,
};
use lg_native::{json_object, sha, Error, Result};

fn package(seed: &str) -> Value {
    let mut files = lg_native::json::Map::new();
    let mut loader = String::new();
    for (kind, ext) in [("core", "js"), ("controller", "qml"), ("loader", "js")] {
        let digest = sha::hex(format!("{seed}{kind}"));
        let path = format!("{ROOT}/remote-{kind}-{digest}.{ext}");
        if kind == "loader" {
            loader = path.clone();
        }
        files.insert(path, digest);
    }
    json_object! {
        "owner" => OWNER, "version" => 1u64, "mode" => "final", "files" => files,
        "entry" => json_object! {"file" => loader, "handler" => "handleRemoteControls"},
    }
}

fn entry(p: &Value) -> Value {
    p.get("entry").cloned().unwrap()
}

fn chain() -> Vec<Value> {
    [
        "handleSafety",
        "handleAccessibility",
        "handleHomeGate",
        "handleCPHotkeys",
        "handleSystemKeys",
    ]
    .iter()
    .map(|h| json_object! {"file" => "/native.js", "handler" => *h})
    .collect()
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value.as_object_mut().unwrap().insert(key, field);
    value
}

#[test]
fn insert_preserves_foreign_order_and_the_home_gate() {
    let p = package("one");
    let e = entry(&p);
    let result = build_chain(&Value::Array(chain()), &e, &[]).unwrap();
    let mut expected = chain()[..3].to_vec();
    expected.push(e.clone());
    expected.extend(chain()[3..].iter().cloned());
    assert_eq!(result, expected);
    assert_eq!(
        build_chain(&Value::Array(result.clone()), &e, std::slice::from_ref(&e)).unwrap(),
        result
    );
}

#[test]
fn only_exact_owned_entries_are_removed() {
    let e = entry(&package("one"));
    let foreign = with(e.clone(), "extra", Value::Bool(true));
    let mut before = chain();
    before.extend([e.clone(), foreign.clone(), e.clone()]);
    let mut expected = chain();
    expected.push(foreign);
    assert_eq!(remove_owned(&Value::Array(before), &[e]).unwrap(), expected);
}

#[test]
fn ambiguous_anchor_or_unowned_remote_handler_is_refused() {
    let e = entry(&package("one"));
    let mut doubled = chain();
    doubled.push(chain()[3].clone());
    let mut unowned = chain();
    unowned.push(e.clone());
    for before in [vec![], doubled, unowned] {
        assert!(matches!(
            build_chain(&Value::Array(before), &e, &[]),
            Err(Error::Policy(_))
        ));
    }
    assert!(
        build_chain(&Value::Array(vec![Value::from("x")]), &e, &[]).is_err(),
        "non-object entries"
    );
}

#[test]
fn package_rejects_trial_paths_counts_hashes_and_entries() {
    assert!(validate_package(&package("one")).is_ok());
    let p = package("one");
    let files = p.get("files").and_then(Value::as_object).cloned().unwrap();
    let first = files.keys().next().unwrap().to_owned();
    let mut fewer = files.clone();
    fewer.remove(&first);
    let mut foreign_path = files.clone();
    foreign_path.insert("/tmp/remote-core-x.js", sha::hex("x"));
    let mut wrong_digest = files.clone();
    wrong_digest.insert(first.clone(), "0".repeat(64));
    let mut wrong_extension = lg_native::json::Map::new();
    for (path, digest) in files.iter() {
        wrong_extension.insert(path.replace(".qml", ".js"), digest.clone());
    }
    for bad in [
        with(p.clone(), "mode", Value::from("trial")),
        with(p.clone(), "version", Value::Bool(true)),
        with(p.clone(), "files", Value::Object(fewer)),
        with(
            p.clone(),
            "entry",
            with(entry(&p), "handler", Value::from("other")),
        ),
        with(p.clone(), "files", Value::Object(foreign_path)),
        with(p.clone(), "files", Value::Object(wrong_digest)),
        with(p.clone(), "files", Value::Object(wrong_extension)),
        with(p.clone(), "extra", Value::Null),
    ] {
        assert!(
            matches!(validate_package(&bad), Err(Error::Policy(_))),
            "{}",
            json::compact(&bad)
        );
    }
}

struct FakeIo {
    live: Vec<Value>,
    state: Option<Value>,
    pkg: Value,
    seed: String,
    events: Vec<&'static str>,
    fail: Option<&'static str>,
    bad_readback: bool,
    filters_error: Option<fn() -> Error>,
}

impl FakeIo {
    fn new() -> Self {
        FakeIo {
            live: chain(),
            state: None,
            pkg: package("one"),
            seed: "one".into(),
            events: vec![],
            fail: None,
            bad_readback: false,
            filters_error: None,
        }
    }
}

impl Io for FakeIo {
    fn read_json(&mut self, path: &str, optional: bool) -> Result<Option<Value>> {
        if path == STATE {
            let _ = optional;
            Ok(self.state.clone())
        } else {
            Ok(Some(self.pkg.clone()))
        }
    }
    fn read_private(&mut self, path: &str) -> Result<Vec<u8>> {
        let (kind, _, _) = remote::parse_asset_path(path).unwrap();
        Ok(format!("{}{kind}", self.seed).into_bytes())
    }
    fn atomic_json(&mut self, _path: &str, value: &Value) -> Result<()> {
        self.events.push("state");
        if self.fail == Some("state") {
            return Err(Error::io(
                "write state",
                std::io::Error::other("injected state failure"),
            ));
        }
        self.state = Some(value.clone());
        Ok(())
    }
    fn get_filters(&mut self) -> Result<Value> {
        if let Some(error) = self.filters_error {
            return Err(error());
        }
        Ok(Value::Array(self.live.clone()))
    }
    fn set_filters(&mut self, chain: &[Value]) -> Result<()> {
        self.events.push("set");
        if self.fail == Some("set") {
            return Err(Error::io(
                "set filters",
                std::io::Error::other("injected config failure"),
            ));
        }
        self.live = chain.to_vec();
        if self.bad_readback {
            self.live
                .push(json_object! {"handler" => "foreignConcurrent"});
        }
        Ok(())
    }
}

fn owned(io: &FakeIo) -> Vec<Value> {
    io.state
        .as_ref()
        .and_then(|s| s.get("owned"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn install_records_ownership_before_the_write_and_a_repeat_changes_nothing() {
    let mut io = FakeIo::new();
    install(&mut io, "/package.json").unwrap();
    assert_eq!(io.events, ["state", "set"]);
    assert_eq!(owned(&io), vec![entry(&io.pkg)]);
    let mut expected = chain()[..3].to_vec();
    expected.push(entry(&io.pkg));
    expected.extend(chain()[3..].iter().cloned());
    assert_eq!(io.live, expected);
    let again = install(&mut io, "/package.json").unwrap();
    assert_eq!(io.events, ["state", "set"]);
    assert_eq!(again.get("changed"), Some(&Value::Bool(false)));
    assert!(check(&mut io).is_ok());
}

#[test]
fn a_state_failure_prevents_the_config_write() {
    let mut io = FakeIo::new();
    io.fail = Some("state");
    assert!(matches!(
        install(&mut io, "/package.json"),
        Err(Error::Io { .. })
    ));
    assert_eq!(io.live, chain());
    assert!(!io.events.contains(&"set"));
}

#[test]
fn a_config_failure_retains_recoverable_ownership() {
    let mut io = FakeIo::new();
    io.fail = Some("set");
    assert!(install(&mut io, "/package.json").is_err());
    assert_eq!(owned(&io), vec![entry(&io.pkg)]);
    io.fail = None;
    disable(&mut io).unwrap();
    assert_eq!(io.live, chain());
}

#[test]
fn a_bad_readback_is_a_failure_not_a_blanket_restore() {
    let mut io = FakeIo::new();
    io.bad_readback = true;
    assert!(matches!(
        install(&mut io, "/package.json"),
        Err(Error::Policy(_))
    ));
    assert!(io.state.is_some());
    assert_eq!(
        io.live.last(),
        Some(&json_object! {"handler" => "foreignConcurrent"})
    );
}

#[test]
fn swapping_packages_journals_both_entries_and_disable_keeps_live_foreign_entries() {
    let mut io = FakeIo::new();
    install(&mut io, "/package.json").unwrap();
    let old = entry(&io.pkg);
    io.pkg = package("two");
    io.seed = "two".into();
    install(&mut io, "/package.json").unwrap();
    assert_eq!(owned(&io), vec![old.clone(), entry(&io.pkg)]);
    let extra = with(old, "foreign", Value::Bool(true));
    io.live
        .extend([extra.clone(), json_object! {"handler" => "newForeign"}]);
    disable(&mut io).unwrap();
    let mut expected = chain();
    expected.extend([extra, json_object! {"handler" => "newForeign"}]);
    assert_eq!(io.live, expected);
    let writes = io.events.iter().filter(|e| **e == "set").count();
    disable(&mut io).unwrap();
    assert_eq!(io.events.iter().filter(|e| **e == "set").count(), writes);
}

#[test]
fn disable_without_state_cannot_claim_ownership() {
    let mut io = FakeIo::new();
    io.live.push(entry(&io.pkg));
    let before = io.live.clone();
    let result = disable(&mut io).unwrap();
    assert_eq!(
        result.get("reason"),
        Some(&Value::from("no ownership state"))
    );
    assert_eq!(io.live, before);
    assert!(io.events.is_empty());
}

#[test]
fn a_hash_failure_prevents_journal_and_config() {
    let mut io = FakeIo::new();
    io.seed = "tampered".into();
    assert!(matches!(
        install(&mut io, "/package.json"),
        Err(Error::Policy(_))
    ));
    assert!(io.events.is_empty());
}

#[test]
fn check_rejects_missing_duplicate_other_remote_and_corruption() {
    let mut io = FakeIo::new();
    install(&mut io, "/package.json").unwrap();
    let good = io.live.clone();
    let mut duplicate = good.clone();
    duplicate.push(entry(&io.pkg));
    let mut other = good.clone();
    other.push(entry(&package("two")));
    for bad in [chain(), duplicate, other] {
        io.live = bad;
        assert!(matches!(check(&mut io), Err(Error::Policy(_))));
    }
    io.live = good;
    io.seed = "corrupt".into();
    assert!(matches!(check(&mut io), Err(Error::Policy(_))));
}

#[test]
fn failures_never_print_service_output_or_exception_detail() {
    let mut io = FakeIo::new();
    io.filters_error = Some(|| Error::Timeout("secret luna-send output".into()));
    let mut out = Vec::new();
    assert_eq!(
        remote::run(&Command::Install("/package.json".into()), Ok(io), &mut out),
        1
    );
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(!text.contains("secret"));
    assert_eq!(
        text,
        "{\"ok\": false, \"error\": \"I/O or data failure; ownership retained\"}\n"
    );
}

#[test]
fn command_line_dispatches_and_reports_without_a_device() {
    let mut io = FakeIo::new();
    let mut out = Vec::new();
    // Drive the commands through the same state by reusing one fake.
    struct Shared<'a>(&'a mut FakeIo);
    impl Io for Shared<'_> {
        fn read_json(&mut self, p: &str, o: bool) -> Result<Option<Value>> {
            self.0.read_json(p, o)
        }
        fn read_private(&mut self, p: &str) -> Result<Vec<u8>> {
            self.0.read_private(p)
        }
        fn atomic_json(&mut self, p: &str, v: &Value) -> Result<()> {
            self.0.atomic_json(p, v)
        }
        fn get_filters(&mut self) -> Result<Value> {
            self.0.get_filters()
        }
        fn set_filters(&mut self, c: &[Value]) -> Result<()> {
            self.0.set_filters(c)
        }
    }
    assert_eq!(
        remote::run(
            &Command::Install("/package.json".into()),
            Ok(Shared(&mut io)),
            &mut out
        ),
        0
    );
    assert_eq!(
        remote::run(&Command::Check, Ok(Shared(&mut io)), &mut out),
        0
    );
    assert_eq!(
        remote::run(&Command::Disable, Ok(Shared(&mut io)), &mut out),
        0
    );
    assert_eq!(
        remote::run(&Command::Check, Ok(Shared(&mut io)), &mut out),
        1
    );
    let lines: Vec<String> = String::from_utf8_lossy(&out)
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        lines[0],
        "{\"changed\": true, \"filters\": 6, \"ok\": true}"
    );
    assert_eq!(lines[1], "{\"filters\": 6, \"ok\": true}");
    assert_eq!(
        lines[3],
        "{\"ok\": false, \"error\": \"Expected exactly one current remote entry\"}"
    );
    let refused: Result<FakeIo> = Err(Error::Policy("Linux root required".into()));
    let mut out = Vec::new();
    assert_eq!(remote::run(&Command::Check, refused, &mut out), 1);
    assert!(String::from_utf8_lossy(&out).contains("Linux root required"));
}

#[cfg(unix)]
#[test]
fn metadata_rules_reject_symlinks_non_root_open_permissions_and_oversize() {
    use lg_native::fsx::{Meta, S_IFDIR, S_IFLNK, S_IFREG};
    use lg_native::remote::validate_metadata;
    let file = |mode: u32, uid: u32, size: u64| Meta {
        mode,
        uid,
        size,
        nlink: 1,
        ..Meta::default()
    };
    assert!(validate_metadata(&file(S_IFREG | 0o600, 0, 10), false).is_ok());
    for bad in [
        file(S_IFLNK | 0o600, 0, 10),
        file(S_IFREG | 0o600, 1, 10),
        file(S_IFREG | 0o644, 0, 10),
        file(S_IFREG | 0o600, 0, remote::LIMIT + 1),
        file(S_IFREG | 0o600, 0, 0),
        file(S_IFDIR | 0o700, 0, 10),
    ] {
        assert!(
            matches!(validate_metadata(&bad, false), Err(Error::Policy(_))),
            "{bad:?}"
        );
    }
    assert!(validate_metadata(&file(S_IFDIR | 0o755, 0, 10), true).is_ok());
    assert!(validate_metadata(&file(S_IFDIR | 0o777, 0, 10), true).is_err());
}

#[test]
fn state_round_trip_requires_exact_identity() {
    let mut io = FakeIo::new();
    install(&mut io, "/package.json").unwrap();
    let state = io.state.clone().unwrap();
    assert_eq!(
        json::to_string(&state, json::Style::Compact, true)
            .chars()
            .next(),
        Some('{')
    );
    for bad in [
        with(state.clone(), "owner", Value::from("foreign")),
        with(state.clone(), "version", Value::from(2u64)),
        with(state.clone(), "owned", Value::Array(vec![])),
        with(
            state.clone(),
            "owned",
            Value::Array(vec![entry(&package("two"))]),
        ),
        with(state.clone(), "extra", Value::Null),
    ] {
        io.state = Some(bad);
        assert!(matches!(check(&mut io), Err(Error::Policy(_))));
    }
}
