//! Trusted reads against REAL files: symlinks, hard links, modes, ownership,
//! duplicate keys and oversize input. Root-only (ownership must be uid 0) and
//! skipped unless every ancestor of the scratch directory is root-owned and
//! not group/other writable — exactly the condition the code requires.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lg_native::fsx;
use lg_native::gate::{load_state_at, trusted_bytes, STATE_NAME};
use lg_native::home::linux::inspect_file;
use lg_native::remote::{Io, LinuxIo};
use lg_native::{json_object, sha, Error};
use std::os::unix::fs::{symlink, PermissionsExt};

fn scratch(name: &str) -> Option<String> {
    if fsx::euid() != 0 {
        eprintln!("skipped: trusted-file tests need root ownership semantics");
        return None;
    }
    let base = format!(
        "{}/trusted-{name}-{}",
        env!("CARGO_TARGET_TMPDIR"),
        std::process::id()
    );
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).ok()?;
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).ok()?;
    let safe = fsx::for_each_parent(&format!("{base}/x"), |_, meta| {
        if fsx::is_safe_parent(meta) {
            Ok(())
        } else {
            Err(Error::Policy("unsafe".into()))
        }
    });
    if safe.is_err() {
        eprintln!("skipped: scratch ancestors are not root-owned and unwritable by others");
        return None;
    }
    Some(base)
}

fn write(path: &str, data: &[u8], mode: u32) {
    std::fs::write(path, data).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn state_json() -> String {
    let file = format!(
        "/var/lib/lg-native/native-home-gate-loader-{}.js",
        "a".repeat(64)
    );
    lg_native::json::compact(&json_object! {
        "version" => 1u64, "owner" => "lg-native-home-gate-v1",
        "entry" => json_object! {"file" => file, "handler" => "handleNativeHomeGate"},
    })
}

#[test]
fn gate_state_reads_validate_parent_file_and_opened_inode() {
    let Some(root) = scratch("gate") else { return };
    let state = format!("{root}/{STATE_NAME}");
    assert!(
        load_state_at(&format!("{root}/absent")).unwrap().is_none(),
        "absent root is no state"
    );
    assert!(
        load_state_at(&root).unwrap().is_none(),
        "absent state file is no state"
    );
    write(&state, state_json().as_bytes(), 0o600);
    assert!(load_state_at(&root).unwrap().is_some());

    write(&state, state_json().as_bytes(), 0o622);
    assert!(load_state_at(&root).is_err(), "group/other writable");
    write(&state, state_json().as_bytes(), 0o600);

    let link = format!("{root}/hard-link");
    std::fs::hard_link(&state, &link).unwrap();
    assert!(load_state_at(&root).is_err(), "second hard link");
    std::fs::remove_file(&link).unwrap();

    let dup = r#"{"owner":"foreign","owner":"lg-native-home-gate-v1"}"#;
    write(&state, dup.as_bytes(), 0o600);
    assert!(load_state_at(&root).is_err(), "duplicate key");
    write(&state, &vec![b' '; 1024 * 1024 + 1], 0o600);
    assert!(load_state_at(&root).is_err(), "oversized");

    std::fs::remove_file(&state).unwrap();
    let real = format!("{root}/real.json");
    write(&real, state_json().as_bytes(), 0o600);
    symlink(&real, &state).unwrap();
    assert!(
        load_state_at(&root).is_err(),
        "symlinked state is never followed"
    );
    assert!(trusted_bytes(&state, true).is_err());

    let open_root = format!("{root}/open");
    std::fs::create_dir(&open_root).unwrap();
    std::fs::set_permissions(&open_root, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        load_state_at(&open_root).is_err(),
        "world-writable root is not absence"
    );
    let dangling = format!("{root}/dangling");
    symlink(format!("{root}/nowhere"), &dangling).unwrap();
    assert!(
        load_state_at(&dangling).is_err(),
        "a dangling symlink is NOT an absent root"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn inspection_hashes_regular_files_and_refuses_symlinks() {
    let Some(root) = scratch("inspect") else {
        return;
    };
    let file = format!("{root}/payload.qml");
    write(&file, b"import QtQuick 2.4\n", 0o644);
    let info = inspect_file(&file).unwrap();
    assert_eq!(info.digest, sha::hex(b"import QtQuick 2.4\n"));
    assert_eq!(
        (info.realpath.as_str(), info.mode, info.nlink, info.uid),
        (file.as_str(), 0o644, 1, 0)
    );
    let link = format!("{root}/link.qml");
    symlink(&file, &link).unwrap();
    let error = inspect_file(&link).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Not a regular nonsymlink file"),
        "{error}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn remote_private_reads_require_private_nonempty_files() {
    let Some(root) = scratch("remote") else {
        return;
    };
    let mut io = LinuxIo::new().unwrap();
    let file = format!("{root}/package.json");
    write(&file, b"{}", 0o600);
    assert_eq!(io.read_private(&file).unwrap(), b"{}");
    write(&file, b"{}", 0o644);
    assert!(
        matches!(io.read_private(&file), Err(Error::Policy(m)) if m == "Expected private regular file")
    );
    write(&file, b"", 0o600);
    assert!(
        matches!(io.read_private(&file), Err(Error::Policy(m)) if m == "File size outside limit")
    );
    write(&file, br#"{"a":1,"a":2}"#, 0o600);
    assert!(
        matches!(io.read_json(&file, false), Err(Error::Policy(m)) if m == "Duplicate JSON key")
    );
    assert!(io
        .read_json(&format!("{root}/absent.json"), true)
        .unwrap()
        .is_none());
    std::fs::remove_dir_all(&root).unwrap();
}
