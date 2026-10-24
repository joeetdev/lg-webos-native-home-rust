//! Pure (I/O-free) path rules shared by every subsystem.

/// Private state directory. Fixed: there is deliberately no configurable root.
pub const ROOT: &str = "/var/lib/lg-native";

/// The single installed device binary every hook and helper executes.
pub const BINARY: &str = "/var/lib/lg-native/lg-native";

/// `PATH` given to child processes (never the caller's `PATH`, never `.`).
pub const SAFE_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// True for a lexically canonical absolute POSIX path: no empty, `.` or `..`
/// components and no trailing slash (Python `normpath(p) == p`, excluding the
/// special `//` prefix, which is always refused).
pub fn is_canonical_absolute(path: &str) -> bool {
    path == "/"
        || (path.starts_with('/')
            && !path.ends_with('/')
            && path[1..]
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."))
}

/// True when every character is printable ASCII without space or backslash.
pub fn is_plain_ascii(path: &str) -> bool {
    path.bytes().all(|b| (33..127).contains(&b) && b != b'\\')
}

/// Canonical, absolute and plain ASCII: the rule for every trusted file path.
pub fn is_trusted_path(path: &str) -> bool {
    is_canonical_absolute(path) && is_plain_ascii(path)
}

/// The parent directory (`posixpath.dirname` for canonical absolute paths).
pub fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(index) => &path[..index],
    }
}

/// The final component.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Joins the private root with a file name.
pub fn in_root(name: &str) -> String {
    format!("{ROOT}/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_paths() {
        for good in ["/", "/var", "/var/lib/lg-native/a.json", "/a/b-c_d.e"] {
            assert!(is_canonical_absolute(good), "{good}");
        }
        for bad in [
            "",
            "relative",
            "/var/",
            "//var",
            "/var//lib",
            "/var/./lib",
            "/var/../lib",
            "/..",
            "/.",
            "/var/lib/..",
        ] {
            assert!(!is_canonical_absolute(bad), "{bad}");
        }
        assert!(!is_trusted_path("/var/lib/a b"));
        assert!(!is_trusted_path("/var/lib/a\\b"));
        assert!(!is_trusted_path("/var/lib/\u{e9}"));
        assert!(!is_trusted_path("/var/lib/a\n"));
    }

    #[test]
    fn parents_and_names() {
        assert_eq!(parent("/var/lib/x"), "/var/lib");
        assert_eq!(parent("/var"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(file_name("/var/lib/x.json"), "x.json");
        assert_eq!(in_root("home.lock"), "/var/lib/lg-native/home.lock");
    }
}
