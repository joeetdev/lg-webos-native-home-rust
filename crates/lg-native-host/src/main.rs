//! `lg-native-host`: offline tools for the authoring host. Never contacts a TV.

mod build_home;
mod pack;
mod security;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: lg-native-host <command>

  build-home --stock-dir DIR --out DIR [--normalize-crlf] [--labels english|original]
      Verify the eight stock hashes, apply home/deltas.json, verify the eight
      pinned output hashes, then write the eight native-* sources and
      home-final-manifest.json into a NEW directory.
      --normalize-crlf  explicit in-memory CRLF -> LF conversion before hashing
      --labels          english (default): Search / Settings / All apps
                        original: the byte-exact, physically verified build
  pack-gate trial|final [--out DIR]     (default DIR: gate/packed-<mode>)
  pack-remote [--out-dir DIR]           (default DIR: remote; writes remote-packed-{trial,final})
  security-check [--root DIR]           (default DIR: .)

Run from the repository root.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    if words.iter().any(|w| *w == "--help" || *w == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match words.split_first() {
        Some((&"build-home", rest)) => build_home_command(rest),
        Some((&"pack-gate", rest)) => pack_gate_command(rest),
        Some((&"pack-remote", rest)) => pack_remote_command(rest),
        Some((&"security-check", rest)) => security_command(rest),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprint!("{USAGE}");
    ExitCode::from(2)
}

/// Parses `--flag value` / `--switch` options.
fn options<'a>(
    words: &[&'a str],
    valued: &[&str],
    switches: &[&str],
) -> Option<Vec<(&'a str, Option<&'a str>)>> {
    let mut parsed = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        if valued.contains(&word) {
            parsed.push((word, Some(*words.get(i + 1)?)));
            i += 2;
        } else if switches.contains(&word) {
            parsed.push((word, None));
            i += 1;
        } else {
            return None;
        }
    }
    Some(parsed)
}

fn value<'a>(parsed: &[(&str, Option<&'a str>)], key: &str) -> Option<&'a str> {
    parsed
        .iter()
        .rev()
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| *v)
}

fn build_home_command(words: &[&str]) -> ExitCode {
    let Some(parsed) = options(
        words,
        &["--stock-dir", "--out", "--labels"],
        &["--normalize-crlf"],
    ) else {
        return usage();
    };
    let (Some(stock), Some(out)) = (value(&parsed, "--stock-dir"), value(&parsed, "--out")) else {
        return usage();
    };
    let labels = match value(&parsed, "--labels").unwrap_or("english") {
        "english" => build_home::Labels::English,
        "original" => build_home::Labels::Original,
        _ => return usage(),
    };
    let normalize = parsed.iter().any(|(k, _)| *k == "--normalize-crlf");
    let result = build_home::production_spec().and_then(|spec| {
        build_home::build(Path::new(stock), Path::new(out), &spec, normalize, labels)
    });
    match result {
        Ok(manifest) => {
            println!(
                "Verified {n} stock hashes and {n} generated hashes; wrote {n} native sources and manifest.",
                n = manifest.len()
            );
            if labels == build_home::Labels::English {
                println!(
                    "Applied English labels (Search, Settings, All apps) after verifying the pinned output; \
                     manifest hashes describe the labelled files."
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

fn pack_gate_command(words: &[&str]) -> ExitCode {
    let Some((mode, rest)) = words.split_first() else {
        return usage();
    };
    let Some(mode) = pack::Mode::parse(mode) else {
        return usage();
    };
    let Some(parsed) = options(rest, &["--out"], &[]) else {
        return usage();
    };
    let out = value(&parsed, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("gate/packed-{}", mode.as_str())));
    match pack::pack_gate(mode, &out) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

fn pack_remote_command(words: &[&str]) -> ExitCode {
    let Some(parsed) = options(words, &["--out-dir"], &[]) else {
        return usage();
    };
    let base = PathBuf::from(value(&parsed, "--out-dir").unwrap_or("remote"));
    for mode in [pack::Mode::Trial, pack::Mode::Final] {
        match pack::pack_remote(mode, &base.join(format!("remote-packed-{}", mode.as_str()))) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("ERROR: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn security_command(words: &[&str]) -> ExitCode {
    let Some(parsed) = options(words, &["--root"], &[]) else {
        return usage();
    };
    let root = PathBuf::from(value(&parsed, "--root").unwrap_or("."));
    match security::scan(&root) {
        Ok((count, findings)) => {
            for (name, issue) in &findings {
                println!("{name}: {issue}");
            }
            println!(
                "Publication check: {count} Git-visible files, {} findings",
                findings.len()
            );
            if findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
