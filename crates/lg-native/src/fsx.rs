//! Trusted filesystem primitives.
//!
//! Every trusted read follows the same pattern: validate each ancestor
//! directory with `lstat`, validate the file with `lstat`, open it with
//! `O_NOFOLLOW`, validate the *opened* inode again with `fstat` and require the
//! same device/inode pair. The decision is therefore made about the bytes that
//! are actually read, not about a path that could be swapped in between.
//!
//! Metadata checks are pure functions over [`Meta`], so they are unit-tested
//! with synthetic values on any host.

/// File-type mask and types (POSIX `S_IFMT` family).
pub const S_IFMT: u32 = 0o170_000;
/// Directory type bits.
pub const S_IFDIR: u32 = 0o040_000;
/// Regular-file type bits.
pub const S_IFREG: u32 = 0o100_000;
/// Symbolic-link type bits.
pub const S_IFLNK: u32 = 0o120_000;

/// The subset of `struct stat` the safety rules need.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// Full `st_mode` (type and permission bits).
    pub mode: u32,
    /// Owner user id.
    pub uid: u32,
    /// Owner group id.
    pub gid: u32,
    /// Hard-link count.
    pub nlink: u64,
    /// Size in bytes.
    pub size: u64,
    /// Device id.
    pub dev: u64,
    /// Inode number.
    pub ino: u64,
    /// Modification time in nanoseconds since the epoch.
    pub mtime_ns: i128,
}

impl Meta {
    /// True for directories.
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }

    /// True for regular files.
    pub fn is_regular(&self) -> bool {
        self.mode & S_IFMT == S_IFREG
    }

    /// True for symbolic links (only observable through `lstat`).
    pub fn is_symlink(&self) -> bool {
        self.mode & S_IFMT == S_IFLNK
    }

    /// Permission bits including set-id and sticky bits (`S_IMODE`).
    pub fn permissions(&self) -> u32 {
        self.mode & 0o7777
    }

    /// The `(dev, ino)` identity of the inode.
    pub fn identity(&self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

/// True for a directory that only root can modify: root-owned and not
/// group/other writable.
pub fn is_safe_parent(meta: &Meta) -> bool {
    meta.is_dir() && meta.uid == 0 && meta.mode & 0o022 == 0
}

/// Rules for a trusted regular file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRule {
    /// Permission bits that must all be clear.
    pub forbidden_mode: u32,
    /// Require exactly one hard link.
    pub single_link: bool,
    /// Largest accepted size in bytes.
    pub max_size: u64,
    /// Reject empty files.
    pub nonempty: bool,
}

impl FileRule {
    /// True when `meta` describes an acceptable file.
    pub fn accepts(&self, meta: &Meta) -> bool {
        meta.is_regular()
            && meta.uid == 0
            && meta.mode & self.forbidden_mode == 0
            && (!self.single_link || meta.nlink == 1)
            && meta.size <= self.max_size
            && (!self.nonempty || meta.size > 0)
    }
}

/// One mebibyte: the size cap for every state file and service response.
pub const MIB: u64 = 1024 * 1024;

#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::error::{Error, Result};
    use crate::paths;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    /// Converts standard metadata.
    pub fn meta_of(metadata: &std::fs::Metadata) -> Meta {
        Meta {
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            nlink: metadata.nlink(),
            size: metadata.size(),
            dev: metadata.dev(),
            ino: metadata.ino(),
            mtime_ns: i128::from(metadata.mtime()) * 1_000_000_000
                + i128::from(metadata.mtime_nsec()),
        }
    }

    /// `lstat` (never follows a final symlink).
    pub fn lstat(path: &str) -> Result<Meta> {
        std::fs::symlink_metadata(path)
            .map(|m| meta_of(&m))
            .map_err(|e| Error::io(format!("lstat {path}"), e))
    }

    /// `fstat` of an open file.
    pub fn fstat(file: &File, path: &str) -> Result<Meta> {
        file.metadata()
            .map(|m| meta_of(&m))
            .map_err(|e| Error::io(format!("fstat {path}"), e))
    }

    /// True when anything (even a dangling symlink) exists at `path`.
    pub fn lexists(path: &str) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }

    /// Calls `check` with the `lstat` of every ancestor of `path`, nearest
    /// first, up to and including `/`.
    pub fn for_each_parent(
        path: &str,
        mut check: impl FnMut(&str, &Meta) -> Result<()>,
    ) -> Result<()> {
        let mut parent = paths::parent(path);
        loop {
            let meta = lstat(parent)?;
            check(parent, &meta)?;
            if parent == "/" {
                return Ok(());
            }
            parent = paths::parent(parent);
        }
    }

    /// Opens an existing file read-only without following a final symlink.
    pub fn open_nofollow(path: &str, nonblocking: bool) -> Result<File> {
        let mut flags = libc::O_NOFOLLOW;
        if nonblocking {
            flags |= libc::O_NONBLOCK;
        }
        OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(path)
            .map_err(|e| Error::io(format!("open {path}"), e))
    }

    /// Creates a new file exclusively (`O_CREAT|O_EXCL|O_NOFOLLOW`).
    pub fn create_exclusive(path: &str, mode: u32) -> Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| Error::io(format!("create {path}"), e))
    }

    /// Reads at most `limit + 1` bytes so callers can detect oversize input
    /// without ever buffering an unbounded file.
    pub fn read_capped(file: &mut File, limit: u64, path: &str) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut data)
            .map_err(|e| Error::io(format!("read {path}"), e))?;
        Ok(data)
    }

    /// Writes all bytes and flushes them to stable storage.
    pub fn write_synced(file: &mut File, data: &[u8], path: &str) -> Result<()> {
        file.write_all(data)
            .map_err(|e| Error::io(format!("write {path}"), e))?;
        file.sync_all()
            .map_err(|e| Error::io(format!("fsync {path}"), e))
    }

    /// `fsync` a directory so a rename or creation inside it is durable.
    pub fn fsync_dir(path: &str) -> Result<()> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| Error::io(format!("fsync directory {path}"), e))
    }

    /// Atomically replaces `to` with `from` (same directory).
    pub fn rename(from: &str, to: &str) -> Result<()> {
        std::fs::rename(from, to).map_err(|e| Error::io(format!("rename {from} -> {to}"), e))
    }

    /// Removes a file (not a directory).
    pub fn unlink(path: &str) -> Result<()> {
        std::fs::remove_file(path).map_err(|e| Error::io(format!("unlink {path}"), e))
    }

    /// Resolves symlinks and `..`; `None` when the path cannot be resolved.
    pub fn realpath(path: &str) -> Option<String> {
        std::fs::canonicalize(path)
            .ok()
            .and_then(|p| p.into_os_string().into_string().ok())
    }

    /// `count` random bytes from the kernel, hex encoded.
    pub fn random_hex(count: usize) -> Result<String> {
        let mut bytes = vec![0u8; count];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|e| Error::io("read /dev/urandom", e))?;
        Ok(crate::sha::to_hex(&bytes))
    }

    /// Effective user id.
    #[allow(unsafe_code)]
    pub fn euid() -> u32 {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    /// Opens `path` with `O_NOFOLLOW`, then checks that the opened inode is the
    /// one `before` described and that it still satisfies `accepts`.
    pub fn open_verified(
        path: &str,
        before: &Meta,
        nonblocking: bool,
        accepts: impl Fn(&Meta) -> bool,
        unsafe_message: &str,
        changed_message: &str,
    ) -> Result<(File, Meta)> {
        let file = open_nofollow(path, nonblocking)?;
        let opened = fstat(&file, path)?;
        if !accepts(&opened) {
            return Err(Error::Policy(unsafe_message.to_owned()));
        }
        if opened.identity() != before.identity() {
            return Err(Error::Policy(changed_message.to_owned()));
        }
        Ok((file, opened))
    }
}

/// Fallbacks so the pure parts of the crate build on non-Unix hosts.
#[cfg(not(unix))]
pub fn euid() -> u32 {
    u32::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regular(mode: u32) -> Meta {
        Meta {
            mode: S_IFREG | mode,
            uid: 0,
            nlink: 1,
            size: 10,
            ..Meta::default()
        }
    }

    #[test]
    fn parent_rule_requires_root_owned_unwritable_directory() {
        assert!(is_safe_parent(&Meta {
            mode: S_IFDIR | 0o755,
            ..Meta::default()
        }));
        assert!(is_safe_parent(&Meta {
            mode: S_IFDIR | 0o700,
            ..Meta::default()
        }));
        for bad in [
            Meta {
                mode: S_IFDIR | 0o777,
                ..Meta::default()
            },
            Meta {
                mode: S_IFDIR | 0o775,
                ..Meta::default()
            },
            Meta {
                mode: S_IFLNK | 0o700,
                ..Meta::default()
            },
            Meta {
                mode: S_IFDIR | 0o700,
                uid: 1000,
                ..Meta::default()
            },
        ] {
            assert!(!is_safe_parent(&bad), "{bad:?}");
        }
    }

    #[test]
    fn file_rule_checks_type_owner_mode_links_and_size() {
        let rule = FileRule {
            forbidden_mode: 0o022,
            single_link: true,
            max_size: MIB,
            nonempty: false,
        };
        assert!(rule.accepts(&regular(0o600)));
        assert!(rule.accepts(&regular(0o755)));
        for bad in [
            Meta {
                uid: 1000,
                ..regular(0o600)
            },
            Meta {
                nlink: 2,
                ..regular(0o600)
            },
            Meta {
                mode: S_IFLNK | 0o600,
                ..regular(0)
            },
            Meta {
                mode: S_IFDIR | 0o700,
                ..regular(0)
            },
            regular(0o622),
            Meta {
                size: MIB + 1,
                ..regular(0o600)
            },
        ] {
            assert!(!rule.accepts(&bad), "{bad:?}");
        }
        let private = FileRule {
            forbidden_mode: 0o077,
            single_link: false,
            max_size: MIB,
            nonempty: true,
        };
        assert!(private.accepts(&Meta {
            nlink: 3,
            ..regular(0o600)
        }));
        assert!(!private.accepts(&regular(0o644)));
        assert!(!private.accepts(&Meta {
            size: 0,
            ..regular(0o600)
        }));
    }
}
