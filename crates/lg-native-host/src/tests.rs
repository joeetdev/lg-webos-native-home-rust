//! Host tool tests: synthetic fixtures only, never stock firmware.

use crate::build_home::{
    self, apply_delta, apply_labels, build, split_lines, validate_spec, Labels, ENGLISH_LABELS,
};
use crate::pack::{self, Mode};
use lg_native::json::{self, Value};
use lg_native::{json_object, sha};
use std::path::{Path, PathBuf};

fn edits(text: &str) -> Value {
    json::parse(text).unwrap()
}

#[test]
fn offsets_replace_original_lines_and_preserve_untouched_bytes() {
    let result = apply_delta(
        b"alpha\nbeta\ngamma\n",
        &edits(r#"[{"start":1,"delete":1,"lines":["custom\n"]},{"start":3,"delete":0,"lines":["tail"]}]"#),
    )
    .unwrap();
    assert_eq!(result, b"alpha\ncustom\ngamma\ntail");
}

#[test]
fn line_splitting_matches_python_bytes_splitlines() {
    assert_eq!(
        split_lines(b"a\r\nb\rc\nd"),
        vec![&b"a\r\n"[..], b"b\r", b"c\n", b"d"]
    );
    assert_eq!(split_lines(b""), Vec::<&[u8]>::new());
    assert_eq!(split_lines(b"\n\n"), vec![&b"\n"[..], b"\n"]);
}

struct Fixture {
    root: PathBuf,
    stock: PathBuf,
    out: PathBuf,
    entry: Value,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("lg-native-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let stock = root.join("stock");
        std::fs::create_dir_all(&stock).unwrap();
        std::fs::write(stock.join("part.qml"), b"stock\n").unwrap();
        let entry = json_object! {
            "stock" => "part.qml", "output" => "native-part.qml",
            "target" => "/usr/palm/applications/com.webos.app.home/qml/part.qml",
            "sourceSHA256" => sha::hex(b"stock\n"), "targetSHA256" => sha::hex(b"custom\n"),
            "edits" => edits(r#"[{"start":0,"delete":1,"lines":["custom\n"]}]"#),
        };
        Fixture {
            out: root.join("output"),
            root,
            stock,
            entry,
        }
    }

    fn spec(&self, files: Vec<Value>) -> Value {
        json_object! {"format" => 1u64, "files" => Value::Array(files)}
    }

    fn build(&self, files: Vec<Value>, crlf: bool) -> Result<Vec<Value>, build_home::BuildError> {
        build(
            &self.stock,
            &self.out,
            &self.spec(files),
            crlf,
            Labels::Original,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value.as_object_mut().unwrap().insert(key, field);
    value
}

#[test]
fn build_emits_exact_bytes_and_a_manual_staging_manifest() {
    let f = Fixture::new("emit");
    f.build(vec![f.entry.clone()], false).unwrap();
    assert_eq!(
        std::fs::read(f.out.join("native-part.qml")).unwrap(),
        b"custom\n"
    );
    let text = std::fs::read_to_string(f.out.join("home-final-manifest.json")).unwrap();
    assert_eq!(
        json::parse(&text).unwrap(),
        Value::Array(vec![json_object! {
            "source" => "/var/lib/lg-native/native-part.qml",
            "target" => "/usr/palm/applications/com.webos.app.home/qml/part.qml",
            "originalHash" => sha::hex(b"stock\n"),
            "newHash" => sha::hex(b"custom\n"),
        }])
    );
    assert!(
        text.starts_with("[\n  {\n    \"source\": ") && text.ends_with("}\n]\n"),
        "Python indent=2 layout"
    );
}

#[test]
fn unsafe_schema_is_rejected_before_any_output_exists() {
    let f = Fixture::new("schema");
    let cases: Vec<(&str, Value)> = vec![
        ("stock", Value::from("../part.qml")),
        ("stock", Value::from("/part.qml")),
        ("stock", Value::from("C:/part.qml")),
        ("stock", Value::from("a\\part.qml")),
        ("output", Value::from("../native-part.qml")),
        ("output", Value::from("part.qml")),
        ("target", Value::from("/etc/config")),
        ("sourceSHA256", Value::from("invalid")),
        ("targetSHA256", Value::from("0".repeat(64))),
        ("edits", edits(r#"[{"start":-1,"delete":0,"lines":[]}]"#)),
        ("edits", edits(r#"[{"start":0,"delete":9,"lines":[]}]"#)),
        ("edits", edits(r#"[{"start":true,"delete":0,"lines":[]}]"#)),
        ("edits", edits(r#"[{"start":0.0,"delete":0,"lines":[]}]"#)),
        (
            "edits",
            edits(r#"[{"start":0,"delete":1,"lines":"custom\n"}]"#),
        ),
        (
            "edits",
            edits(r#"[{"start":0,"delete":1,"lines":["a\nb\n"]}]"#),
        ),
        (
            "edits",
            edits(r#"[{"start":0,"delete":1,"lines":["a\r\n"]}]"#),
        ),
        ("edits", edits(r#"[{"start":0,"delete":1,"lines":[""]}]"#)),
        (
            "edits",
            edits(r#"[{"start":1,"delete":0,"lines":[]},{"start":0,"delete":0,"lines":[]}]"#),
        ),
        (
            "edits",
            edits(r#"[{"start":0,"delete":0,"lines":[]},{"start":0,"delete":0,"lines":[]}]"#),
        ),
    ];
    for (key, value) in cases {
        let entry = with(f.entry.clone(), key, value.clone());
        assert!(
            matches!(
                f.build(vec![entry], false),
                Err(build_home::BuildError::Invalid(_))
            ),
            "{key} {}",
            json::compact(&value)
        );
        assert!(!f.out.exists());
    }
    for spec in [
        json_object! {"format" => 2u64, "files" => Value::Array(vec![f.entry.clone()])},
        json_object! {"format" => 1u64, "files" => Value::Array(vec![])},
        json_object! {"format" => 1u64, "files" => Value::Array(vec![f.entry.clone(), f.entry.clone()])},
    ] {
        assert!(build(&f.stock, &f.out, &spec, false, Labels::Original).is_err());
        assert!(!f.out.exists());
    }
}

#[test]
fn every_stock_hash_is_checked_before_any_transform() {
    let f = Fixture::new("order");
    let second = json_object! {
        "stock" => "second.qml", "output" => "native-second.qml",
        "target" => "/usr/palm/applications/com.webos.app.home/qml/second.qml",
        "sourceSHA256" => sha::hex(b"stock\n"), "targetSHA256" => sha::hex(b"custom\n"),
        "edits" => edits(r#"[{"start":0,"delete":1,"lines":["custom\n"]}]"#),
    };
    std::fs::write(f.stock.join("second.qml"), b"wrong firmware\n").unwrap();
    let first = with(
        f.entry.clone(),
        "edits",
        edits(r#"[{"start":99,"delete":0,"lines":[]}]"#),
    );
    let error = f.build(vec![first, second], false).unwrap_err();
    assert_eq!(error.to_string(), "stock hash mismatch: second.qml");
    assert!(!f.out.exists());
}

#[test]
fn crlf_conversion_is_explicit_opt_in() {
    let f = Fixture::new("crlf");
    std::fs::write(f.stock.join("part.qml"), b"stock\r\n").unwrap();
    assert!(f
        .build(vec![f.entry.clone()], false)
        .unwrap_err()
        .to_string()
        .starts_with("stock hash mismatch"));
    assert!(!f.out.exists());
    f.build(vec![f.entry.clone()], true).unwrap();
    assert_eq!(
        std::fs::read(f.out.join("native-part.qml")).unwrap(),
        b"custom\n"
    );
}

#[test]
fn missing_inputs_and_existing_outputs_are_refused() {
    let f = Fixture::new("missing");
    std::fs::remove_file(f.stock.join("part.qml")).unwrap();
    assert!(f.build(vec![f.entry.clone()], false).is_err());
    assert!(!f.out.exists());
    std::fs::write(f.stock.join("part.qml"), b"stock\n").unwrap();
    std::fs::create_dir(&f.out).unwrap();
    let error = f.build(vec![f.entry.clone()], false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "output must be a new directory; refusing overwrite"
    );
    std::fs::remove_dir(&f.out).unwrap();
    let inside = build(
        &f.stock,
        &f.stock.join("out"),
        &f.spec(vec![f.entry.clone()]),
        false,
        Labels::Original,
    );
    assert_eq!(
        inside.unwrap_err().to_string(),
        "output must be outside stock directory"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_input_is_refused_even_when_its_hash_matches() {
    let f = Fixture::new("symlink");
    let real = f.root.join("real.qml");
    std::fs::write(&real, b"stock\n").unwrap();
    std::fs::remove_file(f.stock.join("part.qml")).unwrap();
    std::os::unix::fs::symlink(&real, f.stock.join("part.qml")).unwrap();
    assert!(f.build(vec![f.entry.clone()], false).is_err());
    assert!(!f.out.exists());
}

#[test]
fn production_spec_is_the_exact_eight_file_allowlist() {
    let spec = build_home::production_spec().unwrap();
    let files = validate_spec(&spec).unwrap();
    assert_eq!(files.len(), 8);
    let targets: Vec<&str> = files.iter().map(|f| f.target.as_str()).collect();
    for target in lg_native::home::model::ALLOWED {
        assert!(targets.contains(&target), "{target}");
    }
}

#[test]
fn every_english_label_replaces_exactly_one_inserted_line_of_the_real_deltas() {
    let spec = build_home::production_spec().unwrap();
    let files = validate_spec(&spec).unwrap();
    for label in ENGLISH_LABELS {
        let file = files.iter().find(|f| f.output == label.output).unwrap();
        let count = file
            .edits
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|e| {
                e.get("lines")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .filter(|l| l.as_str() == Some(label.from))
            .count();
        assert_eq!(count, 1, "{}", label.from);
        // Only the quoted display text differs: identical code before and after it.
        let outside = |line: &'static str| (line.split('"').next(), line.rsplit('"').next());
        assert_eq!(outside(label.from), outside(label.to), "{}", label.to);
        assert_eq!(label.from.matches('"').count(), 2);
    }
}

#[test]
fn english_labels_are_applied_once_and_fail_closed() {
    let original = format!(
        "head\n{}{}tail\n",
        ENGLISH_LABELS[0].from, ENGLISH_LABELS[1].from
    );
    let labelled = apply_labels(
        "native-MainView.qml",
        original.clone().into_bytes(),
        &ENGLISH_LABELS,
    )
    .unwrap();
    let text = String::from_utf8(labelled).unwrap();
    assert!(text.contains("text: \"Search\"") && text.contains("text: \"Settings\""));
    assert!(!text.contains("Buscar") && !text.contains("Ajustes"));
    assert!(apply_labels(
        "native-MainView.qml",
        b"no labels here\n".to_vec(),
        &ENGLISH_LABELS
    )
    .is_err());
    let doubled = format!("{0}{0}{1}", ENGLISH_LABELS[0].from, ENGLISH_LABELS[1].from);
    assert!(apply_labels("native-MainView.qml", doubled.into_bytes(), &ENGLISH_LABELS).is_err());
    assert_eq!(
        apply_labels("native-ShelfList.js", b"x\n".to_vec(), &ENGLISH_LABELS).unwrap(),
        b"x\n"
    );
}

#[test]
fn english_build_verifies_the_pinned_output_before_relabelling() {
    let f = Fixture::new("labels");
    let main_view = json_object! {
        "stock" => "MainView.qml", "output" => "native-MainView.qml",
        "target" => "/usr/palm/applications/com.webos.app.home/qml/MainView.qml",
        "sourceSHA256" => sha::hex(b"stock\n"),
        "targetSHA256" => sha::hex(format!("{}{}", ENGLISH_LABELS[0].from, ENGLISH_LABELS[1].from)),
        "edits" => Value::Array(vec![json_object! {
            "start" => 0u64, "delete" => 1u64,
            "lines" => Value::Array(vec![Value::from(ENGLISH_LABELS[0].from), Value::from(ENGLISH_LABELS[1].from)]),
        }]),
    };
    std::fs::write(f.stock.join("MainView.qml"), b"stock\n").unwrap();
    let manifest = build(
        &f.stock,
        &f.out,
        &f.spec(vec![main_view.clone()]),
        false,
        Labels::English,
    )
    .unwrap();
    let generated = std::fs::read(f.out.join("native-MainView.qml")).unwrap();
    assert_eq!(
        manifest[0].get("newHash").and_then(Value::as_str),
        Some(sha::hex(&generated).as_str())
    );
    assert!(String::from_utf8_lossy(&generated).contains("\"Settings\""));
    std::fs::remove_dir_all(&f.out).unwrap();
    // A pinned-hash mismatch stops the build before any label is applied.
    let wrong = with(main_view, "targetSHA256", Value::from(sha::hex("other")));
    let error = build(
        &f.stock,
        &f.out,
        &f.spec(vec![wrong]),
        false,
        Labels::English,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "output hash mismatch: native-MainView.qml"
    );
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lg-native-pack-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read_package(dir: &Path) -> Value {
    json::parse(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

#[test]
fn final_gate_bundle_is_content_addressed_and_trace_free() {
    let dir = scratch("gate-final");
    let out = dir.join("packed-final");
    let printed = pack::pack_gate(Mode::Final, &out).unwrap();
    let package = read_package(&out);
    assert_eq!(json::parse(&printed).unwrap(), package);
    assert_eq!(
        package
            .get("entry")
            .and_then(|e| e.get("handler"))
            .and_then(Value::as_str),
        Some("handleNativeHomeGate")
    );
    let files = package.get("files").and_then(Value::as_object).unwrap();
    assert_eq!(files.len(), 3);
    for (path, digest) in files.iter() {
        let data = std::fs::read(out.join(file_name(path))).unwrap();
        assert_eq!(Some(sha::hex(&data).as_str()), digest.as_str());
        assert!(path.contains(digest.as_str().unwrap()));
    }
    let qml_path = package.get("qml").and_then(Value::as_str).unwrap();
    let qml = std::fs::read_to_string(out.join(file_name(qml_path))).unwrap();
    let loader_path = package
        .get("entry")
        .and_then(|e| e.get("file"))
        .and_then(Value::as_str)
        .unwrap();
    let loader = std::fs::read_to_string(out.join(file_name(loader_path))).unwrap();
    assert!(qml.contains(file_name(
        package.get("proof").and_then(Value::as_str).unwrap()
    )));
    assert!(loader.contains(qml_path));
    for text in [&qml, &loader] {
        assert!(!text.contains("127.0.0.1") && !text.contains("bootEvents"));
    }
    // Repacking is idempotent; a tampered asset is refused, never overwritten.
    pack::pack_gate(Mode::Final, &out).unwrap();
    std::fs::write(out.join(file_name(qml_path)), "tampered").unwrap();
    assert!(pack::pack_gate(Mode::Final, &out).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn trial_gate_trace_is_bounded_and_uses_immutable_sources() {
    let (assets, package) = pack::gate_assets(Mode::Trial);
    let qml = &assets[1].text;
    let loader = &assets[2].text;
    assert!(
        qml.contains("bootEvents")
            && qml.contains("127.0.0.1:18766/boot")
            && qml.contains("publishAttempts < 150")
    );
    assert!(loader.contains("traceSend:lunaCall.send"));
    assert!(
        qml.contains("gate.recordBoot(\"init\", null)")
            && qml.contains("gate.recordBoot(\"surface\", item)")
    );
    for asset in &assets {
        let digest = sha::hex(&asset.text);
        assert!(asset.name.contains(&digest));
        assert!(package
            .get("files")
            .and_then(|f| f.get(&format!("/var/lib/lg-native/{}", asset.name)))
            .is_some());
    }
}

#[test]
fn remote_bundles_have_immutable_dependencies_and_final_has_no_trace() {
    let dir = scratch("remote");
    let trial = pack::pack_remote(Mode::Trial, &dir.join("trial")).unwrap();
    let final_text = pack::pack_remote(Mode::Final, &dir.join("final")).unwrap();
    let (a, b) = (
        json::parse(&trial).unwrap(),
        json::parse(&final_text).unwrap(),
    );
    for (package, sub) in [(&a, "trial"), (&b, "final")] {
        let files = package.get("files").and_then(Value::as_object).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(
            package
                .get("entry")
                .and_then(|e| e.get("handler"))
                .and_then(Value::as_str),
            Some("handleRemoteControls")
        );
        for (path, digest) in files.iter() {
            let data = std::fs::read(dir.join(sub).join(file_name(path))).unwrap();
            assert_eq!(Some(sha::hex(&data).as_str()), digest.as_str());
        }
        lg_native::remote::validate_package(package)
            .map_or_else(|_| assert_eq!(sub, "trial"), |()| assert_eq!(sub, "final"));
    }
    assert_ne!(a.get("entry"), b.get("entry"));
    let loader = |p: &Value, sub: &str| {
        let path = p
            .get("entry")
            .and_then(|e| e.get("file"))
            .and_then(Value::as_str)
            .unwrap();
        std::fs::read_to_string(dir.join(sub).join(file_name(path))).unwrap()
    };
    assert!(loader(&a, "trial").contains("trialMilliseconds = 0"));
    let final_loader = loader(&b, "final");
    assert!(
        !final_loader.contains("127.0.0.1")
            && !final_loader.contains("__")
            && final_loader.contains("trialMilliseconds = 0")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn universal_newlines_match_python_text_mode() {
    assert_eq!(pack::universal_newlines("a\r\nb\rc\n"), "a\nb\nc\n");
}

#[test]
fn overflowing_edit_offsets_are_rejected_not_wrapped() {
    for text in [
        r#"[{"start":9223372036854775807,"delete":9223372036854775807,"lines":[]}]"#,
        r#"[{"start":1,"delete":9223372036854775807,"lines":[]}]"#,
        r#"[{"start":0,"delete":18446744073709551615,"lines":[]}]"#,
    ] {
        let result = apply_delta(b"a\nb\n", &edits(text));
        assert!(
            matches!(result, Err(build_home::BuildError::Invalid(_))),
            "{text}"
        );
    }
}

#[test]
fn every_publication_pattern_compiles() {
    assert_eq!(
        crate::security::patterns().len(),
        crate::security::PATTERN_COUNT
    );
}
