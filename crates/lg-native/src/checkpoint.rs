//! Removal of the derived Home CRIU checkpoints — and nothing else.
//!
//! Only the two Home checkpoint roots are considered; `inputcommon` and the
//! global CRIU state are never touched. Every file must be a known checkpoint
//! artefact, otherwise nothing is removed.

use crate::json::{self, Style, Value};

/// The only directories this module may empty and remove.
pub const ROOTS: [&str; 2] = [
    "/tmp/checkpoint/com.webos.app.home",
    "/mnt/lg/cmn_data/checkpoint/com.webos.app.home",
];

const METADATA: [&str; 9] = [
    "stats-restore",
    "stats-dump",
    "img_size",
    "pgid",
    "MD5SUM",
    "starfish-release",
    "flock",
    "fault_pages.dat",
    "invalid_img",
];

/// Files planned for removal in one checkpoint root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// The checkpoint root.
    pub root: String,
    /// File names inside it.
    pub files: Vec<String>,
}

/// True for a known checkpoint artefact name: fixed metadata names or
/// `[A-Za-z0-9_.-]+.img`, optionally followed by `.ori` or `.old`.
pub fn is_checkpoint_file(name: &str) -> bool {
    if METADATA.contains(&name) {
        return true;
    }
    let allowed = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    let stem = [".img", ".img.ori", ".img.old"]
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix));
    allowed && stem.is_some_and(|stem| !stem.is_empty())
}

/// The removal plan as printed by the command line.
pub fn plan_json(plan: &[Group], apply: bool) -> String {
    let groups = plan
        .iter()
        .map(|g| {
            crate::json_object! {
                "root" => g.root.as_str(),
                "files" => Value::Array(g.files.iter().map(Value::from).collect()),
            }
        })
        .collect();
    json::to_string(
        &crate::json_object! {"plan" => Value::Array(groups), "apply" => apply},
        Style::Spaced,
        false,
    )
}

#[cfg(unix)]
pub use unix::{apply, plan, run};

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::ensure;
    use crate::error::{Error, Result};
    use crate::fsx;
    use std::io::Write;

    /// Inspects both roots. Refuses while CRIU is restoring, and refuses any
    /// unexpected directory or file name.
    pub fn plan() -> Result<Vec<Group>> {
        ensure!(
            !fsx::lexists("/tmp/criu_restoring"),
            "Restore active: refuse cache change"
        );
        let mut plan = Vec::new();
        for root in ROOTS {
            if std::fs::metadata(root).is_err() {
                continue;
            }
            let is_plain_dir = std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir())
                && fsx::realpath(root).as_deref() == Some(root);
            ensure!(is_plain_dir, "Unexpected checkpoint root: {root}");
            let mut files = Vec::new();
            for entry in
                std::fs::read_dir(root).map_err(|e| Error::io(format!("list {root}"), e))?
            {
                let entry = entry.map_err(|e| Error::io(format!("list {root}"), e))?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| Error::Policy(format!("Unknown checkpoint file in {root}")))?;
                let path = format!("{root}/{name}");
                ensure!(
                    !std::fs::metadata(&path).is_ok_and(|m| m.is_dir()),
                    "Unexpected directory: {path}"
                );
                ensure!(is_checkpoint_file(&name), "Unknown checkpoint file: {path}");
                files.push(name);
            }
            plan.push(Group {
                root: root.to_owned(),
                files,
            });
        }
        Ok(plan)
    }

    /// Removes exactly the planned files and their roots.
    pub fn apply(plan: &[Group], backup: &str) -> Result<()> {
        ensure!(
            std::fs::metadata(backup).is_ok_and(|m| m.is_file()),
            "Checkpoint backup record missing"
        );
        for group in plan {
            for name in &group.files {
                fsx::unlink(&format!("{}/{name}", group.root))?;
            }
            std::fs::remove_dir(&group.root)
                .map_err(|e| Error::io(format!("rmdir {}", group.root), e))?;
            ensure!(
                std::fs::metadata(&group.root).is_err(),
                "Checkpoint root still present: {}",
                group.root
            );
        }
        Ok(())
    }

    /// `lg-native checkpoint --dry-run|--apply`.
    pub fn run(apply_mode: bool, out: &mut impl Write) -> Result<()> {
        let plan = plan()?;
        let _ = writeln!(out, "{}", plan_json(&plan, apply_mode));
        if apply_mode {
            apply(&plan, crate::home::model::BACKUP_PATH)?;
            let _ = writeln!(
                out,
                "Only Home checkpoint roots removed; inputcommon and global CRIU untouched"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_checkpoint_artefacts_are_accepted() {
        for good in [
            "pages-1.img",
            "core-123.img.ori",
            "x.img.old",
            "a.img.img",
            "MD5SUM",
            "fault_pages.dat",
        ] {
            assert!(is_checkpoint_file(good), "{good}");
        }
        for bad in [
            ".img",
            "a.img.bak",
            "a.img/",
            "../a.img",
            "a b.img",
            "config.json",
            "img",
            "a.imgx",
            "",
        ] {
            assert!(!is_checkpoint_file(bad), "{bad}");
        }
    }

    #[test]
    fn plan_output_matches_the_documented_shape() {
        let plan = vec![Group {
            root: ROOTS[0].into(),
            files: vec!["pgid".into()],
        }];
        assert_eq!(
            plan_json(&plan, false),
            r#"{"plan": [{"root": "/tmp/checkpoint/com.webos.app.home", "files": ["pgid"]}], "apply": false}"#
        );
    }
}
