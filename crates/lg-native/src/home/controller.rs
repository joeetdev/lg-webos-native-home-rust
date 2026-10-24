//! The Home transaction. Side effects are confined to [`HomeOps`]; every
//! intent is durably journaled BEFORE the side effect it describes, and every
//! failure unwinds through a recovery that restores stock Home.

use super::model::{
    self, copy_id, launch_payload, mounts_at, owned_mount, plan_mounts, text, validate_file,
    validate_journal, validate_manifest, FileInfo, Journal, MountInfo, Preload, ALLOWED,
    COPIES_DIR,
};
use crate::ensure;
use crate::error::{Error, Result};
use crate::json::{Map, Value};
use crate::signals;

/// Read-only view of the device. The gate's readiness proof is written
/// against this trait alone, so it cannot mutate anything by construction.
pub trait HomeReader {
    /// Composite identity of the current boot.
    fn boot_id(&self) -> &str;
    /// True when the persistent disable marker exists.
    fn is_disabled(&self) -> bool;
    /// The raw manifest document.
    fn load_manifest(&self) -> Result<Value>;
    /// The raw journal document, `None` when absent.
    fn load_journal(&self) -> Result<Option<Value>>;
    /// Current mount table.
    fn mounts(&self) -> Result<Vec<MountInfo>>;
    /// Securely inspects and hashes one file.
    fn inspect(&self, path: &str) -> Result<FileInfo>;
}

/// Every side effect the transaction may perform.
pub trait HomeOps: HomeReader {
    /// Durably replaces the journal (with read-back verification).
    fn save_journal(&mut self, journal: &Value) -> Result<()>;
    /// Creates the persistent disable marker.
    fn disable(&mut self) -> Result<()>;
    /// Foreground application id (`None` when the service reports none).
    fn foreground(&mut self) -> Result<Option<String>>;
    /// Home preload policy.
    fn preload(&mut self) -> Result<bool>;
    /// Changes and verifies the Home preload policy.
    fn set_preload(&mut self, enabled: bool) -> Result<()>;
    /// Creates (or verifies) the immutable backing copy for an entry.
    fn copy_source(&mut self, entry: &Map) -> Result<()>;
    /// Bind-mounts the entry's backing copy over its target.
    fn bind(&mut self, entry: &Map) -> Result<()>;
    /// Remounts the entry's bind mount read-only.
    fn readonly(&mut self, entry: &Map) -> Result<()>;
    /// Unmounts the entry's target.
    fn unmount(&mut self, entry: &Map) -> Result<()>;
    /// Removes the derived Home checkpoints.
    fn clear_checkpoint(&mut self) -> Result<()>;
    /// Kills processes running exactly the Home executable.
    fn kill_home(&mut self) -> Result<()>;
    /// Launches Home with the given payload.
    fn launch(&mut self, payload: &Value) -> Result<()>;
}

/// Outcome of a successful command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Overrides applied.
    Applied,
    /// Apply skipped: the persistent disable marker exists.
    Disabled,
    /// Stock Home restored.
    RolledBack,
}

impl Outcome {
    /// Text printed by the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Applied => "applied",
            Outcome::Disabled => "disabled",
            Outcome::RolledBack => "rolled-back",
        }
    }
}

/// The transaction over a set of device operations.
#[derive(Debug)]
pub struct Controller<'a, O: HomeOps> {
    ops: &'a mut O,
    journal: Journal,
}

impl<'a, O: HomeOps> Controller<'a, O> {
    /// A controller with an empty in-memory journal (call [`Self::load`]).
    pub fn new(ops: &'a mut O) -> Self {
        Controller {
            ops,
            journal: Journal::empty(),
        }
    }

    /// The in-memory journal.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Loads and validates the journal.
    pub fn load(&mut self) -> Result<()> {
        self.journal = validate_journal(self.ops.load_journal()?)?;
        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        let document = self.journal.to_value();
        self.ops.save_journal(&document)
    }

    /// Records `action` (and the entry it applies to) before performing it.
    /// A pending SIGTERM unwinds here, before the next side effect starts.
    fn intent(&mut self, action: &str, index: Option<usize>) -> Result<()> {
        signals::check()?;
        self.record(action, index)
    }

    fn record(&mut self, action: &str, index: Option<usize>) -> Result<()> {
        self.journal.fields.insert("action", action);
        let target = index
            .and_then(|i| self.journal.mounts.get(i))
            .and_then(|entry| entry.get("target").cloned())
            .unwrap_or(Value::Null);
        self.journal.fields.insert("actionTarget", target);
        if let Some(entry) = index.and_then(|i| self.journal.mounts.get_mut(i)) {
            entry.insert("state", action);
        }
        self.save()
    }

    fn entry(&self, index: usize) -> Result<Map> {
        self.journal
            .mounts
            .get(index)
            .cloned()
            .ok_or_else(|| Error::Policy("Journal entry vanished".into()))
    }

    fn set_field(&mut self, index: usize, key: &str, value: impl Into<Value>) {
        if let Some(entry) = self.journal.mounts.get_mut(index) {
            entry.insert(key, value);
        }
    }

    /// True only when live mount, inode, hash and identity evidence prove the
    /// entry's mount belongs to this runtime.
    pub fn is_owned(&self, entry: &Map) -> Result<bool> {
        let mounts = self.ops.mounts()?;
        let target = text(entry, "target");
        if mounts_at(&mounts, target).is_empty() {
            return Ok(false);
        }
        let source = self.ops.inspect(text(entry, "source"))?;
        let stock = self.ops.inspect(target)?;
        owned_mount(entry, &source, &stock, &mounts, self.ops.boot_id())
    }

    /// Unmounts every provably owned mount, newest first. Unowned or foreign
    /// mounts are left alone; failures are collected so one bad entry does not
    /// stop the cleanup of the others.
    fn remove_owned(&mut self) -> Result<()> {
        let mut errors: Vec<String> = Vec::new();
        for index in (0..self.journal.mounts.len()).rev() {
            match self.remove_one(index) {
                Ok(()) => {}
                Err(Error::Interrupted) => return Err(Error::Interrupted),
                Err(error) => errors.push(error.to_string()),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Policy(format!(
                "Owned cleanup incomplete: {}",
                errors.join("; ")
            )))
        }
    }

    fn remove_one(&mut self, index: usize) -> Result<()> {
        let entry = self.entry(index)?;
        if !self.is_owned(&entry)? {
            return Ok(());
        }
        self.intent("unmount", Some(index))?;
        self.ops.unmount(&self.entry(index)?)?;
        let target = text(&entry, "target");
        ensure!(
            mounts_at(&self.ops.mounts()?, target).is_empty(),
            "Unmount did not remove override"
        );
        self.set_field(index, "state", "removed");
        self.save()
    }

    /// Refuses any mount on, or above, an allowed target (a directory
    /// override hides the firmware just like a file bind), except `/` and `/usr`.
    fn assert_no_overrides(&self) -> Result<()> {
        for mount in self.ops.mounts()? {
            if mount.target == "/" || mount.target == "/usr" {
                continue;
            }
            let prefix = format!("{}/", mount.target.trim_end_matches('/'));
            let hides = ALLOWED
                .iter()
                .any(|t| *t == mount.target || t.starts_with(&prefix));
            ensure!(!hides, "Unexpected Home/ancestor mount: {}", mount.target);
        }
        Ok(())
    }

    /// Records the current preload policy (keeping an unrestored value from an
    /// earlier attempt in the same boot), then disables preload.
    fn begin_preload(&mut self) -> Result<()> {
        let mut previous = self.ops.preload()?;
        let boot_id = self.ops.boot_id().to_owned();
        if let Some(saved) = self.journal.preload() {
            if saved.pending && saved.boot_id == boot_id {
                previous = saved.original;
            }
        }
        self.journal.set_preload(&Preload {
            original: previous,
            boot_id,
            pending: true,
        });
        self.intent("preload-disable", None)?;
        self.ops.set_preload(false)
    }

    /// Restores a pending preload value from THIS boot. A journal write failure
    /// must not prevent the restore itself, and an interruption never skips it.
    fn restore_preload(&mut self) -> Result<()> {
        let Some(saved) = self.journal.preload() else {
            return Ok(());
        };
        if !saved.pending || saved.boot_id != self.ops.boot_id() {
            return Ok(());
        }
        let journal_error = self.record("preload-restore", None).err();
        self.ops.set_preload(saved.original)?;
        ensure!(
            self.ops.preload()? == saved.original,
            "Home preload restore readback failed"
        );
        self.journal.complete_preload();
        self.save()?;
        journal_error.map_or(Ok(()), Err)
    }

    /// Unknown must never become the explicitly safe empty foreground.
    fn foreground_or_unknown(&mut self) -> Option<String> {
        self.ops.foreground().ok().flatten()
    }

    fn restart(&mut self, previous: Option<&str>, background: bool) -> Result<()> {
        self.intent("clear-checkpoint", None)?;
        self.ops.clear_checkpoint()?;
        self.intent("kill-exact-home", None)?;
        self.ops.kill_home()?;
        // Recheck: the user may have switched to HDMI or casting meanwhile.
        let current = self.foreground_or_unknown();
        let needs_preload = |app: Option<&str>| launch_payload(app, false).get("preload").is_some();
        let force_background =
            background || needs_preload(previous) || needs_preload(current.as_deref());
        let payload = launch_payload(previous, force_background);
        self.intent("launch-home", None)?;
        self.ops.launch(&payload)
    }

    fn disable(&mut self) -> Result<()> {
        self.intent("disable", None)?;
        self.ops.disable()
    }

    /// Applies the manifest. Any failure disables the overrides, removes owned
    /// mounts and relaunches stock Home in the background before reporting.
    pub fn apply(&mut self, staging: bool) -> Result<Outcome> {
        if self.ops.is_disabled() {
            return Ok(Outcome::Disabled);
        }
        self.load()?;
        let previous = self.foreground_or_unknown();
        let result = match self.apply_body(staging, previous.as_deref()) {
            Ok(()) => Ok(Outcome::Applied),
            Err(original) => match self.recover(previous.as_deref()) {
                Ok(()) => Err(original),
                Err(recovery) => Err(Error::Policy(format!(
                    "Apply failed: {original}; rollback incomplete: {recovery}"
                ))),
            },
        };
        self.restore_preload()?;
        result
    }

    fn recover(&mut self, previous: Option<&str>) -> Result<()> {
        self.disable()?;
        self.remove_owned()?;
        self.assert_no_overrides()?;
        self.restart(previous, true)
    }

    fn apply_body(&mut self, staging: bool, previous: Option<&str>) -> Result<()> {
        self.begin_preload()?;
        let manifest = validate_manifest(&self.ops.load_manifest()?, staging)?;
        let mut sources = Vec::with_capacity(manifest.len());
        for entry in &manifest {
            sources.push(self.ops.inspect(&entry.source)?);
        }
        for (entry, info) in manifest.iter().zip(&sources) {
            validate_file(info, &entry.source, Some(&entry.new_hash))?;
        }
        self.remove_owned()?;
        self.assert_no_overrides()?;
        let mut targets = Vec::with_capacity(manifest.len());
        for entry in &manifest {
            targets.push(self.ops.inspect(&entry.target)?);
        }
        // ALL stock hashes are verified before ANY mount.
        let plan = plan_mounts(&manifest, &sources, &targets)?;
        self.journal.mounts.clear();
        for planned in plan {
            self.mount_one(&planned)?;
        }
        self.restart(previous, false)
    }

    fn mount_one(&mut self, planned: &model::PlannedMount) -> Result<()> {
        let e = &planned.entry;
        let copy = format!(
            "{COPIES_DIR}/{}",
            copy_id(
                &e.target,
                &e.new_hash,
                planned.uid,
                planned.gid,
                planned.mode
            )
        );
        let mut entry = Map::new();
        entry.insert("source", copy.as_str());
        entry.insert("target", e.target.as_str());
        entry.insert("originalHash", e.original_hash.as_str());
        entry.insert("newHash", e.new_hash.as_str());
        entry.insert("uid", planned.uid);
        entry.insert("gid", planned.gid);
        entry.insert("mode", planned.mode);
        entry.insert("inputSource", e.source.as_str());
        entry.insert("bootId", self.ops.boot_id());
        self.journal.mounts.push(entry);
        let index = self.journal.mounts.len() - 1;

        self.intent("copy", Some(index))?;
        self.ops.copy_source(&self.entry(index)?)?;
        let backing = self.ops.inspect(&copy)?;
        validate_file(&backing, &copy, Some(&e.new_hash))?;
        self.set_field(index, "sourceDev", backing.dev);
        self.set_field(index, "sourceIno", backing.ino);
        ensure!(
            mounts_at(&self.ops.mounts()?, &e.target).is_empty(),
            "Target became mounted"
        );
        validate_file(
            &self.ops.inspect(&e.target)?,
            &e.target,
            Some(&e.original_hash),
        )?;

        self.intent("bind", Some(index))?;
        self.ops.bind(&self.entry(index)?)?;
        ensure!(
            self.is_owned(&self.entry(index)?)?,
            "Bind ownership readback failed"
        );
        let mount_id = mounts_at(&self.ops.mounts()?, &e.target)
            .first()
            .map(|m| m.id)
            .ok_or_else(|| Error::Policy("Bind ownership readback failed".into()))?;
        self.set_field(index, "mountId", mount_id);

        self.intent("readonly", Some(index))?;
        self.ops.readonly(&self.entry(index)?)?;
        let readonly = mounts_at(&self.ops.mounts()?, &e.target)
            .first()
            .is_some_and(|m| m.is_readonly());
        ensure!(
            self.is_owned(&self.entry(index)?)? && readonly,
            "Read-only bind readback failed"
        );
        self.set_field(index, "state", "mounted");
        self.save()
    }

    /// Removes owned overrides and restores stock Home. Independent of the
    /// manifest; the persistent disable marker is written even when the journal
    /// cannot be parsed.
    pub fn rollback(&mut self, background: bool) -> Result<Outcome> {
        if let Err(error) = self.load() {
            self.ops.disable()?;
            return Err(error);
        }
        self.disable()?;
        let previous = self.foreground_or_unknown();
        let result = self.rollback_body(previous.as_deref(), background);
        self.restore_preload()?;
        result.map(|()| Outcome::RolledBack)
    }

    fn rollback_body(&mut self, previous: Option<&str>, background: bool) -> Result<()> {
        self.begin_preload()?;
        self.remove_owned()?;
        self.assert_no_overrides()?;
        self.restart(previous, background)
    }
}
