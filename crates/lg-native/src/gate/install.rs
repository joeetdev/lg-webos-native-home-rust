//! Gate installation: validate the immutable assets, record ownership and the
//! safety hooks durably, arm the independent trial guard, and only then insert
//! the gate entry before the system key handler.

use super::{remove_owned_filter, validate_entry, validate_state, HANDLER, OWNER};
use crate::ensure;
use crate::error::{Error, Result};
use crate::json::{Map, Value};

/// The webOS Homebrew Channel boot hook that applies the overrides.
pub const HOOK: &str = "/var/lib/webosbrew/init.d/55-lg-native";
/// Temporary boot hook that re-arms the trial guard until `gate commit`.
pub const TRIAL: &str = "/var/lib/webosbrew/init.d/54-lg-native-gate-trial";
/// Accepted package locations.
pub const PACKAGES: [&str; 2] = [
    "/var/lib/lg-native/gate-package-trial.json",
    "/var/lib/lg-native/gate-package-final.json",
];

/// The safe boot hook: apply the overrides once per boot, then release the
/// gate unless all eight overrides are proven healthy. Identical to
/// `runtime/55-lg-native`.
pub const SAFE_HOOK: &str = include_str!("../../../../runtime/55-lg-native");

/// The trial hook: arm the guard on every boot until the gate is committed.
pub const TRIAL_HOOK: &str = "#!/bin/sh
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
exec /var/lib/lg-native/lg-native gate guard --spawn
";

/// The trial hook written by the earlier Python implementation. Recognised as
/// owned so an uncommitted trial can still be replaced or committed.
pub const LEGACY_TRIAL_HOOK: &str = "#!/bin/sh
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
exec /usr/bin/python3 -B -E -s /var/lib/lg-native/gate_trial_guard.py --spawn
";

/// True when `content` is a trial hook this project wrote.
pub fn is_owned_trial_hook(content: &[u8]) -> bool {
    content == TRIAL_HOOK.as_bytes() || content == LEGACY_TRIAL_HOOK.as_bytes()
}

/// The system key handler the gate must precede.
const SYSTEM_KEYS_FILE: &str = "/usr/lib/qt5/qml/KeyFilters/systemUi.js";

/// Builds the new chain: drop owned generations, then insert `entry`
/// immediately before the single original `handleSystemKeys` entry.
pub fn build_chain(current: &Value, entry: &Value, state: Option<&Value>) -> Result<Vec<Value>> {
    validate_entry(Some(entry))?;
    let mut remaining = match state {
        Some(state) => remove_owned_filter(current, state)?,
        None => current
            .as_array()
            .cloned()
            .ok_or_else(|| Error::Policy("keyFilters must be an array".into()))?,
    };
    let field = |e: &Value, key: &str| e.get(key).and_then(Value::as_str).map(str::to_owned);
    let slots: Vec<usize> = remaining
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            field(e, "handler").as_deref() == Some("handleSystemKeys")
                && field(e, "file").as_deref() == Some(SYSTEM_KEYS_FILE)
        })
        .map(|(i, _)| i)
        .collect();
    let [slot] = slots.as_slice() else {
        return Err(Error::Policy(
            "Expected one original system key handler".into(),
        ));
    };
    let foreign_gate = remaining
        .iter()
        .any(|e| field(e, "handler").as_deref() == Some(HANDLER));
    ensure!(!foreign_gate, "Unknown Home gate reference");
    remaining.insert(*slot, entry.clone());
    Ok(remaining)
}

/// The new state: the package plus every owned generation and the original
/// chain/hook recorded by the FIRST installation.
pub fn next_state(
    package: &Value,
    previous: Option<&Value>,
    current: &Value,
    hook: &str,
    rollback: Option<&str>,
) -> Result<Value> {
    validate_state(package)?;
    let entry = package.get("entry").cloned().unwrap_or(Value::Null);
    let mut entries = previous.map(super::owned_entries).unwrap_or_default();
    if !entries.contains(&entry) {
        entries.push(entry);
    }
    let mut state: Map = package.as_object().cloned().unwrap_or_default();
    state.insert("ownedEntries", Value::Array(entries));
    let inherited = |key: &str| -> Result<Option<Value>> {
        match previous {
            Some(previous) => previous
                .get(key)
                .cloned()
                .map(Some)
                .ok_or_else(|| Error::Policy(format!("Previous gate state lacks {key}"))),
            None => Ok(None),
        }
    };
    state.insert(
        "originalKeyFilters17",
        inherited("originalKeyFilters17")?.unwrap_or_else(|| current.clone()),
    );
    state.insert(
        "originalHook",
        inherited("originalHook")?.unwrap_or_else(|| Value::from(hook)),
    );
    // Only the Python implementation had a separate rollback wrapper; keep its
    // record when present, never invent one.
    let original_rollback = previous
        .and_then(|p| p.get("originalRollback").cloned())
        .or(rollback.map(Value::from));
    if let Some(original_rollback) = original_rollback {
        state.insert("originalRollback", original_rollback);
    }
    let state = Value::Object(state);
    validate_state(&state)?;
    Ok(state)
}

/// The device side of an installation (fakeable in tests).
pub trait InstallIo {
    /// Validates the three content-addressed assets against the package.
    fn check_files(&mut self, package: &Value) -> Result<()>;
    /// Current gate state.
    fn load_state(&mut self) -> Result<Option<Value>>;
    /// Live key-filter chain.
    fn get_filters(&mut self) -> Result<Value>;
    /// The current hook (and legacy rollback wrapper, if any).
    fn read_originals(&mut self) -> Result<(String, Option<String>)>;
    /// Writes the safe boot hook and the trial hook.
    fn install_safety(&mut self) -> Result<()>;
    /// Durably writes gate state.
    fn save_state(&mut self, state: &Value) -> Result<()>;
    /// Arms the independent trial guard.
    fn arm_guard(&mut self) -> Result<()>;
    /// Writes the key-filter chain.
    fn set_filters(&mut self, chain: &[Value]) -> Result<()>;
}

/// Installs a gate package. Ownership and recovery are durable BEFORE the
/// configuration changes; the live chain is verified afterwards.
pub fn install(package: &Value, io: &mut impl InstallIo) -> Result<Value> {
    io.check_files(package)?;
    let previous = io.load_state()?;
    let current = io.get_filters()?;
    let (hook, rollback) = io.read_originals()?;
    let entry = package.get("entry").cloned().unwrap_or(Value::Null);
    let chain = build_chain(&current, &entry, previous.as_ref())?;
    let state = next_state(
        package,
        previous.as_ref(),
        &current,
        &hook,
        rollback.as_deref(),
    )?;
    io.install_safety()?;
    io.save_state(&state)?;
    io.arm_guard()?;
    io.set_filters(&chain)?;
    ensure!(
        io.get_filters()? == Value::Array(chain),
        "Gate installation readback mismatch"
    );
    Ok(state)
}

/// Validates the package file names: `<root>/(gate-proof|native-home-gate|
/// native-home-gate-loader)-<digest>.(js|qml)` for each listed digest.
pub fn check_package_paths(package: &Value) -> Result<Vec<(String, String)>> {
    validate_state(package)?;
    let files = package.get("files").and_then(Value::as_object);
    let files = files
        .filter(|f| f.len() == 3)
        .ok_or_else(|| Error::Policy("Expected three immutable gate files".into()))?;
    let mut result = Vec::with_capacity(3);
    for (path, digest) in files.iter() {
        let digest = digest.as_str().unwrap_or_default();
        let addressed = ["gate-proof", "native-home-gate", "native-home-gate-loader"]
            .iter()
            .any(|kind| {
                [".js", ".qml"]
                    .iter()
                    .any(|ext| path == format!("/var/lib/lg-native/{kind}-{digest}{ext}"))
            });
        ensure!(addressed, "Non-addressed gate asset");
        result.push((path.to_owned(), digest.to_owned()));
    }
    let entry_file = package
        .get("entry")
        .and_then(|e| e.get("file"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure!(files.contains_key(entry_file), "Missing loader asset");
    Ok(result)
}

/// Accepts only the owned package identity.
pub fn package_owner_ok(package: &Value) -> bool {
    package.get("owner").and_then(Value::as_str) == Some(OWNER)
}

#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::fsx;
    use crate::gate::{
        self, guard, luna_call, secure_parents, trusted_bytes, trusted_required, STATE_NAME,
    };
    use crate::json::{self, Style};
    use crate::paths::{self, ROOT};
    use std::io::Write;
    use std::time::Duration;

    /// The device implementation of [`InstallIo`].
    #[derive(Debug)]
    pub struct LinuxIo(());

    impl LinuxIo {
        /// Requires root on Linux and a private (0700) root directory.
        pub fn new() -> Result<Self> {
            gate::require_root()?;
            secure_parents(&paths::in_root("sentinel"))?;
            let root = std::fs::metadata(ROOT).map_err(|e| Error::io(format!("stat {ROOT}"), e))?;
            ensure!(
                fsx::meta_of(&root).permissions() & 0o777 == 0o700,
                "Private root mode"
            );
            Ok(LinuxIo(()))
        }
    }

    /// Atomic write with read-back: validate any existing file, write a new
    /// temporary file, fsync it, rename it over the target, fsync the directory.
    pub fn atomic(path: &str, data: &[u8], mode: u32) -> Result<()> {
        secure_parents(path)?;
        if fsx::lexists(path) {
            trusted_bytes(path, false)?;
        }
        let temporary = format!(
            "{}/.{}.gate-new-{}",
            paths::parent(path),
            paths::file_name(path),
            fsx::random_hex(8)?
        );
        let result = (|| {
            let mut file = fsx::create_exclusive(&temporary, mode)?;
            fsx::write_synced(&mut file, data, &temporary)?;
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(|e| Error::io(format!("chmod {temporary}"), e))?;
            drop(file);
            fsx::rename(&temporary, path)?;
            fsx::fsync_dir(paths::parent(path))
        })();
        if fsx::lexists(&temporary) {
            let _ = fsx::unlink(&temporary);
        }
        result?;
        ensure!(
            trusted_required(path)? == data,
            "Atomic write readback mismatch"
        );
        Ok(())
    }

    fn check_trial_hook() -> Result<()> {
        if fsx::lexists(TRIAL) {
            ensure!(
                is_owned_trial_hook(&trusted_required(TRIAL)?),
                "Foreign trial hook"
            );
        }
        Ok(())
    }

    impl InstallIo for LinuxIo {
        fn check_files(&mut self, package: &Value) -> Result<()> {
            for (path, digest) in check_package_paths(package)? {
                ensure!(
                    crate::sha::hex(trusted_required(&path)?) == digest,
                    "Gate asset hash mismatch"
                );
            }
            Ok(())
        }

        fn load_state(&mut self) -> Result<Option<Value>> {
            gate::load_state()
        }

        fn get_filters(&mut self) -> Result<Value> {
            gate::get_filters(&mut luna_call)
        }

        fn read_originals(&mut self) -> Result<(String, Option<String>)> {
            let text = |bytes: Vec<u8>| {
                String::from_utf8(bytes).map_err(|_| Error::Data("Hook is not UTF-8".into()))
            };
            let hook = text(trusted_required(HOOK)?)?;
            let legacy = paths::in_root("rollback.py");
            let rollback = trusted_bytes(&legacy, true)?.map(text).transpose()?;
            Ok((hook, rollback))
        }

        fn install_safety(&mut self) -> Result<()> {
            check_trial_hook()?;
            atomic(HOOK, SAFE_HOOK.as_bytes(), 0o755)?;
            atomic(TRIAL, TRIAL_HOOK.as_bytes(), 0o755)
        }

        fn save_state(&mut self, state: &Value) -> Result<()> {
            atomic(
                &paths::in_root(STATE_NAME),
                json::to_string(state, Style::Indented, false).as_bytes(),
                0o600,
            )
        }

        fn arm_guard(&mut self) -> Result<()> {
            if guard::read_token()?.is_some() {
                // Cancels any running guard: it notices the token change within a second.
                fsx::unlink(guard::PENDING)?;
                std::thread::sleep(Duration::from_millis(1300));
            }
            let token = fsx::random_hex(16)?;
            let record = crate::json_object! {"token" => token.as_str()};
            atomic(
                guard::PENDING,
                json::to_string(&record, Style::Spaced, false).as_bytes(),
                0o600,
            )?;
            guard::arm_watcher(&token)
        }

        fn set_filters(&mut self, chain: &[Value]) -> Result<()> {
            let payload = crate::json_object! {"configs" => crate::json_object! {gate::KEY => Value::Array(chain.to_vec())}};
            gate::checked_call(&mut luna_call, "setConfigs", &payload).map(drop)
        }
    }

    fn print(out: &mut impl Write, value: &Value) {
        let _ = writeln!(out, "{}", json::to_string(value, Style::Spaced, false));
    }

    /// `lg-native gate check`.
    pub fn check(out: &mut impl Write) -> Result<()> {
        let mut io = LinuxIo::new()?;
        let ready = gate::device_ready()?;
        let filters = io.get_filters()?;
        print(
            out,
            &crate::json_object! {"nativeReady" => ready, "keyFilters" => filters},
        );
        Ok(())
    }

    /// `lg-native gate install <package>`.
    pub fn install_package(path: &str, out: &mut impl Write) -> Result<()> {
        let mut io = LinuxIo::new()?;
        ensure!(PACKAGES.contains(&path), "Unknown package path");
        let package = gate::parse_trusted(&trusted_required(path)?)?;
        let state = install(&package, &mut io)?;
        let count = io.get_filters()?.as_array().map_or(0, Vec::len);
        let ready = gate::device_ready()?;
        print(
            out,
            &crate::json_object! {
                "installed" => true,
                "entry" => state.get("entry").cloned().unwrap_or(Value::Null),
                "filterCount" => count,
                "nativeReady" => ready,
            },
        );
        Ok(())
    }

    /// `lg-native gate commit`: after final verification, retire the trial
    /// token and the temporary hook.
    pub fn commit(out: &mut impl Write) -> Result<()> {
        let mut io = LinuxIo::new()?;
        let state = io
            .load_state()?
            .ok_or_else(|| Error::Policy("No gate state".into()))?;
        ensure!(
            state.get("mode").and_then(Value::as_str) == Some("final"),
            "Only clean final gate can commit"
        );
        let filters = io.get_filters()?;
        let entry = state.get("entry").cloned().unwrap_or(Value::Null);
        let count = filters
            .as_array()
            .map_or(0, |f| f.iter().filter(|e| **e == entry).count());
        ensure!(count == 1, "Final reference missing");
        ensure!(gate::device_ready()?, "Native overlays not ready");
        if guard::read_token()?.is_some() {
            fsx::unlink(guard::PENDING)?;
        }
        if fsx::lexists(TRIAL) {
            check_trial_hook()?;
            fsx::unlink(TRIAL)?;
        }
        std::thread::sleep(Duration::from_millis(1300));
        let total = filters.as_array().map_or(0, Vec::len);
        print(
            out,
            &crate::json_object! {"committed" => true, "filterCount" => total, "nativeReady" => true},
        );
        Ok(())
    }
}
