//! Content-addressed packaging of the QML/JS assets that run inside the
//! existing compositor.
//!
//! Qt may reuse previously compiled code when a file changes but its URL does
//! not, so every asset is named by its SHA-256, and references are chained
//! (proof -> QML -> loader) BEFORE the referencing file is hashed. The output
//! is byte-identical to the previous Python packers, so existing installations
//! keep their exact asset names.

use lg_native::json::{self, Map, Style, Value};
use lg_native::sha;
use std::fs;
use std::io;
use std::path::Path;

const R: &str = "/var/lib/lg-native";

const GATE_PROOF: &str = include_str!("../../../gate/gate-proof.js");
const GATE_QML: &str = include_str!("../../../gate/native-home-gate.qml");
const GATE_LOADER: &str = include_str!("../../../gate/native-home-gate-loader.js");
const GATE_TRACE: &str = include_str!("../../../gate/gate-trial-trace.qmlpart");
const REMOTE_CORE: &str = include_str!("../../../remote/remote-core.js");
const REMOTE_QML: &str = include_str!("../../../remote/remote-controller.qml");
const REMOTE_LOADER: &str = include_str!("../../../remote/remote-loader.template.js");

/// Package flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Observation build with bounded boot traces (never for permanent use).
    Trial,
    /// Clean production build.
    Final,
}

impl Mode {
    /// Parses `trial` / `final`.
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "trial" => Some(Mode::Trial),
            "final" => Some(Mode::Final),
            _ => None,
        }
    }

    /// The mode name.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Trial => "trial",
            Mode::Final => "final",
        }
    }
}

/// Universal-newline normalisation (`\r\n` and lone `\r` become `\n`), as
/// Python's text mode applied when reading the sources.
pub fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn sha(text: &str) -> String {
    sha::hex(text.as_bytes())
}

/// One generated asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// File name (content addressed).
    pub name: String,
    /// Content.
    pub text: String,
}

/// Builds the gate assets and package for `mode` without touching the disk.
pub fn gate_assets(mode: Mode) -> (Vec<Asset>, Value) {
    let proof = universal_newlines(GATE_PROOF);
    let proof_name = format!("gate-proof-{}.js", sha(&proof));
    let mut qml = universal_newlines(GATE_QML).replace("gate-proof.js", &proof_name);
    if mode == Mode::Trial {
        let trace = universal_newlines(GATE_TRACE);
        qml = qml.replacen(
            "    property int attempts: 0\n",
            &format!("    property int attempts: 0\n{trace}"),
            1,
        );
        qml = qml.replacen(
            "    Component.onCompleted: {\n",
            "    Component.onCompleted: {\n        gate.recordBoot(\"init\", null);\n",
            1,
        );
        qml = qml.replacen(
            "        gate.attempts = 0;\n        gate.refresh();",
            "        gate.attempts = 0;\n        gate.refresh();\n        gate.recordBoot(\"surface\", item);",
            1,
        );
    }
    let qml_name = format!("native-home-gate-{}.qml", sha(&qml));
    let mut loader = universal_newlines(GATE_LOADER).replace(
        &format!("{R}/native-home-gate.qml"),
        &format!("{R}/{qml_name}"),
    );
    if mode == Mode::Trial {
        loader = loader.replacen(
            "component.createObject(root)",
            "component.createObject(root, {traceSend:lunaCall.send})",
            1,
        );
    }
    let loader_name = format!("native-home-gate-loader-{}.js", sha(&loader));
    let assets = vec![
        Asset {
            name: proof_name.clone(),
            text: proof,
        },
        Asset {
            name: qml_name.clone(),
            text: qml,
        },
        Asset {
            name: loader_name.clone(),
            text: loader,
        },
    ];
    let files: Map = assets
        .iter()
        .map(|a| (format!("{R}/{}", a.name), Value::from(sha(&a.text))))
        .collect();
    let package = lg_native::json_object! {
        "version" => 1u64,
        "owner" => "lg-native-home-gate-v1",
        "mode" => mode.as_str(),
        "entry" => lg_native::json_object! {"file" => format!("{R}/{loader_name}"), "handler" => "handleNativeHomeGate"},
        "files" => files,
        "qml" => format!("{R}/{qml_name}"),
        "proof" => format!("{R}/{proof_name}"),
    };
    (assets, package)
}

/// Writes the gate package into `out` (created if missing; existing assets
/// must already hold identical content) and returns the package JSON text.
pub fn pack_gate(mode: Mode, out: &Path) -> io::Result<String> {
    let (assets, package) = gate_assets(mode);
    if !out.is_dir() {
        fs::create_dir(out)?;
    }
    for asset in &assets {
        let path = out.join(&asset.name);
        if path.exists() {
            let existing = universal_newlines(&fs::read_to_string(&path)?);
            if existing != asset.text {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} differs", asset.name),
                ));
            }
        } else {
            fs::write(&path, asset.text.as_bytes())?;
        }
    }
    let text = json::to_string(&package, Style::Indented, false);
    fs::write(out.join("package.json"), &text)?;
    Ok(text)
}

const REMOTE_TRACE: &str = "function remoteTrace(event){var x=new XMLHttpRequest();x.open('POST','http://127.0.0.1:18767/remote');x.setRequestHeader('Content-Type','application/json');x.send(JSON.stringify({time:Date.now(),event:event,blocked:remoteController?remoteController.pointerBlocked:null,power:global.powerState}));}\n";

/// Builds the remote-control assets and package for `mode`.
pub fn remote_assets(mode: Mode) -> (Vec<Asset>, Value) {
    let mut assets = Vec::with_capacity(3);
    let mut add = |prefix: &str, text: String, extension: &str| -> String {
        let name = format!("{prefix}-{}{extension}", sha(&text));
        assets.push(Asset {
            name: name.clone(),
            text,
        });
        format!("{R}/{name}")
    };
    let core = add("remote-core", universal_newlines(REMOTE_CORE), ".js");
    let qml = add("remote-controller", universal_newlines(REMOTE_QML), ".qml");
    let quoted = |url: String| json::compact(&Value::from(url));
    let trial = mode == Mode::Trial;
    let mut loader = universal_newlines(REMOTE_LOADER)
        .replace("__CORE_URL__", &quoted(format!("file://{core}")))
        .replace("__QML_URL__", &quoted(format!("file://{qml}")))
        .replace("__TRIAL_MS__", "0");
    loader = loader.replace(
        "__TRACE_KEYS__",
        if trial {
            "if(key===18874398||key===18874413)remoteTrace('key:'+key+':'+pressed+':'+autoRepeat+':'+deviceId+':model='+!!remoteControls);"
        } else {
            ""
        },
    );
    loader = loader.replace(
        "__TRACE_MOVIES__",
        if trial { "remoteTrace('movies');" } else { "" },
    );
    loader = loader.replace(
        "__TRACE_INIT__",
        if trial {
            "if(typeof remoteController.backend.cursorVisibleChanged.connect!=='function')throw new Error('Cursor notification unavailable');remoteController.pointerBlockedChanged.connect(function(){remoteTrace('pointer');});remoteTrace('ready');"
        } else {
            ""
        },
    );
    if trial {
        loader = format!(
            "{REMOTE_TRACE}{}",
            loader.replace("function init() {", "function init() {try {").replace(
                "\n}\nfunction handleRemoteControls",
                "\n}catch(e){remoteTrace('error:'+String(e));throw e;}\n}\nfunction handleRemoteControls",
            )
        );
    }
    let entry_file = add("remote-loader", loader, ".js");
    let files: Map = assets
        .iter()
        .map(|a| (format!("{R}/{}", a.name), Value::from(sha(&a.text))))
        .collect();
    let package = lg_native::json_object! {
        "version" => 1u64,
        "owner" => "lg-native-remote-controls-v1",
        "mode" => mode.as_str(),
        "entry" => lg_native::json_object! {"file" => entry_file, "handler" => "handleRemoteControls"},
        "files" => files,
    };
    (assets, package)
}

/// Writes the remote package into `out` (created with parents; assets are
/// overwritten) and returns the package JSON text.
pub fn pack_remote(mode: Mode, out: &Path) -> io::Result<String> {
    let (assets, package) = remote_assets(mode);
    fs::create_dir_all(out)?;
    for asset in &assets {
        fs::write(out.join(&asset.name), asset.text.as_bytes())?;
    }
    let text = json::to_string(&package, Style::Indented, false);
    fs::write(out.join("package.json"), &text)?;
    Ok(text)
}
