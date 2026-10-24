//! The real device operations: fixed paths, no configurable root, no shell,
//! no broad deletes. Mounts use the `mount(2)` / `umount2(2)` system calls
//! directly instead of spawning `mount` three times per file.

use super::controller::{HomeOps, HomeReader};
use super::model::{
    self, composite_boot_id, home_pids, parse_mountinfo, preserved_mount_flags, source_path, text,
    validate_file, FileInfo, MountInfo, BACKUP_PATH, BOOT_PATH, COPIES_DIR, DISABLED_PATH, EXE,
    HOME, JOURNAL_PATH, LOCK_PATH, MANIFEST_PATH,
};
use crate::checkpoint;
use crate::ensure;
use crate::error::{Error, Result};
use crate::fsx::{self, MIB};
use crate::json::{self, Map, Style, Value};
use crate::luna;
use crate::paths::{self, ROOT};
use crate::process::Limits;
use crate::sha::{Hasher, BLOCK};
use crate::signals;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::time::Duration;

/// Default LS2 call budget.
const LUNA_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest accepted LS2 response.
const LUNA_OUTPUT_LIMIT: usize = 1024 * 1024;
/// Marker recreated by Homebrew Channel on every boot.
const STARTUP_MARKER: &str = "/tmp/webosbrew_startup";
/// Present while CRIU restores a checkpointed process.
const CRIU_RESTORING: &str = "/tmp/criu_restoring";

/// The device implementation of [`HomeOps`].
#[derive(Debug)]
pub struct LinuxOps {
    boot_id: String,
    lock: Option<File>,
}

/// `secure_parents` of the Home runtime: canonical path, every ancestor a
/// root-owned directory that group/other cannot write.
pub fn secure_parents(path: &str) -> Result<()> {
    ensure!(paths::is_canonical_absolute(path), "Noncanonical path");
    fsx::for_each_parent(path, |parent, meta| {
        ensure!(
            fsx::is_safe_parent(meta),
            "Unsafe parent directory: {parent}"
        );
        Ok(())
    })
}

impl LinuxOps {
    /// Checks the platform and private root, then derives the boot identity.
    pub fn new() -> Result<Self> {
        ensure!(
            cfg!(target_os = "linux") && fsx::euid() == 0,
            "Requires root on Linux; not a host installer"
        );
        secure_parents(&paths::in_root("sentinel"))?;
        ensure!(
            fsx::lstat(ROOT)?.permissions() == 0o700,
            "Owned root must be mode 0700"
        );
        let uuid = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| Error::io("read kernel boot ID", e))?;
        let boot_id = composite_boot_id(&uuid, &fsx::lstat(STARTUP_MARKER)?)?;
        Ok(LinuxOps {
            boot_id,
            lock: None,
        })
    }

    /// Inspects and validates a trusted file, optionally against a hash.
    pub fn trusted_file(&self, path: &str, digest: Option<&str>) -> Result<FileInfo> {
        let info = self.inspect(path)?;
        validate_file(&info, path, digest)?;
        Ok(info)
    }

    /// Reads a trusted (root-owned, single-link, ≤ 1 MiB) JSON file without
    /// hashing it; duplicate keys are rejected.
    pub fn read_json(&self, path: &str, optional: bool) -> Result<Option<Value>> {
        if optional && !fsx::lexists(path) {
            return Ok(None);
        }
        secure_parents(path)?;
        let before = fsx::lstat(path)?;
        ensure!(before.is_regular(), "Not a regular nonsymlink file: {path}");
        let (mut file, opened) = fsx::open_verified(
            path,
            &before,
            false,
            |_| true,
            "",
            "File changed during open",
        )?;
        ensure!(
            fsx::realpath(path).as_deref() == Some(path),
            "Unsafe file: {path}"
        );
        ensure!(
            opened.is_regular()
                && opened.uid == 0
                && opened.permissions() & 0o7022 == 0
                && opened.nlink == 1,
            "Unsafe ownership/mode/link count: {path}"
        );
        ensure!(opened.size <= MIB, "Oversized state file");
        let data = fsx::read_capped(&mut file, MIB, path)?;
        ensure!(data.len() as u64 <= MIB, "Oversized state file");
        json::parse_bytes(&data)
            .map(Some)
            .map_err(|e| Error::Policy(e.to_string()))
    }

    /// Atomically writes one of the two runtime state files, then reads it back.
    pub fn write_json(&self, path: &str, data: &Value) -> Result<()> {
        ensure!(
            path == JOURNAL_PATH || path == BOOT_PATH,
            "Unowned state path"
        );
        secure_parents(path)?;
        if fsx::lexists(path) {
            self.trusted_file(path, None)?;
        }
        let temporary = format!("{path}.tmp-{}", fsx::random_hex(16)?);
        let result = (|| {
            let mut file = fsx::create_exclusive(&temporary, 0o600)?;
            let mut text = json::to_string(data, Style::Indented, true);
            text.push('\n');
            fsx::write_synced(&mut file, text.as_bytes(), &temporary)?;
            drop(file);
            fsx::rename(&temporary, path)?;
            fsx::fsync_dir(ROOT)?;
            ensure!(
                self.read_json(path, false)?.as_ref() == Some(data),
                "Journal readback mismatch"
            );
            Ok(())
        })();
        if fsx::lexists(&temporary) {
            let _ = fsx::unlink(&temporary);
        }
        result
    }

    /// Takes the exclusive, non-blocking transaction lock.
    #[allow(unsafe_code)]
    pub fn lock(&mut self) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        secure_parents(LOCK_PATH)?;
        if fsx::lexists(LOCK_PATH) {
            self.trusted_file(LOCK_PATH, None)?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(LOCK_PATH)
            .map_err(|e| Error::io(format!("open {LOCK_PATH}"), e))?;
        // SAFETY: flock on a valid descriptor owned by `file`.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::io(
                "lock Home runtime",
                std::io::Error::last_os_error(),
            ));
        }
        self.lock = Some(file);
        Ok(())
    }

    /// Releases the lock (closing the descriptor releases the `flock`).
    pub fn unlock(&mut self) {
        self.lock = None;
    }

    /// Bounded LS2 call that requires `returnValue: true`.
    pub fn luna(&self, method: &str, payload: &Value, timeout: Duration) -> Result<Value> {
        let limits = Limits {
            timeout,
            max_output: LUNA_OUTPUT_LIMIT,
            capture_stderr: true,
            interruptible: true,
        };
        let output = luna::send(method, payload, limits)?;
        ensure!(
            output.status.success(),
            "Command failed: luna-send {method}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let response = json::parse_bytes(&output.stdout).map_err(|e| Error::Data(e.to_string()))?;
        ensure!(
            response.get("returnValue").and_then(Value::as_bool) == Some(true),
            "LS2 failure: {}",
            json::compact(&response)
        );
        Ok(response)
    }

    /// Records one apply attempt per boot; `false` when already attempted.
    pub fn claim_boot(&mut self) -> Result<bool> {
        let saved = self.read_json(BOOT_PATH, true)?;
        if super::boot_already_claimed(saved.as_ref(), &self.boot_id) {
            return Ok(false);
        }
        let record = crate::json_object! {"bootId" => self.boot_id.as_str(), "attempted" => true};
        self.write_json(BOOT_PATH, &record)?;
        Ok(true)
    }

    /// Waits (bounded) until CRIU is idle and both LS2 services answer.
    pub fn wait_ready(&mut self, budget: f64) -> Result<()> {
        super::wait_ready(&mut DeviceProbe { ops: self }, budget)
    }

    fn mount_options(&self, target: &str) -> Result<Vec<String>> {
        Ok(model::mounts_at(&self.mounts()?, target)
            .first()
            .map(|m| m.options.clone())
            .unwrap_or_default())
    }
}

struct DeviceProbe<'a> {
    ops: &'a LinuxOps,
}

impl super::ReadinessProbe for DeviceProbe<'_> {
    fn now(&self) -> f64 {
        monotonic_seconds()
    }
    fn sleep(&mut self, seconds: f64) -> Result<()> {
        signals::sleep(Duration::from_secs_f64(seconds.max(0.0)))
    }
    fn criu_restoring(&self) -> bool {
        fsx::lexists(CRIU_RESTORING)
    }
    fn call(&mut self, method: &str, timeout: f64) -> Result<()> {
        self.ops
            .luna(
                method,
                &Value::Object(Map::new()),
                Duration::from_secs_f64(timeout.max(0.001)),
            )
            .map(drop)
    }
}

fn monotonic_seconds() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn cstring(path: &str) -> Result<CString> {
    CString::new(path).map_err(|_| Error::Policy(format!("Invalid path: {path}")))
}

#[allow(unsafe_code)]
fn sys_mount(source: Option<&str>, target: &str, flags: libc::c_ulong) -> Result<()> {
    let source = source.map(cstring).transpose()?;
    let target_c = cstring(target)?;
    // SAFETY: all pointers are valid NUL-terminated strings or null; no data
    // argument is passed for bind/remount operations.
    let rc = unsafe {
        libc::mount(
            source.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            target_c.as_ptr(),
            std::ptr::null(),
            flags,
            std::ptr::null(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(Error::io(
            format!("mount {target}"),
            std::io::Error::last_os_error(),
        ))
    }
}

impl HomeReader for LinuxOps {
    fn boot_id(&self) -> &str {
        &self.boot_id
    }

    fn is_disabled(&self) -> bool {
        // Any entry, even an unexpected symlink, disables apply; never followed.
        fsx::lexists(DISABLED_PATH)
    }

    fn load_manifest(&self) -> Result<Value> {
        self.read_json(MANIFEST_PATH, false)?
            .ok_or_else(|| Error::Policy("Manifest missing".into()))
    }

    fn load_journal(&self) -> Result<Option<Value>> {
        self.read_json(JOURNAL_PATH, true)
    }

    fn mounts(&self) -> Result<Vec<MountInfo>> {
        let text = std::fs::read_to_string("/proc/self/mountinfo")
            .map_err(|e| Error::io("read /proc/self/mountinfo", e))?;
        parse_mountinfo(&text)
    }

    fn inspect(&self, path: &str) -> Result<FileInfo> {
        inspect_file(path)
    }
}

impl HomeOps for LinuxOps {
    fn save_journal(&mut self, journal: &Value) -> Result<()> {
        self.write_json(JOURNAL_PATH, journal)
    }

    fn disable(&mut self) -> Result<()> {
        if fsx::lexists(DISABLED_PATH) {
            return Ok(());
        }
        let file = fsx::create_exclusive(DISABLED_PATH, 0o600)?;
        file.sync_all()
            .map_err(|e| Error::io(format!("fsync {DISABLED_PATH}"), e))?;
        drop(file);
        fsx::fsync_dir(ROOT)?;
        ensure!(self.is_disabled(), "Disable marker readback failed");
        Ok(())
    }

    fn foreground(&mut self) -> Result<Option<String>> {
        let response = self.luna(
            "com.webos.applicationManager/getForegroundAppInfo",
            &Value::Object(Map::new()),
            LUNA_TIMEOUT,
        )?;
        Ok(response
            .get("appId")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    fn preload(&mut self) -> Result<bool> {
        let response = self.luna(
            "com.webos.service.preloadmanager/getPreloadPolicy",
            &Value::Object(Map::new()),
            LUNA_TIMEOUT,
        )?;
        let applications = response
            .get("applications")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let homes: Vec<&Value> = applications
            .iter()
            .filter(|app| app.get("id").and_then(Value::as_str) == Some(HOME))
            .collect();
        let enabled = match homes.as_slice() {
            [home] => home.get("isEnabled").and_then(Value::as_bool),
            _ => None,
        };
        enabled.ok_or_else(|| Error::Policy("Invalid Home preload response".into()))
    }

    fn set_preload(&mut self, enabled: bool) -> Result<()> {
        let payload = crate::json_object! {
            "application" => crate::json_object! {"id" => HOME, "isEnabled" => enabled},
        };
        self.luna(
            "com.webos.service.preloadmanager/setPreloadPolicy",
            &payload,
            LUNA_TIMEOUT,
        )?;
        ensure!(
            self.preload()? == enabled,
            "Preload change readback mismatch"
        );
        Ok(())
    }

    fn copy_source(&mut self, entry: &Map) -> Result<()> {
        let input = source_path(entry.get("inputSource"))?.to_owned();
        let copy = source_path(entry.get("source"))?.to_owned();
        let new_hash = text(entry, "newHash").to_owned();
        let identity = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
        };
        let (Some(uid), Some(gid), Some(mode)) =
            (identity("uid"), identity("gid"), identity("mode"))
        else {
            return Err(Error::Policy("Invalid copy metadata".into()));
        };
        self.trusted_file(&input, Some(&new_hash))?;
        if fsx::lexists(&copy) {
            // Immutable content-addressed copy: never truncate or rewrite a
            // backing inode that may already be mounted.
            let meta = self.trusted_file(&copy, Some(&new_hash))?;
            ensure!(
                (meta.uid, meta.gid, meta.mode) == (uid, gid, mode),
                "Existing copy metadata mismatch"
            );
            return Ok(());
        }
        if !fsx::lexists(COPIES_DIR) {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(COPIES_DIR)
                .map_err(|e| Error::io(format!("mkdir {COPIES_DIR}"), e))?;
            fsx::fsync_dir(ROOT)?;
        }
        secure_parents(&copy)?;
        // Build the copy under a temporary name and only link it into place
        // when its hash matches, so a failed copy never leaves a corrupt
        // file at the content-addressed path.
        let temporary = format!(
            "{COPIES_DIR}/.{}.tmp-{}",
            paths::file_name(&copy),
            fsx::random_hex(8)?
        );
        let result = (|| {
            let before = fsx::lstat(&input)?;
            let (mut reader, _) = fsx::open_verified(
                &input,
                &before,
                false,
                |_| true,
                "",
                "File changed during open",
            )?;
            let mut output = fsx::create_exclusive(&temporary, 0o600)?;
            let mut hasher = Hasher::new();
            let mut block = [0u8; BLOCK];
            loop {
                let read = reader
                    .read(&mut block)
                    .map_err(|e| Error::io(format!("read {input}"), e))?;
                if read == 0 {
                    break;
                }
                hasher.update(&block[..read]);
                output
                    .write_all(&block[..read])
                    .map_err(|e| Error::io(format!("write {temporary}"), e))?;
            }
            ensure!(hasher.hex() == new_hash, "Hash mismatch: {input}");
            std::os::unix::fs::fchown(&output, Some(uid), Some(gid))
                .map_err(|e| Error::io(format!("chown {temporary}"), e))?;
            output
                .set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(|e| Error::io(format!("chmod {temporary}"), e))?;
            output
                .sync_all()
                .map_err(|e| Error::io(format!("fsync {temporary}"), e))?;
            drop(output);
            std::fs::hard_link(&temporary, &copy)
                .map_err(|e| Error::io(format!("link {copy}"), e))?;
            Ok(())
        })();
        if fsx::lexists(&temporary) {
            let _ = fsx::unlink(&temporary);
        }
        result?;
        fsx::fsync_dir(COPIES_DIR)?;
        let meta = self.trusted_file(&copy, Some(&new_hash))?;
        ensure!(
            (meta.uid, meta.gid, meta.mode) == (uid, gid, mode),
            "Copied metadata mismatch"
        );
        Ok(())
    }

    fn bind(&mut self, entry: &Map) -> Result<()> {
        const MS_BIND: libc::c_ulong = 4096;
        sys_mount(Some(text(entry, "source")), text(entry, "target"), MS_BIND)
    }

    fn readonly(&mut self, entry: &Map) -> Result<()> {
        const MS_RDONLY: u64 = 1;
        const MS_REMOUNT: u64 = 32;
        const MS_BIND: u64 = 4096;
        let target = text(entry, "target");
        let flags =
            MS_REMOUNT | MS_BIND | MS_RDONLY | preserved_mount_flags(&self.mount_options(target)?);
        sys_mount(None, target, flags as libc::c_ulong)
    }

    #[allow(unsafe_code)]
    fn unmount(&mut self, entry: &Map) -> Result<()> {
        let target = text(entry, "target");
        let target_c = cstring(target)?;
        // SAFETY: valid NUL-terminated path; no flags (plain unmount).
        if unsafe { libc::umount2(target_c.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(Error::io(
                format!("umount {target}"),
                std::io::Error::last_os_error(),
            ))
        }
    }

    fn clear_checkpoint(&mut self) -> Result<()> {
        self.trusted_file(BACKUP_PATH, None)?;
        let plan = checkpoint::plan()?;
        checkpoint::apply(&plan, BACKUP_PATH)
    }

    #[allow(unsafe_code)]
    fn kill_home(&mut self) -> Result<()> {
        for pid in home_pids(&executables()?) {
            // Recheck the exact executable immediately before the signal;
            // never pkill/killall by name.
            if exe_of(pid).as_deref() == Some(EXE) {
                // SAFETY: kill with a concrete positive pid and a valid signal.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        signals::sleep(Duration::from_millis(500))
    }

    fn launch(&mut self, payload: &Value) -> Result<()> {
        self.luna("com.webos.applicationManager/launch", payload, LUNA_TIMEOUT)
            .map(drop)
    }
}

/// Securely inspects and hashes one file: safe parents, `lstat` regular file,
/// `O_NOFOLLOW` open, same inode after `fstat`, streaming SHA-256.
pub fn inspect_file(path: &str) -> Result<FileInfo> {
    secure_parents(path)?;
    let before = fsx::lstat(path)?;
    ensure!(before.is_regular(), "Not a regular nonsymlink file: {path}");
    let (mut file, opened) = fsx::open_verified(
        path,
        &before,
        false,
        |_| true,
        "",
        "File changed during open",
    )?;
    let mut hasher = Hasher::new();
    let mut block = [0u8; BLOCK];
    loop {
        let read = file
            .read(&mut block)
            .map_err(|e| Error::io(format!("read {path}"), e))?;
        if read == 0 {
            break;
        }
        hasher.update(&block[..read]);
    }
    Ok(FileInfo {
        path: path.to_owned(),
        realpath: fsx::realpath(path).unwrap_or_default(),
        regular: opened.is_regular(),
        symlink: opened.is_symlink(),
        uid: opened.uid,
        gid: opened.gid,
        mode: opened.permissions(),
        nlink: opened.nlink,
        dev: opened.dev,
        ino: opened.ino,
        digest: hasher.hex(),
    })
}

fn exe_of(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
}

fn executables() -> Result<Vec<(i32, String)>> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(|e| Error::io("list /proc", e))? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some(exe) = exe_of(pid) {
            result.push((pid, exe));
        }
    }
    Ok(result)
}
