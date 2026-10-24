//! Pure rules of the Home runtime: the exact target allowlist, manifest and
//! journal validation, mount ownership and launch decisions. No I/O.

use crate::ensure;
use crate::error::{Error, Result};
use crate::fsx::Meta;
use crate::json::{self, Map, Value};
use crate::paths::{self, ROOT};
use crate::sha;

/// Application id of the native Home.
pub const HOME: &str = "com.webos.app.home";
/// The exact Home executable; only processes running it are ever signalled.
pub const EXE: &str = "/usr/bin/com.webos.app.home";
/// Directory holding the Home QML tree.
pub const QML: &str = "/usr/palm/applications/com.webos.app.home/qml/";
/// Journal owner identity.
pub const OWNER: &str = "lg-native-home-v1";

/// The only eight files that may ever be overridden.
pub const ALLOWED: [&str; 8] = [
    "/usr/palm/applications/com.webos.app.home/qml/UserInterfaceLayer/Containers/MainView.qml",
    "/usr/palm/applications/com.webos.app.home/qml/UserInterfaceLayer/Containers/Main/MainRibbon.qml",
    "/usr/palm/applications/com.webos.app.home/qml/UserInterfaceLayer/Containers/Main/TopShelf.qml",
    "/usr/palm/applications/com.webos.app.home/qml/UserInterfaceLayer/Containers/Main/HomeDashboard.qml",
    "/usr/palm/applications/com.webos.app.home/qml/UserInterfaceLayer/Containers/Sub/MobileComponent.qml",
    "/usr/palm/applications/com.webos.app.home/qml/Model/ShelfList.js",
    "/usr/palm/applications/com.webos.app.home/qml/Interfaces/ConnectionsInterfaces/ConnectionsInterfaces.qml",
    "/usr/palm/applications/com.webos.app.home/qml/Configurations/SystemServiceResponseProcessors.js",
];

/// Names under the private root that a manifest source may never point into:
/// runtime state, helpers and (legacy) implementation files.
pub const RESERVED: [&str; 18] = [
    "home-mounts.json",
    "home-final-manifest.json",
    "home-disabled",
    "home-boot.json",
    "home.lock",
    "home-preload-trial.json",
    "checkpoint-backup.json",
    "native_home.py",
    "nativeapply.py",
    "rollback.py",
    "clear-home-checkpoint.py",
    "55-lg-native",
    "home-preload-runtime.json",
    "home-boot.log",
    "lg-native",
    "native-home-gate-state.json",
    "native-home-gate.pending",
    "remote-controls-state.json",
];

/// Foreground apps over which Home may be relaunched in the foreground; any
/// other (or unknown) foreground gets a background `preload` launch instead,
/// so HDMI, casting or a running app never loses focus.
const FOREGROUND_SAFE: [&str; 4] = ["", HOME, "com.webos.app.livetv", "com.webos.app.lgchannels"];

/// The validated manifest file (`home-final-manifest.json`).
pub const MANIFEST_PATH: &str = "/var/lib/lg-native/home-final-manifest.json";
/// The write-ahead journal (`home-mounts.json`).
pub const JOURNAL_PATH: &str = "/var/lib/lg-native/home-mounts.json";
/// The once-per-boot marker (`home-boot.json`).
pub const BOOT_PATH: &str = "/var/lib/lg-native/home-boot.json";
/// Exclusive transaction lock.
pub const LOCK_PATH: &str = "/var/lib/lg-native/home.lock";
/// Persistent disable marker: any entry at this path disables apply.
pub const DISABLED_PATH: &str = "/var/lib/lg-native/home-disabled";
/// Operator-provided evidence that the checkpoints were backed up externally.
pub const BACKUP_PATH: &str = "/var/lib/lg-native/checkpoint-backup.json";
/// Immutable, content-addressed backing copies used as bind sources.
pub const COPIES_DIR: &str = "/var/lib/lg-native/mount-copies";

/// True for one of the eight allowed targets.
pub fn is_allowed(target: &str) -> bool {
    ALLOWED.contains(&target)
}

/// Validates a manifest/journal source path: canonical, plain ASCII, inside
/// the private root and not naming any reserved runtime file.
pub fn source_path(path: Option<&Value>) -> Result<&str> {
    let path = path.and_then(Value::as_str).unwrap_or("");
    let Some(rest) = path.strip_prefix(ROOT).and_then(|r| r.strip_prefix('/')) else {
        return Err(Error::Policy("Source outside owned root".into()));
    };
    ensure!(paths::is_trusted_path(path), "Noncanonical source");
    ensure!(
        !rest.split('/').any(|part| RESERVED.contains(&part)),
        "Reserved runtime path"
    );
    Ok(path)
}

/// Validates a list of `{source, target, originalHash, newHash, ...}` entries.
pub fn validate_entries(entries: &Value, minimum: usize) -> Result<&[Value]> {
    let items = entries.as_array().map(Vec::as_slice).unwrap_or(&[]);
    ensure!(
        entries.as_array().is_some() && minimum <= items.len() && items.len() <= ALLOWED.len(),
        "Invalid entry count"
    );
    let mut sources: Vec<&str> = Vec::with_capacity(items.len());
    let mut targets: Vec<&str> = Vec::with_capacity(items.len());
    for item in items {
        let entry = item
            .as_object()
            .ok_or_else(|| Error::Policy("Entry must be an object".into()))?;
        validate_entry(entry, &mut sources, &mut targets)?;
    }
    Ok(items)
}

fn validate_entry<'a>(
    entry: &'a Map,
    sources: &mut Vec<&'a str>,
    targets: &mut Vec<&'a str>,
) -> Result<()> {
    let target = entry
        .get("target")
        .and_then(Value::as_str)
        .filter(|t| is_allowed(t));
    let target =
        target.ok_or_else(|| Error::Policy("Target is not an allowed Home file".into()))?;
    let source = source_path(entry.get("source"))?;
    ensure!(
        !targets.contains(&target) && !sources.contains(&source),
        "Duplicate source/target"
    );
    sources.push(source);
    targets.push(target);
    for key in ["originalHash", "newHash"] {
        let valid = entry
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(sha::is_hex64);
        ensure!(valid, "Expected lowercase SHA256: {key}");
    }
    Ok(())
}

/// Validates one journal entry on its own.
pub fn validate_single(entry: &Map) -> Result<()> {
    validate_entry(entry, &mut Vec::new(), &mut Vec::new())
}

/// One manifest line: install `source` over the stock `target`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Staged payload under the private root.
    pub source: String,
    /// Allowed stock target.
    pub target: String,
    /// SHA-256 of the stock target.
    pub original_hash: String,
    /// SHA-256 of the payload.
    pub new_hash: String,
}

impl ManifestEntry {
    /// The manifest's JSON form (key order as written by the builder).
    pub fn to_value(&self) -> Value {
        crate::json_object! {
            "source" => self.source.as_str(),
            "target" => self.target.as_str(),
            "originalHash" => self.original_hash.as_str(),
            "newHash" => self.new_hash.as_str(),
        }
    }
}

/// Validates a manifest: 6–8 entries (3–8 with `staging`) with exactly the
/// four manifest keys. Journal identity fields can never be injected.
pub fn validate_manifest(raw: &Value, staging: bool) -> Result<Vec<ManifestEntry>> {
    let items = validate_entries(raw, if staging { 3 } else { 6 })?;
    items
        .iter()
        .map(|item| {
            let entry = item
                .as_object()
                .ok_or_else(|| Error::Policy("Entry must be an object".into()))?;
            ensure!(
                entry.has_exact_keys(&["source", "target", "originalHash", "newHash"]),
                "Manifest must contain only source, target, originalHash, newHash"
            );
            let text = |key: &str| {
                entry
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            Ok(ManifestEntry {
                source: text("source"),
                target: text("target"),
                original_hash: text("originalHash"),
                new_hash: text("newHash"),
            })
        })
        .collect()
}

/// Observed state of one file (from `inspect`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    /// Path that was inspected.
    pub path: String,
    /// Fully resolved path.
    pub realpath: String,
    /// The opened inode is a regular file.
    pub regular: bool,
    /// The opened inode is a symlink.
    pub symlink: bool,
    /// Owner.
    pub uid: u32,
    /// Group.
    pub gid: u32,
    /// Permission bits (`S_IMODE`).
    pub mode: u32,
    /// Hard links.
    pub nlink: u64,
    /// Device.
    pub dev: u64,
    /// Inode.
    pub ino: u64,
    /// SHA-256 of the content.
    pub digest: String,
}

/// Checks that an inspected file is the exact, safe, root-owned file expected.
pub fn validate_file(info: &FileInfo, path: &str, digest: Option<&str>) -> Result<()> {
    ensure!(
        info.path == path && info.realpath == path && info.regular && !info.symlink,
        "Unsafe file: {path}"
    );
    ensure!(
        info.uid == 0 && info.mode & 0o7022 == 0 && info.nlink == 1,
        "Unsafe ownership/mode/link count: {path}"
    );
    if let Some(digest) = digest {
        ensure!(info.digest == digest, "Hash mismatch: {path}");
    }
    Ok(())
}

/// A manifest entry with the stock target's ownership and mode, which the
/// backing copy must reproduce exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedMount {
    /// The manifest entry.
    pub entry: ManifestEntry,
    /// Stock owner.
    pub uid: u32,
    /// Stock group.
    pub gid: u32,
    /// Stock permission bits.
    pub mode: u32,
}

fn find<'a>(infos: &'a [FileInfo], path: &str) -> Result<&'a FileInfo> {
    infos
        .iter()
        .find(|info| info.path == path)
        .ok_or_else(|| Error::Policy(format!("Unsafe file: {path}")))
}

/// Validates EVERY payload and stock hash before any mount is planned.
pub fn plan_mounts(
    manifest: &[ManifestEntry],
    sources: &[FileInfo],
    targets: &[FileInfo],
) -> Result<Vec<PlannedMount>> {
    ensure!(
        (3..=ALLOWED.len()).contains(&manifest.len()),
        "Invalid entry count"
    );
    let mut planned = Vec::with_capacity(manifest.len());
    for entry in manifest {
        validate_file(
            find(sources, &entry.source)?,
            &entry.source,
            Some(&entry.new_hash),
        )?;
        let stock = find(targets, &entry.target)?;
        validate_file(stock, &entry.target, Some(&entry.original_hash))?;
        planned.push(PlannedMount {
            entry: entry.clone(),
            uid: stock.uid,
            gid: stock.gid,
            mode: stock.mode,
        });
    }
    Ok(planned)
}

/// One line of `/proc/self/mountinfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountInfo {
    /// Mount id.
    pub id: u64,
    /// Mount point (octal escapes decoded).
    pub target: String,
    /// Per-mount options (`ro`, `nosuid`, ...).
    pub options: Vec<String>,
}

impl MountInfo {
    /// True when mounted read-only.
    pub fn is_readonly(&self) -> bool {
        self.options.iter().any(|o| o == "ro")
    }
}

fn decode_mount_path(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)));
        match (bytes[i], octal) {
            (b'\\', Some(digits)) => {
                let code = digits
                    .iter()
                    .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
                out.push((code & 0xFF) as u8);
                i += 4;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses `/proc/self/mountinfo`.
pub fn parse_mountinfo(text: &str) -> Result<Vec<MountInfo>> {
    text.lines()
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            ensure!(
                fields.len() >= 10 && fields.contains(&"-"),
                "Malformed mountinfo"
            );
            let id = fields[0]
                .parse()
                .map_err(|_| Error::Policy("Malformed mountinfo".into()))?;
            Ok(MountInfo {
                id,
                target: decode_mount_path(fields[4]),
                options: fields[5].split(',').map(str::to_owned).collect(),
            })
        })
        .collect()
}

/// Mounts at exactly `target`.
pub fn mounts_at<'a>(mounts: &'a [MountInfo], target: &str) -> Vec<&'a MountInfo> {
    mounts.iter().filter(|m| m.target == target).collect()
}

/// Proves from LIVE evidence that `entry`'s mount belongs to this runtime:
/// exactly one mount at the target, payload and target are the same inode with
/// the payload hash, and every recorded identity field still matches.
/// Returns `false` (not an error) when nothing is mounted at the target.
pub fn owned_mount(
    entry: &Map,
    source: &FileInfo,
    target: &FileInfo,
    mounts: &[MountInfo],
    boot_id: &str,
) -> Result<bool> {
    validate_single(entry)?;
    let target_path = entry
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source_path = entry
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new_hash = entry.get("newHash").and_then(Value::as_str);
    let exact = mounts_at(mounts, target_path);
    let Some(mount) = exact.first() else {
        return Ok(false);
    };
    ensure!(exact.len() == 1, "Stacked mount refused: {target_path}");
    validate_file(source, source_path, new_hash)?;
    validate_file(target, target_path, new_hash)?;
    ensure!(
        (source.dev, source.ino) == (target.dev, target.ino),
        "Mount is not owned: {target_path}"
    );
    let expected = [
        ("sourceDev", Value::from(source.dev)),
        ("sourceIno", Value::from(source.ino)),
        ("mountId", Value::from(mount.id)),
        ("bootId", Value::from(boot_id)),
    ];
    for (key, value) in expected {
        if let Some(recorded) = entry.get(key).filter(|v| !v.is_null()) {
            ensure!(*recorded == value, "Ownership identity mismatch: {key}");
        }
    }
    Ok(true)
}

/// The `applicationManager/launch` payload. Home is relaunched in the
/// background (`preload: full`) unless the previous foreground was Home, Live
/// TV, LG Channels or explicitly empty.
pub fn launch_payload(previous_foreground: Option<&str>, background: bool) -> Value {
    let mut payload = Map::new();
    payload.insert("id", HOME);
    let safe = previous_foreground.is_some_and(|app| FOREGROUND_SAFE.contains(&app));
    if background || !safe {
        payload.insert("preload", "full");
    }
    Value::Object(payload)
}

/// Process ids whose executable is exactly the Home binary, sorted.
pub fn home_pids(executables: &[(i32, String)]) -> Vec<i32> {
    let mut pids: Vec<i32> = executables
        .iter()
        .filter(|(_, exe)| exe == EXE)
        .map(|(pid, _)| *pid)
        .collect();
    pids.sort_unstable();
    pids
}

/// Name of the immutable backing copy for a planned mount. Identical to the
/// previous implementation (`sha256(json.dumps([...], separators=(',', ':')))`),
/// so existing copies are reused rather than duplicated.
pub fn copy_id(target: &str, new_hash: &str, uid: u32, gid: u32, mode: u32) -> String {
    let identity = Value::Array(vec![
        Value::from(target),
        Value::from(new_hash),
        Value::from(uid),
        Value::from(gid),
        Value::from(mode),
    ]);
    sha::hex(json::compact(&identity))
}

/// The write-ahead journal, `home-mounts.json`.
///
/// Mount entries stay as JSON objects so any field written by an earlier
/// version round-trips untouched.
#[derive(Clone, Debug, PartialEq)]
pub struct Journal {
    /// Top-level fields other than `mounts` (`version`, `owner`, `preload`,
    /// `action`, `actionTarget`, ...).
    pub fields: Map,
    /// Mount entries in application order.
    pub mounts: Vec<Map>,
}

/// The recorded preload intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preload {
    /// Policy value before this runtime changed it.
    pub original: bool,
    /// Boot in which the intent was recorded.
    pub boot_id: String,
    /// The original value still has to be restored.
    pub pending: bool,
}

impl Journal {
    /// A fresh journal with no mounts.
    pub fn empty() -> Self {
        let mut fields = Map::new();
        fields.insert("version", 1u64);
        fields.insert("owner", OWNER);
        Journal {
            fields,
            mounts: Vec::new(),
        }
    }

    /// The JSON document.
    pub fn to_value(&self) -> Value {
        let mut map = self.fields.clone();
        map.insert(
            "mounts",
            Value::Array(self.mounts.iter().cloned().map(Value::Object).collect()),
        );
        Value::Object(map)
    }

    /// The validated preload record, if any.
    pub fn preload(&self) -> Option<Preload> {
        let record = self.fields.get("preload")?.as_object()?;
        Some(Preload {
            original: record.get("original")?.as_bool()?,
            boot_id: record.get("bootId")?.as_str()?.to_owned(),
            pending: record.get("pending")?.as_bool()?,
        })
    }

    /// Replaces the preload record.
    pub fn set_preload(&mut self, preload: &Preload) {
        let mut record = Map::new();
        record.insert("original", preload.original);
        record.insert("bootId", preload.boot_id.as_str());
        record.insert("pending", preload.pending);
        self.fields.insert("preload", record);
    }

    /// Marks the preload record as restored, keeping any other recorded field.
    pub fn complete_preload(&mut self) {
        if let Some(record) = self
            .fields
            .get_mut("preload")
            .and_then(Value::as_object_mut)
        {
            record.insert("pending", false);
        }
    }
}

/// Parses and validates the journal. A missing journal is empty; a legacy list
/// of entries is migrated, but still has to prove ownership from live state.
pub fn validate_journal(raw: Option<Value>) -> Result<Journal> {
    let Some(raw) = raw else {
        return Ok(Journal::empty());
    };
    if raw.as_array().is_some() {
        validate_entries(&raw, 0)?;
        let mut journal = Journal::empty();
        journal.mounts = into_maps(raw);
        return Ok(journal);
    }
    let Value::Object(mut fields) = raw else {
        return Err(Error::Policy("Unknown journal owner/version".into()));
    };
    ensure!(
        fields.get("version") == Some(&Value::from(1u64))
            && fields.get("owner").and_then(Value::as_str) == Some(OWNER),
        "Unknown journal owner/version"
    );
    let mounts = fields.remove("mounts").unwrap_or(Value::Null);
    validate_entries(&mounts, 0)?;
    if let Some(preload) = fields.get("preload").filter(|p| !p.is_null()) {
        let valid = preload.as_object().is_some_and(|p| {
            p.get("original").and_then(Value::as_bool).is_some()
                && p.get("pending").and_then(Value::as_bool).is_some()
                && p.get("bootId").and_then(Value::as_str).is_some()
        });
        ensure!(valid, "Invalid preload recovery record");
    }
    Ok(Journal {
        fields,
        mounts: into_maps(mounts),
    })
}

fn into_maps(list: Value) -> Vec<Map> {
    match list {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| match item {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// String field of an object value (empty when absent).
pub fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// String field of a journal entry (empty when absent).
pub fn text<'a>(entry: &'a Map, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Combines the kernel boot UUID with the Homebrew Channel startup marker's
/// mtime: LG fast boot restores the kernel UUID, the marker is per boot.
pub fn composite_boot_id(kernel_uuid: &str, marker: &Meta) -> Result<String> {
    let uuid = kernel_uuid.trim();
    ensure!(
        uuid.len() == 36
            && uuid
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-'),
        "Invalid kernel boot ID"
    );
    ensure!(
        marker.is_regular() && marker.uid == 0 && marker.nlink == 1 && marker.mode & 0o022 == 0,
        "Untrusted HBC startup marker"
    );
    Ok(format!("{uuid}:{}", marker.mtime_ns))
}

/// Per-mount flags to keep when remounting read-only, from the live options.
/// Like util-linux, `remount,bind,ro` must not silently clear `nosuid`,
/// `nodev`, `noexec` or the access-time policy.
pub fn preserved_mount_flags(options: &[String]) -> u64 {
    // Linux MS_* values (stable kernel ABI).
    const MS_NOSUID: u64 = 2;
    const MS_NODEV: u64 = 4;
    const MS_NOEXEC: u64 = 8;
    const MS_NOATIME: u64 = 1024;
    const MS_NODIRATIME: u64 = 2048;
    const MS_NOSYMFOLLOW: u64 = 256;
    const MS_RELATIME: u64 = 1 << 21;
    options.iter().fold(0, |flags, option| {
        flags
            | match option.as_str() {
                "nosuid" => MS_NOSUID,
                "nodev" => MS_NODEV,
                "noexec" => MS_NOEXEC,
                "noatime" => MS_NOATIME,
                "nodiratime" => MS_NODIRATIME,
                "relatime" => MS_RELATIME,
                "nosymfollow" => MS_NOSYMFOLLOW,
                _ => 0,
            }
    })
}
