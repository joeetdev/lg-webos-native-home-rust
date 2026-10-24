//! Root-only, reversible recovery for the native Home presentation gate.
//!
//! The gate itself is a non-visual QML controller loaded through the
//! compositor's `keyFilters` chain; it keeps Home windows transparent until the
//! eight overrides are proven AND the window carries the customised Home
//! marker. This module can only remove the exact key-filter entries recorded
//! as owned; Home mounts, preload policy, processes and files stay untouched.

pub mod guard;
pub mod install;

use crate::ensure;
use crate::error::{Error, Result};
use crate::home::model::{self, validate_file, validate_journal, validate_manifest, ALLOWED};
use crate::home::HomeReader;
use crate::json::{self, Map, Style, Value};
use crate::paths;
use crate::sha;

/// State file name inside the private root.
pub const STATE_NAME: &str = "native-home-gate-state.json";
/// Gate state owner identity.
pub const OWNER: &str = "lg-native-home-gate-v1";
/// The compositor key-filter configuration.
pub const KEY: &str = "com.webos.surfacemanager.keyFilters";
/// Handler exported by the gate loader.
pub const HANDLER: &str = "handleNativeHomeGate";
/// The config service methods this subsystem may call.
pub const GET_CONFIGS: &str = "com.webos.service.config/getConfigs";
/// See [`GET_CONFIGS`].
pub const SET_CONFIGS: &str = "com.webos.service.config/setConfigs";

/// The injectable LS2 boundary: `call(method, payload) -> response`.
pub trait ConfigCall {
    /// Performs one LS2 call.
    fn call(&mut self, method: &str, payload: &Value) -> Result<Value>;
}

impl<F: FnMut(&str, &Value) -> Result<Value>> ConfigCall for F {
    fn call(&mut self, method: &str, payload: &Value) -> Result<Value> {
        self(method, payload)
    }
}

fn is_loader_path(path: &str) -> bool {
    path.strip_prefix("/var/lib/lg-native/native-home-gate-loader-")
        .and_then(|rest| rest.strip_suffix(".js"))
        .is_some_and(sha::is_hex64)
}

/// An owned key-filter entry: exactly `{file: <content-addressed loader>,
/// handler: "handleNativeHomeGate"}`.
pub fn validate_entry(entry: Option<&Value>) -> Result<()> {
    let valid = entry.and_then(Value::as_object).is_some_and(|e| {
        e.has_exact_keys(&["file", "handler"])
            && e.get("handler").and_then(Value::as_str) == Some(HANDLER)
            && e.get("file")
                .and_then(Value::as_str)
                .is_some_and(is_loader_path)
    });
    ensure!(valid, "Invalid owned gate entry");
    Ok(())
}

/// Validates gate state (or a gate package, which shares its shape).
pub fn validate_state(state: &Value) -> Result<()> {
    let valid_identity = state
        .get("version")
        .is_some_and(|v| v.is_integer() && v.as_i64() == Some(1))
        && state.get("owner").and_then(Value::as_str) == Some(OWNER);
    ensure!(
        state.as_object().is_some() && valid_identity,
        "Unknown gate state owner/version"
    );
    let entry = state.get("entry");
    validate_entry(entry)?;
    if let Some(owned) = state.get("ownedEntries") {
        let entries = owned.as_array().filter(|e| (1..=4).contains(&e.len()));
        let entries = entries
            .ok_or_else(|| Error::Policy("Expected one to four owned gate entries".into()))?;
        for item in entries {
            validate_entry(Some(item))?;
        }
        ensure!(
            entry.is_some_and(|e| entries.contains(e)),
            "Active gate entry missing from ownedEntries"
        );
        let mut pairs: Vec<(&str, &str)> = entries
            .iter()
            .map(|e| (model::text_of(e, "file"), model::text_of(e, "handler")))
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        ensure!(pairs.len() == entries.len(), "Duplicate owned gate entry");
    }
    if let Some(files) = state.get("files") {
        let files = files
            .as_object()
            .ok_or_else(|| Error::Policy("Gate files must be an object".into()))?;
        for (path, digest) in files.iter() {
            ensure!(paths::is_trusted_path(path), "Noncanonical gate file path");
            ensure!(
                digest.as_str().is_some_and(sha::is_hex64),
                "Invalid gate file SHA256"
            );
        }
    }
    Ok(())
}

/// The entries recorded as owned (legacy states only record `entry`).
pub fn owned_entries(state: &Value) -> Vec<Value> {
    match state.get("ownedEntries").and_then(Value::as_array) {
        Some(entries) => entries.clone(),
        None => state.get("entry").cloned().into_iter().collect(),
    }
}

/// A fresh chain without EXACT owned matches; foreign values and order kept.
pub fn remove_owned_filter(filters: &Value, state: &Value) -> Result<Vec<Value>> {
    validate_state(state)?;
    let owned = owned_entries(state);
    let filters = filters
        .as_array()
        .ok_or_else(|| Error::Policy("keyFilters must be an array".into()))?;
    Ok(filters
        .iter()
        .filter(|item| !owned.contains(item))
        .cloned()
        .collect())
}

/// One config call that must report `returnValue: true`.
pub fn checked_call(call: &mut impl ConfigCall, method: &str, payload: &Value) -> Result<Value> {
    let response = call.call(&format!("com.webos.service.config/{method}"), payload)?;
    ensure!(
        response.get("returnValue").and_then(Value::as_bool) == Some(true),
        "Config LS2 call failed: {method}"
    );
    Ok(response)
}

/// The live key-filter chain.
pub fn get_filters(call: &mut impl ConfigCall) -> Result<Value> {
    let payload = crate::json_object! {"configNames" => Value::Array(vec![Value::from(KEY)])};
    let response = checked_call(call, "getConfigs", &payload)?;
    let filters = response
        .get("configs")
        .and_then(|c| c.get(KEY))
        .filter(|f| f.as_array().is_some());
    filters
        .cloned()
        .ok_or_else(|| Error::Policy("Missing or invalid keyFilters response".into()))
}

/// Result of a gate removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disabled {
    /// The owned entries are verified absent.
    pub disabled: bool,
    /// The chain had to be rewritten.
    pub changed: bool,
}

/// Removes the exact owned entries from the LIVE chain and verifies the write
/// byte-for-byte (type and order drift in foreign entries is detected). No
/// state means no LS2 call. A saved list is never restored.
pub fn disable_gate(state: Option<&Value>, call: &mut impl ConfigCall) -> Result<Disabled> {
    let Some(state) = state else {
        return Ok(Disabled {
            disabled: false,
            changed: false,
        });
    };
    validate_state(state)?;
    let before = get_filters(call)?;
    let after = Value::Array(remove_owned_filter(&before, state)?);
    let changed = !after.strict_eq(&before);
    if changed {
        let payload = crate::json_object! {"configs" => crate::json_object! {KEY => after.clone()}};
        checked_call(call, "setConfigs", &payload)?;
        ensure!(
            get_filters(call)?.strict_eq(&after),
            "Gate removal readback mismatch"
        );
    }
    Ok(Disabled {
        disabled: true,
        changed,
    })
}

/// Read-only proof that all eight overrides are healthy in THIS boot.
///
/// Requires: no disable marker; an 8-entry manifest covering every allowed
/// target; a modern journal whose preload intent completed in this boot; and,
/// for every entry, live mount/inode/hash ownership, matching metadata on
/// payload, backing copy and target, and exactly one read-only mount.
/// `HomeReader` exposes no mutating operation, so this cannot change anything.
pub fn native_ready(ops: &impl HomeReader) -> Result<bool> {
    ensure!(!ops.is_disabled(), "Native Home is disabled");
    let manifest = validate_manifest(&ops.load_manifest()?, false)?;
    ensure!(
        manifest.len() == ALLOWED.len()
            && ALLOWED
                .iter()
                .all(|t| manifest.iter().any(|e| e.target == *t)),
        "Expected all eight native Home overlays"
    );
    let raw = ops.load_journal()?;
    ensure!(
        raw.as_ref().is_some_and(|j| j.as_object().is_some()),
        "Modern native Home journal required"
    );
    let journal = validate_journal(raw)?;
    let preload_done = journal
        .preload()
        .is_some_and(|p| !p.pending && p.boot_id == ops.boot_id());
    ensure!(preload_done, "Native preload recovery incomplete or stale");
    ensure!(
        journal.mounts.len() == ALLOWED.len(),
        "Expected eight journal-owned overlays"
    );
    let same_targets = journal.mounts.iter().all(|m| {
        manifest
            .iter()
            .any(|e| e.target == model::text(m, "target"))
    }) && manifest.iter().all(|e| {
        journal
            .mounts
            .iter()
            .any(|m| model::text(m, "target") == e.target)
    });
    ensure!(same_targets, "Journal/manifest target mismatch");
    for entry in &journal.mounts {
        check_overlay(ops, entry, &manifest)?;
    }
    Ok(true)
}

fn check_overlay(
    ops: &impl HomeReader,
    entry: &Map,
    manifest: &[model::ManifestEntry],
) -> Result<()> {
    let target = model::text(entry, "target");
    let expected = manifest
        .iter()
        .find(|e| e.target == target)
        .ok_or_else(|| policy_mismatch(target))?;
    let integers = ["mountId", "sourceDev", "sourceIno"]
        .iter()
        .all(|k| entry.get(k).is_some_and(Value::is_integer));
    ensure!(
        model::text(entry, "state") == "mounted"
            && entry.get("bootId").and_then(Value::as_str) == Some(ops.boot_id())
            && integers,
        "Missing or stale current-boot mount identity"
    );
    ensure!(
        entry.get("inputSource").and_then(Value::as_str) == Some(expected.source.as_str())
            && model::text(entry, "originalHash") == expected.original_hash
            && model::text(entry, "newHash") == expected.new_hash,
        "Journal/manifest source or hash mismatch"
    );
    validate_file(
        &ops.inspect(&expected.source)?,
        &expected.source,
        Some(&expected.new_hash),
    )?;
    // LIVE mount/inode/hash proof, not journal text claiming ownership. One
    // snapshot of the mount table and one inspection per file serve every check.
    let mounts = ops.mounts()?;
    let exact = model::mounts_at(&mounts, target);
    ensure!(!exact.is_empty(), "Native Home mount is not owned");
    let copy = ops.inspect(model::text(entry, "source"))?;
    let stock = ops.inspect(target)?;
    ensure!(
        model::owned_mount(entry, &copy, &stock, &mounts, ops.boot_id())?,
        "Native Home mount is not owned"
    );
    let metadata_matches = [
        ("uid", copy.uid, stock.uid),
        ("gid", copy.gid, stock.gid),
        ("mode", copy.mode, stock.mode),
    ]
    .iter()
    .all(|(key, a, b)| a == b && entry.get(key) == Some(&Value::from(*a)));
    ensure!(metadata_matches, "Native Home mount metadata mismatch");
    ensure!(
        exact.len() == 1 && exact[0].is_readonly(),
        "Native Home mount is not readonly"
    );
    Ok(())
}

fn policy_mismatch(target: &str) -> Error {
    Error::Policy(format!("Journal/manifest target mismatch: {target}"))
}

/// Result of a conditional recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovery {
    /// The overrides were proven healthy.
    pub ready: bool,
    /// The gate entries are verified absent.
    pub disabled: bool,
    /// The chain was rewritten.
    pub changed: bool,
    /// Why the proof failed, when it raised.
    pub reason: Option<String>,
}

/// Removes the gate unless the overrides are proven ready. A failed or
/// unknown health check MUST NOT prevent the removal; invalid ownership or a
/// failed read-back still fail, so recovery is never reported unverified.
pub fn recover_if_unready(
    state: Option<&Value>,
    call: &mut impl ConfigCall,
    checker: impl FnOnce() -> Result<bool>,
) -> Result<Recovery> {
    if let Some(state) = state {
        validate_state(state)?;
    }
    let (ready, reason) = match checker() {
        Ok(ready) => (ready, None),
        Err(error) => (false, Some(format!("{}: {error}", error.kind()))),
    };
    if ready {
        return Ok(Recovery {
            ready: true,
            disabled: false,
            changed: false,
            reason: None,
        });
    }
    let removed = disable_gate(state, call)?;
    Ok(Recovery {
        ready: false,
        disabled: removed.disabled,
        changed: removed.changed,
        reason,
    })
}

/// Which recovery command to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Remove only the exact owned gate filter.
    Disable,
    /// Remove the gate unless all eight overlays are healthy.
    RecoverIfUnready,
}

/// `lg-native gate disable|recover-if-unready`, printing one JSON object.
/// `load` performs the root/binary preflight and loads the state; any
/// failure there is reported before a single LS2 call is made.
pub fn run(
    command: Command,
    load: impl FnOnce() -> Result<Option<Value>>,
    call: &mut impl ConfigCall,
    checker: impl FnOnce() -> Result<bool>,
    out: &mut impl std::io::Write,
) -> i32 {
    let result = (|| -> Result<Value> {
        let state = load()?;
        let Some(state) = state else {
            let mut map = Map::new();
            map.insert("disabled", false);
            map.insert("changed", false);
            map.insert("state", "absent");
            if command == Command::RecoverIfUnready {
                map.insert("ready", false);
            }
            return Ok(Value::Object(map));
        };
        match command {
            // Independent of the Home runtime, journal, mounts and UI health.
            Command::Disable => {
                let r = disable_gate(Some(&state), call)?;
                Ok(crate::json_object! {"disabled" => r.disabled, "changed" => r.changed})
            }
            Command::RecoverIfUnready => {
                let r = recover_if_unready(Some(&state), call, checker)?;
                let mut value = crate::json_object! {
                    "ready" => r.ready, "disabled" => r.disabled, "changed" => r.changed,
                };
                if let (Some(reason), Some(map)) = (r.reason, value.as_object_mut()) {
                    map.insert("reason", reason);
                }
                Ok(value)
            }
        }
    })();
    match result {
        Ok(value) => {
            let _ = writeln!(out, "{}", report(&value));
            0
        }
        Err(error) => {
            let message = format!("{}: {error}", error.kind());
            let _ = writeln!(out, "{}", report(&crate::json_object! {"error" => message}));
            1
        }
    }
}

/// Serialises a command result like Python's `json.dumps(..., sort_keys=True)`.
pub fn report(value: &Value) -> String {
    json::to_string(value, Style::Spaced, true)
}

#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::fsx::{self, FileRule, MIB};
    use crate::process::Limits;
    use std::time::Duration;

    /// The private root.
    pub const ROOT: &str = paths::ROOT;

    const TRUSTED: FileRule = FileRule {
        forbidden_mode: 0o022,
        single_link: true,
        max_size: MIB,
        nonempty: false,
    };

    /// Ancestors must be root-owned directories that group/other cannot write.
    pub fn secure_parents(path: &str) -> Result<()> {
        ensure!(paths::is_trusted_path(path), "Noncanonical trusted path");
        fsx::for_each_parent(path, |parent, meta| {
            ensure!(
                fsx::is_safe_parent(meta),
                "Unsafe parent directory: {parent}"
            );
            Ok(())
        })
    }

    /// Reads ONE trusted inode (root-owned, single link, not group/other
    /// writable, ≤ 1 MiB) — never a check-then-reopen of a pathname.
    pub fn trusted_bytes(path: &str, optional: bool) -> Result<Option<Vec<u8>>> {
        secure_parents(path)?;
        let before = match fsx::lstat(path) {
            Ok(meta) => meta,
            Err(error) if optional && error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        let unsafe_message = format!("Unsafe trusted file: {path}");
        ensure!(TRUSTED.accepts(&before), "{unsafe_message}");
        let (mut file, _) = fsx::open_verified(
            path,
            &before,
            true,
            |m| TRUSTED.accepts(m),
            &unsafe_message,
            "Trusted file changed during open",
        )?;
        let data = fsx::read_capped(&mut file, MIB, path)?;
        ensure!(data.len() as u64 <= MIB, "Oversized trusted file");
        Ok(Some(data))
    }

    /// Reads a required trusted file.
    pub fn trusted_required(path: &str) -> Result<Vec<u8>> {
        trusted_bytes(path, false)?
            .ok_or_else(|| Error::Policy(format!("Missing trusted file: {path}")))
    }

    /// Parses trusted JSON, rejecting duplicate keys.
    pub fn parse_trusted(data: &[u8]) -> Result<Value> {
        json::parse_bytes(data).map_err(|e| Error::Policy(e.to_string()))
    }

    /// Loads gate state. An uninstalled private root is a no-op rather than a
    /// reason to touch config — but its ancestors are validated first, because
    /// a dangling symlink is NOT an absent root.
    pub fn load_state() -> Result<Option<Value>> {
        load_state_at(ROOT)
    }

    /// [`load_state`] for an explicit root (tests).
    pub fn load_state_at(root: &str) -> Result<Option<Value>> {
        secure_parents(root)?;
        let directory = match fsx::lstat(root) {
            Ok(meta) => meta,
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        ensure!(fsx::is_safe_parent(&directory), "Unsafe gate state root");
        let Some(raw) = trusted_bytes(&format!("{root}/{STATE_NAME}"), true)? else {
            return Ok(None);
        };
        let state = parse_trusted(&raw)?;
        validate_state(&state)?;
        Ok(Some(state))
    }

    /// The device LS2 boundary: only the two config methods, bounded, no shell,
    /// duplicate keys rejected.
    pub fn luna_call(method: &str, payload: &Value) -> Result<Value> {
        ensure!(
            method == GET_CONFIGS || method == SET_CONFIGS,
            "Unapproved Luna method"
        );
        let limits = Limits {
            timeout: Duration::from_secs(10),
            max_output: MIB as usize,
            capture_stderr: true,
            interruptible: false,
        };
        let output = crate::luna::send(method, payload, limits)?;
        ensure!(
            output.status.success(),
            "Luna command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let response = parse_trusted(&output.stdout)?;
        ensure!(
            response.get("returnValue").and_then(Value::as_bool) == Some(true),
            "Luna returned failure: {}",
            json::compact(&response)
        );
        Ok(response)
    }

    /// Root and platform check shared by every gate command.
    pub fn require_root() -> Result<()> {
        ensure!(
            cfg!(target_os = "linux") && fsx::euid() == 0,
            "Requires root on Linux; not a host installer"
        );
        Ok(())
    }

    /// The default readiness checker: the device Home runtime, read-only.
    #[cfg(target_os = "linux")]
    pub fn device_ready() -> Result<bool> {
        let ops = crate::home::linux::LinuxOps::new()?;
        native_ready(&ops)
    }

    /// Non-Linux hosts never prove readiness.
    #[cfg(not(target_os = "linux"))]
    pub fn device_ready() -> Result<bool> {
        Err(Error::Policy(
            "Requires root on Linux; not a host installer".into(),
        ))
    }

    /// Releases the gate before a native rollback (an owned gate would
    /// otherwise keep stock Home invisible forever).
    pub fn release_for_rollback() -> Result<Disabled> {
        disable_gate(load_state()?.as_ref(), &mut luna_call)
    }
}
