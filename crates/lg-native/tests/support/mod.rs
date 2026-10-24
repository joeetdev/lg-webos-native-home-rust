//! Test doubles that model only the Linux/LS2 boundary; the real controller,
//! guards and recovery code run unchanged against them. All data is synthetic.
#![allow(dead_code)]

use lg_native::home::model::{FileInfo, MountInfo, QML};
use lg_native::home::{HomeOps, HomeReader, Runtime};
use lg_native::json::{Map, Value};
use lg_native::{json_object, Error, Result};
use std::cell::RefCell;
use std::collections::BTreeMap;

pub const ROOT: &str = "/var/lib/lg-native";
pub const REL: [&str; 6] = [
    "UserInterfaceLayer/Containers/MainView.qml",
    "UserInterfaceLayer/Containers/Main/MainRibbon.qml",
    "UserInterfaceLayer/Containers/Main/TopShelf.qml",
    "Model/ShelfList.js",
    "Interfaces/ConnectionsInterfaces/ConnectionsInterfaces.qml",
    "Configurations/SystemServiceResponseProcessors.js",
];

pub fn h(c: char) -> String {
    c.to_string().repeat(64)
}

/// `count` synthetic manifest entries for the first known targets.
pub fn items(count: usize) -> Vec<Value> {
    REL.iter()
        .take(count)
        .enumerate()
        .map(|(i, rel)| {
            json_object! {
                "source" => format!("{ROOT}/candidate/file{i}"),
                "target" => format!("{QML}{rel}"),
                "originalHash" => h('a'),
                "newHash" => h('b'),
            }
        })
        .collect()
}

pub fn observed(path: &str, digest: &str, ino: u64) -> FileInfo {
    FileInfo {
        path: path.into(),
        realpath: path.into(),
        regular: true,
        symlink: false,
        uid: 0,
        gid: 0,
        mode: 0o644,
        nlink: 1,
        dev: 2,
        ino,
        digest: digest.into(),
    }
}

pub fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn entry_text(entry: &Map, key: &str) -> String {
    entry
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Every boundary call is recorded as (action, value).
#[derive(Debug)]
pub struct FakeOps {
    pub manifest: Option<Vec<Value>>,
    pub journal: Option<Value>,
    pub saved: Vec<Value>,
    pub files: BTreeMap<String, FileInfo>,
    pub stock: BTreeMap<String, FileInfo>,
    pub live: Vec<MountInfo>,
    pub events: RefCell<Vec<(String, Value)>>,
    pub enabled: bool,
    pub fg: Option<String>,
    pub fg_fails: bool,
    pub fg_after_copy: Option<String>,
    pub preload_fails: bool,
    pub restore_true_fails: bool,
    pub refuse_clear: bool,
    pub disabled: bool,
    pub fail: Option<String>,
    pub fail_journal_action: Option<String>,
    pub boot_id: String,
    // Signal simulation: LS2-like calls check for termination like the real
    // interruptible luna-send calls, and a signal can be raised at an action.
    pub luna_checks_signals: bool,
    pub raise_on: Option<String>,
    pub raise_signal: i32,
    // Runtime (command-flow) state.
    pub claim: bool,
    pub not_ready: bool,
    pub budgets: Vec<f64>,
    pub locks: usize,
    pub unlocks: usize,
}

impl FakeOps {
    pub fn new() -> Self {
        Self::with(true, Some("com.webos.app.home"), None)
    }

    pub fn with(enabled: bool, fg: Option<&str>, fail: Option<&str>) -> Self {
        let manifest = items(6);
        let mut files = BTreeMap::new();
        let mut stock = BTreeMap::new();
        for (i, e) in manifest.iter().enumerate() {
            let source = text(e, "source");
            files.insert(source.clone(), observed(&source, &h('b'), i as u64 + 20));
            let target = text(e, "target");
            stock.insert(target.clone(), observed(&target, &h('a'), i as u64 + 100));
        }
        files.extend(stock.clone());
        FakeOps {
            manifest: Some(manifest),
            journal: None,
            saved: Vec::new(),
            files,
            stock,
            live: Vec::new(),
            events: RefCell::new(Vec::new()),
            enabled,
            fg: fg.map(str::to_owned),
            fg_fails: false,
            fg_after_copy: None,
            preload_fails: false,
            restore_true_fails: false,
            refuse_clear: false,
            disabled: false,
            fail: fail.map(str::to_owned),
            fail_journal_action: None,
            boot_id: "boot-a".into(),
            luna_checks_signals: false,
            raise_on: None,
            raise_signal: 0,
            claim: true,
            not_ready: false,
            budgets: Vec::new(),
            locks: 0,
            unlocks: 0,
        }
    }

    pub fn event(&self, action: &str, value: Value) {
        self.events.borrow_mut().push((action.to_owned(), value));
    }

    /// Raises the configured signal (synchronously handled) at `action`.
    pub fn maybe_raise(&mut self, action: &str) {
        if self.raise_on.as_deref() == Some(action) {
            self.raise_on = None;
            #[cfg(unix)]
            #[allow(unsafe_code)]
            // SAFETY: raise() with a valid signal number; the installed handler
            // only stores an atomic flag.
            unsafe {
                libc::raise(self.raise_signal);
            }
        }
    }

    fn luna(&mut self) -> Result<()> {
        if self.luna_checks_signals {
            lg_native::signals::check()?;
        }
        Ok(())
    }

    fn hit(&mut self, action: &str, value: Value) -> Result<()> {
        self.event(action, value);
        self.maybe_raise(action);
        if self.fail.as_deref() == Some(action) {
            self.fail = None;
            return Err(Error::Data(format!("injected {action}")));
        }
        Ok(())
    }

    pub fn actions(&self) -> Vec<String> {
        self.events
            .borrow()
            .iter()
            .map(|(a, _)| a.clone())
            .collect()
    }

    pub fn values(&self, action: &str) -> Vec<Value> {
        self.events
            .borrow()
            .iter()
            .filter(|(a, _)| a == action)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn clear_events(&self) {
        self.events.borrow_mut().clear();
    }

    /// Bind `entry` like the real kernel would (used to stage legacy mounts).
    pub fn bind_value(&mut self, entry: &Value) {
        let mut map = Map::new();
        for key in ["source", "target"] {
            map.insert(key, entry.get(key).cloned().unwrap_or(Value::Null));
        }
        let _ = self.bind(&map);
    }

    pub fn journal_mounts(&self) -> Vec<Value> {
        self.journal
            .as_ref()
            .and_then(|j| j.get("mounts"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    pub fn journal_preload(&self) -> Value {
        self.journal
            .as_ref()
            .and_then(|j| j.get("preload"))
            .cloned()
            .unwrap_or(Value::Null)
    }
}

impl HomeReader for FakeOps {
    fn boot_id(&self) -> &str {
        &self.boot_id
    }
    fn is_disabled(&self) -> bool {
        self.disabled
    }
    fn load_manifest(&self) -> Result<Value> {
        self.event("load_manifest", Value::Null);
        self.manifest
            .clone()
            .map(Value::Array)
            .ok_or_else(|| Error::Policy("manifest unavailable".into()))
    }
    fn load_journal(&self) -> Result<Option<Value>> {
        Ok(self.journal.clone())
    }
    fn mounts(&self) -> Result<Vec<MountInfo>> {
        Ok(self.live.clone())
    }
    fn inspect(&self, path: &str) -> Result<FileInfo> {
        self.event("inspect", Value::from(path));
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| Error::io(format!("lstat {path}"), std::io::ErrorKind::NotFound.into()))
    }
}

impl HomeOps for FakeOps {
    fn save_journal(&mut self, journal: &Value) -> Result<()> {
        let action = journal.get("action").cloned().unwrap_or(Value::Null);
        if self.fail_journal_action.is_some()
            && action.as_str() == self.fail_journal_action.as_deref()
        {
            self.fail_journal_action = None;
            return Err(Error::io(
                "write journal",
                std::io::Error::other("disk failure before bind"),
            ));
        }
        self.journal = Some(journal.clone());
        self.saved.push(journal.clone());
        if let Some(name) = action.as_str() {
            self.maybe_raise(&format!("journal:{name}"));
        }
        self.event("journal", action);
        Ok(())
    }
    fn disable(&mut self) -> Result<()> {
        self.hit("disable", Value::Null)?;
        self.disabled = true;
        Ok(())
    }
    fn foreground(&mut self) -> Result<Option<String>> {
        self.luna()?;
        if self.fg_fails {
            return Err(Error::Data("foreground API unavailable".into()));
        }
        Ok(self.fg.clone())
    }
    fn preload(&mut self) -> Result<bool> {
        self.luna()?;
        if self.preload_fails {
            return Err(Error::Data("policy unavailable".into()));
        }
        Ok(self.enabled)
    }
    fn set_preload(&mut self, enabled: bool) -> Result<()> {
        self.luna()?;
        if enabled && self.restore_true_fails {
            return Err(Error::Data("restore API unavailable".into()));
        }
        self.hit("preload", Value::from(enabled))?;
        self.enabled = enabled;
        Ok(())
    }
    fn copy_source(&mut self, entry: &Map) -> Result<()> {
        let source = entry_text(entry, "source");
        self.hit("copy", Value::from(source.as_str()))?;
        let ino = 300 + self.files.len() as u64;
        let mut info = observed(&source, &entry_text(entry, "newHash"), ino);
        let number = |key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(u64::MAX) as u32;
        info.uid = number("uid");
        info.gid = number("gid");
        info.mode = number("mode");
        self.files.insert(source, info);
        if let Some(app) = self.fg_after_copy.clone() {
            self.fg = Some(app);
        }
        Ok(())
    }
    fn bind(&mut self, entry: &Map) -> Result<()> {
        let target = entry_text(entry, "target");
        self.hit("bind", Value::from(target.as_str()))?;
        let mut info = self
            .files
            .get(&entry_text(entry, "source"))
            .cloned()
            .ok_or_else(|| Error::Policy("no source".into()))?;
        info.path = target.clone();
        info.realpath = target.clone();
        self.files.insert(target.clone(), info);
        let id = 500 + self.live.len() as u64;
        self.live.push(MountInfo {
            id,
            target,
            options: vec!["rw".into()],
        });
        Ok(())
    }
    fn readonly(&mut self, entry: &Map) -> Result<()> {
        let target = entry_text(entry, "target");
        self.hit("readonly", Value::from(target.as_str()))?;
        if let Some(mount) = self.live.iter_mut().find(|m| m.target == target) {
            mount.options = vec!["ro".into()];
        }
        Ok(())
    }
    fn unmount(&mut self, entry: &Map) -> Result<()> {
        let target = entry_text(entry, "target");
        self.hit("unmount", Value::from(target.as_str()))?;
        self.live.retain(|m| m.target != target);
        if let Some(stock) = self.stock.get(&target).cloned() {
            self.files.insert(target, stock);
        }
        Ok(())
    }
    fn clear_checkpoint(&mut self) -> Result<()> {
        if self.refuse_clear {
            return Err(Error::Data("CRIU restoration active".into()));
        }
        self.hit("clear", Value::Null)
    }
    fn kill_home(&mut self) -> Result<()> {
        self.hit("kill", Value::Null)
    }
    fn launch(&mut self, payload: &Value) -> Result<()> {
        self.luna()?;
        self.hit("launch", payload.clone())
    }
}

/// Fake config service with real chain state and exact payload capture.
#[derive(Debug, Default)]
pub struct ConfigService {
    pub filters: Vec<Value>,
    pub calls: Vec<(String, Value)>,
    pub bad_readback: bool,
    pub fail: Option<String>,
}

pub const KEY: &str = "com.webos.surfacemanager.keyFilters";

impl ConfigService {
    pub fn new(filters: Vec<Value>) -> Self {
        ConfigService {
            filters,
            ..Default::default()
        }
    }

    pub fn call(&mut self, method: &str, payload: &Value) -> Result<Value> {
        self.calls.push((method.to_owned(), payload.clone()));
        if self.fail.as_deref() == Some(method) {
            return Ok(json_object! {"returnValue" => false, "errorText" => "injected LS2 error"});
        }
        if method == "com.webos.service.config/getConfigs" {
            assert_eq!(
                *payload,
                json_object! {"configNames" => Value::Array(vec![Value::from(KEY)])}
            );
            return Ok(json_object! {
                "returnValue" => true,
                "configs" => json_object! {KEY => Value::Array(self.filters.clone())},
            });
        }
        assert_eq!(method, "com.webos.service.config/setConfigs");
        let configs = payload
            .get("configs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        assert!(payload
            .as_object()
            .is_some_and(|p| p.has_exact_keys(&["configs"])));
        assert!(configs.has_exact_keys(&[KEY]));
        if !self.bad_readback {
            self.filters = configs
                .get(KEY)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
        }
        Ok(json_object! {"returnValue" => true})
    }

    pub fn method_names(&self) -> Vec<String> {
        self.calls
            .iter()
            .map(|(m, _)| m.rsplit('/').next().unwrap_or_default().to_owned())
            .collect()
    }
}

impl Runtime for FakeOps {
    fn lock(&mut self) -> Result<()> {
        self.locks += 1;
        Ok(())
    }
    fn unlock(&mut self) {
        self.unlocks += 1;
    }
    fn claim_boot(&mut self) -> Result<bool> {
        Ok(self.claim)
    }
    fn wait_ready(&mut self, budget: f64) -> Result<()> {
        self.budgets.push(budget);
        if self.not_ready {
            return Err(Error::Timeout("busy".into()));
        }
        Ok(())
    }
    fn verify_checkpoint_backup(&mut self) -> Result<()> {
        Ok(())
    }
}

impl lg_native::gate::ConfigCall for ConfigService {
    fn call(&mut self, method: &str, payload: &Value) -> Result<Value> {
        ConfigService::call(self, method, payload)
    }
}
