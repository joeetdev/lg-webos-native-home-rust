//! Offline, hash-pinned Home delta builder.
//!
//! `home/deltas.json` pins eight exact `sourceSHA256 -> targetSHA256` pairs.
//! Only those firmware bytes are accepted; there is no force mode and no
//! approximate application. All stock hashes are verified before any
//! transform, and all output hashes before any output is created.

use lg_native::json::{self, Style, Value};
use lg_native::sha;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Where the overrides are installed on the TV.
pub const TARGET_ROOT: &str = "/usr/palm/applications/com.webos.app.home/qml/";
/// Where the generated sources are staged on the TV.
pub const STAGING_ROOT: &str = "/var/lib/lg-native/";
/// The production delta specification.
pub const DELTAS: &str = include_str!("../../../home/deltas.json");

/// A builder failure. `Invalid` messages are safe to print; filesystem errors
/// are reported generically so local paths never leak into logs.
#[derive(Debug)]
pub enum BuildError {
    /// The input or specification is not acceptable.
    Invalid(String),
    /// A local filesystem operation failed (details are deliberately not
    /// reported, so private local paths never reach logs).
    Io,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Invalid(message) => f.write_str(message),
            BuildError::Io => f.write_str("local filesystem operation failed"),
        }
    }
}

impl From<std::io::Error> for BuildError {
    fn from(_: std::io::Error) -> Self {
        BuildError::Io
    }
}

type Result<T> = std::result::Result<T, BuildError>;

fn invalid(message: impl Into<String>) -> BuildError {
    BuildError::Invalid(message.into())
}

/// Which on-screen labels the generated Home uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Labels {
    /// English labels ("Search", "Settings", "All apps"), applied after the
    /// pinned output hashes have been verified.
    English,
    /// The byte-exact, physically verified original output (Spanish labels).
    Original,
}

/// One exact full-line label substitution in a generated file.
#[derive(Clone, Copy, Debug)]
pub struct LabelEdit {
    /// Generated file name.
    pub output: &'static str,
    /// The complete original line (must occur exactly once).
    pub from: &'static str,
    /// Its replacement.
    pub to: &'static str,
}

/// The English label layer. Display text only: no identifiers, LaunchPoints,
/// actions or layout values change.
pub const ENGLISH_LABELS: [LabelEdit; 4] = [
    LabelEdit {
        output: "native-MainView.qml",
        from: "            useFixedSize: true; fontSize: styleSheet.getRValue(26); text: \"Buscar\"\n",
        to: "            useFixedSize: true; fontSize: styleSheet.getRValue(26); text: \"Search\"\n",
    },
    LabelEdit {
        output: "native-MainView.qml",
        from: "            useFixedSize: true; fontSize: styleSheet.getRValue(25); text: \"Ajustes\"\n",
        to: "            useFixedSize: true; fontSize: styleSheet.getRValue(25); text: \"Settings\"\n",
    },
    LabelEdit {
        output: "native-MainRibbon.qml",
        from: "                    interfaces.audioguide.readText(\"Todas las apps\", true);\n",
        to: "                    interfaces.audioguide.readText(\"All apps\", true);\n",
    },
    LabelEdit {
        output: "native-MainRibbon.qml",
        from: "                    height: styleSheet.getRValue(26); text: \"Todas las apps\"\n",
        to: "                    height: styleSheet.getRValue(26); text: \"All apps\"\n",
    },
];

/// One validated file entry of the specification.
#[derive(Clone, Debug)]
pub struct FileSpec {
    /// Path of the stock file relative to the stock directory.
    pub stock: String,
    /// Generated file name (`native-<stock name>`).
    pub output: String,
    /// Install target on the TV.
    pub target: String,
    /// Pinned stock hash.
    pub source_sha256: String,
    /// Pinned generated hash (original labels).
    pub target_sha256: String,
    /// Line edits.
    pub edits: Value,
}

/// A path relative to the stock tree: plain components only.
pub fn safe_relative(value: Option<&Value>) -> Result<&str> {
    let text = value.and_then(Value::as_str).filter(|t| !t.is_empty());
    let text = text.ok_or_else(|| invalid("unsafe relative path"))?;
    let safe_part = |part: &str| {
        let mut bytes = part.bytes();
        let first_ok = bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        first_ok
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
            && !part.ends_with('.')
    };
    if text.split('/').all(safe_part) {
        Ok(text)
    } else {
        Err(invalid("unsafe relative path"))
    }
}

/// Validates the specification schema and allowlist.
pub fn validate_spec(specification: &Value) -> Result<Vec<FileSpec>> {
    let object = specification
        .as_object()
        .filter(|s| s.has_exact_keys(&["format", "files"]));
    let format_ok = object
        .and_then(|o| o.get("format"))
        .is_some_and(|f| f.is_integer() && f.as_i64() == Some(1));
    let files = object
        .and_then(|o| o.get("files"))
        .and_then(Value::as_array)
        .filter(|f| !f.is_empty());
    let (true, Some(files)) = (format_ok, files) else {
        return Err(invalid("unsupported delta schema"));
    };
    let mut seen: Vec<(&str, String)> = Vec::new();
    let mut result = Vec::with_capacity(files.len());
    for entry in files {
        let keys = [
            "stock",
            "output",
            "target",
            "sourceSHA256",
            "targetSHA256",
            "edits",
        ];
        let entry = entry.as_object().filter(|e| e.has_exact_keys(&keys));
        let entry = entry.ok_or_else(|| invalid("invalid file entry"))?;
        let stock = safe_relative(entry.get("stock"))?;
        let output = safe_relative(entry.get("output"))?;
        let target = entry
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let stock_name = stock.rsplit('/').next().unwrap_or_default();
        if output.contains('/')
            || output != format!("native-{stock_name}")
            || target != format!("{TARGET_ROOT}{stock}")
        {
            return Err(invalid("output or target outside allowlist"));
        }
        for (key, value) in [("stock", stock), ("output", output), ("target", target)] {
            let identity = (key, value.to_lowercase());
            if seen.contains(&identity) {
                return Err(invalid("duplicate file identity"));
            }
            seen.push(identity);
        }
        let hash = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .filter(|h| sha::is_hex64(h))
                .map(str::to_owned)
        };
        let (Some(source_sha256), Some(target_sha256)) =
            (hash("sourceSHA256"), hash("targetSHA256"))
        else {
            return Err(invalid("invalid SHA256 allowlist"));
        };
        let edits = entry.get("edits").cloned().unwrap_or(Value::Null);
        if edits.as_array().is_none() {
            return Err(invalid("invalid edits"));
        }
        result.push(FileSpec {
            stock: stock.into(),
            output: output.into(),
            target: target.into(),
            source_sha256,
            target_sha256,
            edits,
        });
    }
    Ok(result)
}

/// Splits like Python's `bytes.splitlines(keepends=True)`: at `\r\n`, `\r`
/// and `\n`, keeping the terminators.
pub fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < data.len() {
        let end = match data[i] {
            b'\n' => Some(i + 1),
            b'\r' if data.get(i + 1) == Some(&b'\n') => Some(i + 2),
            b'\r' => Some(i + 1),
            _ => None,
        };
        match end {
            Some(end) => {
                lines.push(&data[start..end]);
                start = end;
                i = end;
            }
            None => i += 1,
        }
    }
    if start < data.len() {
        lines.push(&data[start..]);
    }
    lines
}

/// Applies line edits. Offsets always refer to ORIGINAL lines (zero based),
/// never to the result of earlier edits; edits must be strictly ordered and
/// non-overlapping, and every inserted line is a single LF-terminated (or
/// final, unterminated) line.
pub fn apply_delta(source: &[u8], edits: &Value) -> Result<Vec<u8>> {
    let lines = split_lines(source);
    let edits = edits.as_array().ok_or_else(|| invalid("invalid edits"))?;
    let mut result: Vec<u8> = Vec::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut previous: Option<usize> = None;
    for edit in edits {
        let edit = edit
            .as_object()
            .filter(|e| e.has_exact_keys(&["start", "delete", "lines"]));
        let edit = edit.ok_or_else(|| invalid("invalid edit schema"))?;
        let integer = |key: &str| {
            edit.get(key)
                .filter(|v| v.is_integer())
                .and_then(Value::as_i64)
        };
        let inserted = edit.get("lines").and_then(Value::as_array);
        let (Some(start), Some(count), Some(inserted)) =
            (integer("start"), integer("delete"), inserted)
        else {
            return Err(invalid("invalid, overlapping or out-of-range edit"));
        };
        let in_range = count >= 0
            && start >= cursor as i64
            && previous.map_or(true, |p| start > p as i64)
            && start <= lines.len() as i64
            && start + count <= lines.len() as i64;
        if !in_range {
            return Err(invalid("invalid, overlapping or out-of-range edit"));
        }
        let (start, count) = (start as usize, count as usize);
        for line in inserted {
            let single = line.as_str().is_some_and(|l| {
                let bytes = l.as_bytes();
                !bytes.is_empty()
                    && !bytes.contains(&b'\r')
                    && !bytes[..bytes.len() - 1].contains(&b'\n')
            });
            if !single {
                return Err(invalid("replacement entries must be single LF lines"));
            }
        }
        for line in &lines[cursor..start] {
            result.extend_from_slice(line);
        }
        for line in inserted {
            result.extend_from_slice(line.as_str().unwrap_or_default().as_bytes());
        }
        cursor = start + count;
        previous = Some(start);
    }
    for line in &lines[cursor..] {
        result.extend_from_slice(line);
    }
    Ok(result)
}

/// Applies the English label layer to one generated file.
pub fn apply_labels(output: &str, data: Vec<u8>, labels: &[LabelEdit]) -> Result<Vec<u8>> {
    let mut text = String::from_utf8(data)
        .map_err(|_| invalid(format!("generated file is not UTF-8: {output}")))?;
    for label in labels.iter().filter(|l| l.output == output) {
        if text.matches(label.from).count() != 1 {
            return Err(invalid(format!(
                "label line not found exactly once: {output}"
            )));
        }
        text = text.replacen(label.from, label.to, 1);
    }
    Ok(text.into_bytes())
}

/// `os.path.abspath`: absolute, with `.` and `..` resolved lexically.
fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

#[cfg(windows)]
fn is_link_like(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link_like(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Rejects symlinks (and Windows reparse points) in every path component.
pub fn check_path(path: &Path, regular: bool) -> Result<PathBuf> {
    let path = lexical_absolute(path)?;
    let mut chain: Vec<&Path> = path.ancestors().collect();
    chain.reverse();
    for component in chain {
        match fs::symlink_metadata(component) {
            Ok(metadata) if is_link_like(&metadata) => {
                return Err(invalid("symlink/reparse-point paths are not permitted"))
            }
            Ok(_) => {}
            Err(_) if component == path && !regular => {}
            Err(_) => return Err(invalid("missing input or parent directory")),
        }
    }
    if regular && !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return Err(invalid("input must be a regular file"));
    }
    Ok(path)
}

/// One generated manifest entry.
pub fn manifest_entry(spec: &FileSpec, new_hash: &str) -> Value {
    lg_native::json_object! {
        "source" => format!("{STAGING_ROOT}{}", spec.output),
        "target" => spec.target.as_str(),
        "originalHash" => spec.source_sha256.as_str(),
        "newHash" => new_hash,
    }
}

/// Builds the overrides into a NEW directory `out` and returns the manifest.
pub fn build(
    stock_dir: &Path,
    out: &Path,
    specification: &Value,
    normalize_crlf: bool,
    labels: Labels,
) -> Result<Vec<Value>> {
    let entries = validate_spec(specification)?;
    let stock_dir = check_path(stock_dir, false)?;
    let out = check_path(out, false)?;
    if !stock_dir.is_dir() {
        return Err(invalid("stock directory is missing"));
    }
    if out.exists() {
        return Err(invalid(
            "output must be a new directory; refusing overwrite",
        ));
    }
    if out.starts_with(&stock_dir) {
        return Err(invalid("output must be outside stock directory"));
    }
    // Every stock hash must pass before any transform or output write.
    let mut sources = Vec::with_capacity(entries.len());
    for entry in &entries {
        let path = check_path(&stock_dir.join(&entry.stock), true)?;
        let mut data = fs::read(path)?;
        if normalize_crlf {
            data = replace_crlf(&data);
        }
        if sha::hex(&data) != entry.source_sha256 {
            return Err(invalid(format!("stock hash mismatch: {}", entry.stock)));
        }
        sources.push(data);
    }
    let mut generated = Vec::with_capacity(entries.len());
    let mut manifest = Vec::with_capacity(entries.len());
    for (entry, data) in entries.iter().zip(sources) {
        let mut result = apply_delta(&data, &entry.edits)?;
        if sha::hex(&result) != entry.target_sha256 {
            return Err(invalid(format!("output hash mismatch: {}", entry.output)));
        }
        if labels == Labels::English {
            result = apply_labels(&entry.output, result, &ENGLISH_LABELS)?;
        }
        manifest.push(manifest_entry(entry, &sha::hex(&result)));
        generated.push((entry.output.clone(), result));
    }
    fs::create_dir(&out)?;
    for (name, data) in &generated {
        fs::write(out.join(name), data)?;
    }
    let mut text = json::to_string(&Value::Array(manifest.clone()), Style::Indented, false);
    text.push('\n');
    fs::write(out.join("home-final-manifest.json"), text)?;
    Ok(manifest)
}

/// Explicit, in-memory CRLF -> LF conversion (no BOM removal, lone CR kept).
pub fn replace_crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\r' && data.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// Parses the embedded production specification.
pub fn production_spec() -> Result<Value> {
    let spec = json::parse(DELTAS).map_err(|e| invalid(e.to_string()))?;
    if validate_spec(&spec)?.len() != 8 {
        return Err(invalid(
            "production allowlist must contain exactly eight files",
        ));
    }
    Ok(spec)
}
