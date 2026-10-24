//! One-shot, persistent ownership of the remote-control key filter. No daemon
//! and no boot hook: the compositor loads the filter itself.
//!
//! Error output never contains service responses, subprocess output, paths or
//! arbitrary exception text — only this module's own policy messages.

use crate::error::{Error, Result};
use crate::json::{self, Map, Style, Value};

/// Private root.
pub const ROOT: &str = "/var/lib/lg-native";
/// Ownership identity.
pub const OWNER: &str = "lg-native-remote-controls-v1";
/// Ownership state file.
pub const STATE: &str = "/var/lib/lg-native/remote-controls-state.json";
/// Compositor key-filter configuration.
pub const KEY: &str = "com.webos.surfacemanager.keyFilters";
/// Handler exported by the loader.
pub const HANDLER: &str = "handleRemoteControls";
/// Size cap for files and service responses.
pub const LIMIT: u64 = 1024 * 1024;
/// Most generations of owned entries kept in the state.
pub const MAX_OWNED: usize = 128;

fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Policy(message.to_owned()))
    }
}

/// Parses `/var/lib/lg-native/remote-(core|controller|loader)-<sha256>.(js|qml)`.
pub fn parse_asset_path(path: &str) -> Option<(&str, &str, &str)> {
    let rest = path.strip_prefix("/var/lib/lg-native/remote-")?;
    let (stem, extension) = rest.rsplit_once('.')?;
    let (kind, digest) = stem.split_once('-')?;
    let valid = matches!(kind, "core" | "controller" | "loader")
        && crate::sha::is_hex64(digest)
        && matches!(extension, "js" | "qml");
    valid.then_some((kind, digest, extension))
}

/// An owned entry: exactly `{file: <loader .js>, handler: "handleRemoteControls"}`.
pub fn valid_entry(entry: &Value) -> Result<()> {
    let object = entry
        .as_object()
        .filter(|e| e.has_exact_keys(&["file", "handler"]));
    let object = object.ok_or_else(|| Error::Policy("Invalid owned entry".into()))?;
    let file = object.get("file").and_then(Value::as_str);
    require(
        object.get("handler").and_then(Value::as_str) == Some(HANDLER) && file.is_some(),
        "Invalid owned handler",
    )?;
    let parsed = file.and_then(parse_asset_path);
    require(
        parsed.is_some_and(|(kind, _, ext)| kind == "loader" && ext == "js"),
        "Invalid loader path",
    )
}

/// Validates a final, version-1 package with three content-addressed files.
pub fn validate_package(package: &Value) -> Result<()> {
    let object = package
        .as_object()
        .filter(|p| p.has_exact_keys(&["owner", "version", "mode", "entry", "files"]));
    let object = object.ok_or_else(|| Error::Policy("Invalid package shape".into()))?;
    let version_ok = object
        .get("version")
        .is_some_and(|v| v.is_integer() && v.as_i64() == Some(1));
    require(
        object.get("owner").and_then(Value::as_str) == Some(OWNER)
            && version_ok
            && object.get("mode").and_then(Value::as_str) == Some("final"),
        "Only final version 1 packages accepted",
    )?;
    let entry = object.get("entry").cloned().unwrap_or(Value::Null);
    valid_entry(&entry)?;
    let files = object
        .get("files")
        .and_then(Value::as_object)
        .filter(|f| f.len() == 3);
    let files = files.ok_or_else(|| Error::Policy("Expected three files".into()))?;
    let mut kinds = Vec::with_capacity(3);
    for (path, digest) in files.iter() {
        let (kind, actual, extension) =
            parse_asset_path(path).ok_or_else(|| Error::Policy("Invalid immutable path".into()))?;
        let expected_extension = if kind == "controller" { "qml" } else { "js" };
        require(
            extension == expected_extension && digest.as_str() == Some(actual),
            "Filename digest mismatch",
        )?;
        kinds.push(kind);
    }
    kinds.sort_unstable();
    let entry_file = entry
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    require(
        kinds == ["controller", "core", "loader"] && files.contains_key(entry_file),
        "Missing artifact or entry",
    )
}

/// Every element of the chain must be an object.
pub fn validate_chain(chain: &Value) -> Result<&[Value]> {
    let items = chain
        .as_array()
        .filter(|items| items.iter().all(|e| e.as_object().is_some()));
    items
        .map(Vec::as_slice)
        .ok_or_else(|| Error::Policy("Invalid filter chain".into()))
}

/// The chain without EXACT owned entries.
pub fn remove_owned(chain: &Value, owned: &[Value]) -> Result<Vec<Value>> {
    let items = validate_chain(chain)?;
    for entry in owned {
        valid_entry(entry)?;
    }
    Ok(items
        .iter()
        .filter(|e| !owned.contains(e))
        .cloned()
        .collect())
}

/// Inserts `entry` immediately before the single `handleCPHotkeys` entry,
/// after removing owned generations; refuses an unowned remote handler.
pub fn build_chain(chain: &Value, entry: &Value, owned: &[Value]) -> Result<Vec<Value>> {
    valid_entry(entry)?;
    let mut result = remove_owned(chain, owned)?;
    let handler = |e: &Value| e.get("handler").and_then(Value::as_str).map(str::to_owned);
    require(
        !result
            .iter()
            .any(|e| handler(e).as_deref() == Some(HANDLER)),
        "Unowned remote handler present",
    )?;
    let anchors: Vec<usize> = result
        .iter()
        .enumerate()
        .filter(|(_, e)| handler(e).as_deref() == Some("handleCPHotkeys"))
        .map(|(i, _)| i)
        .collect();
    let [anchor] = anchors.as_slice() else {
        return Err(Error::Policy("Expected one handleCPHotkeys anchor".into()));
    };
    result.insert(*anchor, entry.clone());
    Ok(result)
}

/// Validated ownership state.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    /// The installed package.
    pub package: Value,
    /// Every entry generation ever installed.
    pub owned: Vec<Value>,
}

impl State {
    fn to_value(&self) -> Value {
        crate::json_object! {
            "owner" => OWNER,
            "version" => 1u64,
            "package" => self.package.clone(),
            "owned" => Value::Array(self.owned.clone()),
        }
    }

    fn entry(&self) -> Value {
        self.package.get("entry").cloned().unwrap_or(Value::Null)
    }
}

/// The device boundary (fakeable in tests).
pub trait Io {
    /// Reads private JSON (`None` when optional and absent).
    fn read_json(&mut self, path: &str, optional: bool) -> Result<Option<Value>>;
    /// Reads a private file.
    fn read_private(&mut self, path: &str) -> Result<Vec<u8>>;
    /// Atomically writes the ownership state.
    fn atomic_json(&mut self, path: &str, value: &Value) -> Result<()>;
    /// Live key-filter chain.
    fn get_filters(&mut self) -> Result<Value>;
    /// Writes the key-filter chain.
    fn set_filters(&mut self, chain: &[Value]) -> Result<()>;
}

/// Loads and validates the ownership state.
pub fn read_state(io: &mut impl Io) -> Result<Option<State>> {
    let Some(state) = io.read_json(STATE, true)? else {
        return Ok(None);
    };
    let object = state
        .as_object()
        .filter(|s| s.has_exact_keys(&["owner", "version", "package", "owned"]));
    let object = object.ok_or_else(|| Error::Policy("Invalid ownership state".into()))?;
    let version_ok = object
        .get("version")
        .is_some_and(|v| v.is_integer() && v.as_i64() == Some(1));
    require(
        object.get("owner").and_then(Value::as_str) == Some(OWNER) && version_ok,
        "Invalid ownership identity",
    )?;
    let package = object.get("package").cloned().unwrap_or(Value::Null);
    validate_package(&package)?;
    let owned = object
        .get("owned")
        .and_then(Value::as_array)
        .filter(|o| !o.is_empty() && o.len() <= MAX_OWNED);
    let owned = owned
        .cloned()
        .ok_or_else(|| Error::Policy("Invalid owned entries".into()))?;
    for entry in &owned {
        valid_entry(entry)?;
    }
    let state = State { package, owned };
    require(
        state.owned.contains(&state.entry()),
        "Current entry not journaled",
    )?;
    Ok(Some(state))
}

/// Verifies every package artefact against its content address.
pub fn verify_files(io: &mut impl Io, package: &Value) -> Result<()> {
    let files = package
        .get("files")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (path, digest) in files.iter() {
        require(
            Some(crate::sha::hex(io.read_private(path)?).as_str()) == digest.as_str(),
            "Artifact integrity failure",
        )?;
    }
    Ok(())
}

/// Writes `desired` only if the live chain still equals `before`, then reads
/// it back. A mismatch keeps ownership recorded so `disable` can still clean up.
fn write_chain(io: &mut impl Io, before: &Value, desired: &[Value]) -> Result<bool> {
    let desired_value = Value::Array(desired.to_vec());
    if *before == desired_value {
        return Ok(false);
    }
    require(
        io.get_filters()? == *before,
        "Concurrent configuration change; retry",
    )?;
    io.set_filters(desired)?;
    require(
        io.get_filters()? == desired_value,
        "Configuration readback mismatch; ownership retained",
    )?;
    Ok(true)
}

fn summary(changed: Option<bool>, filters: usize) -> Value {
    let mut map = Map::new();
    if let Some(changed) = changed {
        map.insert("changed", changed);
    }
    map.insert("filters", filters);
    map.insert("ok", true);
    Value::Object(map)
}

/// Installs (or re-installs) a package. Ownership is durable before the
/// configuration changes; re-installing the same package changes nothing.
pub fn install(io: &mut impl Io, package_path: &str) -> Result<Value> {
    let package = io.read_json(package_path, false)?.unwrap_or(Value::Null);
    validate_package(&package)?;
    verify_files(io, &package)?;
    let old = read_state(io)?;
    let mut owned = old.as_ref().map(|s| s.owned.clone()).unwrap_or_default();
    let before = io.get_filters()?;
    let entry = package.get("entry").cloned().unwrap_or(Value::Null);
    let desired = build_chain(&before, &entry, &owned)?;
    if !owned.contains(&entry) {
        owned.push(entry);
    }
    require(owned.len() <= MAX_OWNED, "Ownership history full")?;
    let state = State { package, owned };
    if old.as_ref() != Some(&state) {
        let value = state.to_value();
        io.atomic_json(STATE, &value)?;
        require(
            io.read_json(STATE, false)? == Some(value),
            "Ownership readback mismatch",
        )?;
    }
    let changed = write_chain(io, &before, &desired)?;
    Ok(summary(Some(changed), desired.len()))
}

/// Removes every owned entry from the LIVE chain; foreign entries stay.
pub fn disable(io: &mut impl Io) -> Result<Value> {
    let Some(state) = read_state(io)? else {
        return Ok(
            crate::json_object! {"ok" => true, "changed" => false, "reason" => "no ownership state"},
        );
    };
    let before = io.get_filters()?;
    let desired = remove_owned(&before, &state.owned)?;
    let changed = write_chain(io, &before, &desired)?;
    Ok(summary(Some(changed), desired.len()))
}

/// Verifies integrity and that exactly the current entry sits before
/// `handleCPHotkeys`.
pub fn check(io: &mut impl Io) -> Result<Value> {
    let state = read_state(io)?.ok_or_else(|| Error::Policy("No ownership state".into()))?;
    verify_files(io, &state.package)?;
    let live = io.get_filters()?;
    let items = validate_chain(&live)?;
    let remote: Vec<&Value> = items
        .iter()
        .filter(|e| e.get("handler").and_then(Value::as_str) == Some(HANDLER))
        .collect();
    let entry = state.entry();
    require(
        remote.len() == 1 && *remote[0] == entry,
        "Expected exactly one current remote entry",
    )?;
    let rebuilt = build_chain(&live, &entry, &state.owned)?;
    require(
        Value::Array(rebuilt) == live,
        "Remote entry is not before handleCPHotkeys",
    )?;
    Ok(summary(None, items.len()))
}

/// Which command to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Install the package at this path.
    Install(String),
    /// Remove owned entries.
    Disable,
    /// Verify the installation.
    Check,
}

/// Runs a command and prints one JSON line. Only policy messages are shown.
pub fn run(command: &Command, io: Result<impl Io>, out: &mut impl std::io::Write) -> i32 {
    let result = io.and_then(|mut io| match command {
        Command::Install(path) => install(&mut io, path),
        Command::Disable => disable(&mut io),
        Command::Check => check(&mut io),
    });
    match result {
        Ok(value) => {
            let _ = writeln!(out, "{}", json::to_string(&value, Style::Spaced, true));
            0
        }
        Err(error) => {
            let message = match error {
                Error::Policy(message) => message,
                _ => "I/O or data failure; ownership retained".to_owned(),
            };
            let report = crate::json_object! {"ok" => false, "error" => message};
            let _ = writeln!(out, "{}", json::to_string(&report, Style::Spaced, false));
            1
        }
    }
}

#[cfg(unix)]
pub use unix::{validate_metadata, LinuxIo};

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::fsx::{self, FileRule, Meta};
    use crate::paths;
    use crate::process::Limits;
    use std::time::Duration;

    const PRIVATE: FileRule = FileRule {
        forbidden_mode: 0o077,
        single_link: false,
        max_size: LIMIT,
        nonempty: true,
    };

    /// Metadata rule: root-owned unwritable directory, or private non-empty file.
    pub fn validate_metadata(meta: &Meta, directory: bool) -> Result<()> {
        require(meta.uid == 0, "Expected root ownership")?;
        if directory {
            require(fsx::is_safe_parent(meta), "Unsafe parent directory")
        } else {
            require(
                meta.is_regular() && meta.mode & 0o077 == 0,
                "Expected private regular file",
            )?;
            require(
                meta.size > 0 && meta.size <= LIMIT,
                "File size outside limit",
            )
        }
    }

    /// The device implementation.
    #[derive(Debug)]
    pub struct LinuxIo(());

    impl LinuxIo {
        /// Requires root.
        pub fn new() -> Result<Self> {
            require(
                cfg!(target_os = "linux") && fsx::euid() == 0,
                "Linux root required",
            )?;
            Ok(LinuxIo(()))
        }

        fn parents(path: &str) -> Result<()> {
            require(
                paths::is_canonical_absolute(path),
                "Expected canonical absolute path",
            )?;
            fsx::for_each_parent(path, |_, meta| validate_metadata(meta, true))
        }

        fn luna(method: &str, payload: &Value) -> Result<Value> {
            let limits = Limits {
                timeout: Duration::from_secs(10),
                max_output: LIMIT as usize,
                capture_stderr: false,
                interruptible: false,
            };
            let output = crate::luna::send(
                &format!("com.webos.service.config/{method}"),
                payload,
                limits,
            )?;
            require(output.status.success(), "Config call failed")?;
            let response =
                json::parse_bytes(&output.stdout).map_err(|e| Error::Data(e.to_string()))?;
            require(
                response.get("returnValue").and_then(Value::as_bool) == Some(true),
                "Config service rejected request",
            )?;
            Ok(response)
        }
    }

    impl Io for LinuxIo {
        fn read_json(&mut self, path: &str, optional: bool) -> Result<Option<Value>> {
            let data = match self.read_private(path) {
                Ok(data) => data,
                Err(error) if optional && error.is_not_found() => return Ok(None),
                Err(error) => return Err(error),
            };
            json::parse_bytes(&data).map(Some).map_err(|e| match e {
                json::ParseError::DuplicateKey(_) => Error::Policy("Duplicate JSON key".into()),
                other => Error::Data(other.to_string()),
            })
        }

        fn read_private(&mut self, path: &str) -> Result<Vec<u8>> {
            Self::parents(path)?;
            let before = fsx::lstat(path)?;
            validate_metadata(&before, false)?;
            let (mut file, opened) = fsx::open_verified(
                path,
                &before,
                true,
                |m| PRIVATE.accepts(m),
                "Expected private regular file",
                "File changed while opening",
            )?;
            let data = fsx::read_capped(&mut file, LIMIT, path)?;
            require(
                !data.is_empty() && data.len() as u64 <= LIMIT && data.len() as u64 == opened.size,
                "File changed or exceeds size limit",
            )?;
            Ok(data)
        }

        fn atomic_json(&mut self, path: &str, value: &Value) -> Result<()> {
            require(path == STATE, "Only ownership state may be written")?;
            Self::parents(path)?;
            match self.read_private(path) {
                Ok(_) => {}
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error),
            }
            let data = json::to_string(value, Style::Compact, true);
            require(
                !data.is_empty() && data.len() as u64 <= LIMIT,
                "Ownership state too large",
            )?;
            let temporary = format!("{ROOT}/.remote-controls-{}", fsx::random_hex(8)?);
            let result = (|| {
                let mut file = fsx::create_exclusive(&temporary, 0o600)?;
                fsx::write_synced(&mut file, data.as_bytes(), &temporary)?;
                drop(file);
                fsx::rename(&temporary, path)?;
                fsx::fsync_dir(ROOT)
            })();
            if fsx::lexists(&temporary) {
                let _ = fsx::unlink(&temporary);
            }
            result
        }

        fn get_filters(&mut self) -> Result<Value> {
            let payload =
                crate::json_object! {"configNames" => Value::Array(vec![Value::from(KEY)])};
            let response = Self::luna("getConfigs", &payload)?;
            let filters = response
                .get("configs")
                .filter(|c| c.as_object().is_some())
                .and_then(|c| c.get(KEY));
            let filters = filters
                .cloned()
                .ok_or_else(|| Error::Policy("Missing keyFilters response".into()))?;
            validate_chain(&filters)?;
            Ok(filters)
        }

        fn set_filters(&mut self, chain: &[Value]) -> Result<()> {
            let chain = Value::Array(chain.to_vec());
            validate_chain(&chain)?;
            let payload = crate::json_object! {"configs" => crate::json_object! {KEY => chain}};
            Self::luna("setConfigs", &payload).map(drop)
        }
    }
}
