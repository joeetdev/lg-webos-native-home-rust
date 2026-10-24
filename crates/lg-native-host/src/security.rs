//! Publication hygiene check over every Git-visible file. Matched content is
//! never printed — only the file name and the kind of finding.

use regex::Regex;
use std::path::Path;
use std::process::Command;

/// Content patterns that must never be published.
pub fn patterns() -> Vec<(&'static str, Regex)> {
    let table: [(&str, &str); 7] = [
        (
            "private-key",
            r"-----BEGIN (?:OPENSSH|RSA|EC|DSA|PGP|ENCRYPTED)? ?PRIVATE KEY",
        ),
        ("github-token", r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"),
        ("github-pat", r"github_pat_[A-Za-z0-9_]{20,}"),
        (
            "private-network",
            r"\b(?:192\.168\.\d{1,3}\.\d{1,3}|10\.\d{1,3}\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b",
        ),
        (
            "personal-path",
            r"[A-Za-z]:[\\/]+Users[\\/]+|/Users[/]|/home/[^/\s]+/\.ssh",
        ),
        ("mac-address", r"\b(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b"),
        (
            "proprietary-source",
            concat!("Confidential computer ", "software"),
        ),
    ];
    table
        .iter()
        .filter_map(|(name, pattern)| Regex::new(pattern).ok().map(|regex| (*name, regex)))
        .collect()
}

/// Kinds of sensitive content found in `text`.
pub fn inspect_content(text: &str, patterns: &[(&'static str, Regex)]) -> Vec<&'static str> {
    patterns
        .iter()
        .filter(|(_, regex)| regex.is_match(text))
        .map(|(name, _)| *name)
        .collect()
}

/// Kinds of sensitive file names.
pub fn inspect_name(name: &str) -> Vec<&'static str> {
    let path = Path::new(name);
    let file = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or_default();
    let in_ssh = name.split('/').any(|part| part == ".ssh");
    if [".env", "id_ed25519", "id_rsa", "known_hosts"].contains(&file) || in_ssh {
        return vec!["credential-file"];
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if extension.is_some_and(|e| {
        ["key", "pem", "zip", "tar", "gz", "ipk", "so", "pyc"].contains(&e.as_str())
    }) {
        return vec!["private-or-firmware-artifact"];
    }
    Vec::new()
}

/// Scans the repository at `root`; returns findings as `(file, issue)`.
pub fn scan(root: &Path) -> std::io::Result<(usize, Vec<(String, &'static str)>)> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("git ls-files failed"));
    }
    let mut names: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|n| !n.is_empty())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .collect();
    names.sort();
    names.dedup();
    let patterns = patterns();
    let mut findings = Vec::new();
    for name in &names {
        findings.extend(
            inspect_name(name)
                .into_iter()
                .map(|issue| (name.clone(), issue)),
        );
        let path = root.join(name);
        let metadata = std::fs::symlink_metadata(&path);
        if metadata.as_ref().is_ok_and(|m| m.file_type().is_symlink()) {
            findings.push((name.clone(), "symlink"));
            continue;
        }
        if !metadata.as_ref().is_ok_and(|m| m.is_file()) {
            findings.push((name.clone(), "missing-file"));
            continue;
        }
        if name.starts_with("docs/images/") && name.to_ascii_lowercase().ends_with(".png") {
            continue;
        }
        match std::fs::read(&path).map(String::from_utf8) {
            Ok(Ok(text)) => findings.extend(
                inspect_content(&text, &patterns)
                    .into_iter()
                    .map(|issue| (name.clone(), issue)),
            ),
            _ => findings.push((name.clone(), "unexpected-binary")),
        }
    }
    Ok((names.len(), findings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> bool {
        !inspect_content(text, &patterns()).is_empty()
    }

    #[test]
    fn real_credential_markers_are_rejected() {
        assert!(found(&format!("-----BEGIN {}", "OPENSSH PRIVATE KEY-----")));
        assert!(found(&format!("gh{}_{}", "p", "A".repeat(36))));
        assert!(found(&format!("github_{}_{}", "pat", "A".repeat(30))));
    }

    #[test]
    fn private_networks_and_profile_paths_are_rejected() {
        assert!(found(&format!("{}.{}", "192", "168.8.99")));
        assert!(found(&format!("{}{}", "10.", "1.2.3")));
        assert!(found(&format!("C:/Us{}", "ers/example/private")));
        assert!(found(&format!("/Us{}", "ers/example")));
        assert!(found(&format!("{}:{}", "aa:bb:cc", "dd:ee:ff")));
        assert!(!found("http://127.0.0.1:18767/remote"));
        assert!(!found("/var/lib/lg-native/home-final-manifest.json"));
    }

    #[test]
    fn sensitive_and_firmware_artifact_names_are_rejected() {
        for name in [
            ".env",
            "x/id_ed25519",
            "backup.zip",
            "firmware.so",
            "original.ipk",
            "a/.ssh/config",
            "k.PEM",
        ] {
            assert!(!inspect_name(name).is_empty(), "{name}");
        }
        assert!(inspect_name("crates/lg-native/src/remote.rs").is_empty());
        assert!(inspect_name(".env.example").is_empty());
    }

    #[test]
    fn this_checker_does_not_embed_private_samples() {
        assert!(!found(include_str!("security.rs")));
    }

    #[test]
    fn proprietary_source_notice_is_rejected() {
        assert!(found(&format!(
            "Confidential computer {}",
            "software. Valid license from LG required"
        )));
    }
}
