//! Preconditions shared by every device command.

use crate::ensure;
use crate::error::{Error, Result};
use crate::fsx;
use crate::paths::BINARY;

/// Every device command runs only as root on Linux, and only as the installed
/// binary: the root-owned, single-link file at the fixed private path whose
/// ancestors only root can modify. Hooks and the trial guard re-execute that
/// exact file, so this is the trust anchor for all code that runs as root.
///
/// Messages are path-free so they are safe to print from any subsystem.
pub fn preflight() -> Result<()> {
    ensure!(
        cfg!(target_os = "linux") && fsx::euid() == 0,
        "Requires root on Linux; not a host installer"
    );
    let exe = std::env::current_exe().map_err(|e| Error::io("locate executable", e))?;
    ensure!(
        exe.to_str() == Some(BINARY),
        "Requires the installed binary in the private root"
    );
    fsx::for_each_parent(BINARY, |_, meta| {
        ensure!(
            fsx::is_safe_parent(meta),
            "Untrusted installed binary location"
        );
        Ok(())
    })?;
    let meta = fsx::lstat(BINARY)?;
    ensure!(
        meta.is_regular() && meta.uid == 0 && meta.mode & 0o022 == 0 && meta.nlink == 1,
        "Untrusted installed binary"
    );
    Ok(())
}
