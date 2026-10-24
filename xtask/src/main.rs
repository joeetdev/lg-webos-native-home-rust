//! `cargo xtask test`: every offline suite, in order, stopping at the first
//! failure. No SSH, no LS2 calls, no device deployment.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// One suite: a name, a working directory (relative to the repository root)
/// and the command to run there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Suite {
    name: &'static str,
    dir: &'static str,
    program: String,
    args: Vec<String>,
}

fn suite(name: &'static str, dir: &'static str, program: &str, args: &[&str]) -> Suite {
    Suite {
        name,
        dir,
        program: program.to_owned(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    }
}

fn plan() -> Vec<Suite> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    vec![
        suite(
            "rust-format",
            ".",
            &cargo,
            &["fmt", "--all", "--", "--check"],
        ),
        suite(
            "rust-lint",
            ".",
            &cargo,
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--quiet",
                "--",
                "-D",
                "warnings",
            ],
        ),
        suite(
            "rust-tests",
            ".",
            &cargo,
            &["test", "--workspace", "--quiet"],
        ),
        suite(
            "gate-javascript",
            "gate",
            "node",
            &[
                "--test",
                "--test-reporter=dot",
                "tests-gate-proof.cjs",
                "tests-gate-surface.cjs",
                "tests-gate-loader.cjs",
            ],
        ),
        suite(
            "remote-javascript",
            "remote",
            "node",
            &[
                "--test",
                "--test-reporter=dot",
                "tests-remote-core.cjs",
                "tests-remote-controller.cjs",
                "tests-remote-loader.cjs",
            ],
        ),
        suite("boot-hook-syntax", "runtime", "sh", &["-n", "55-lg-native"]),
        suite(
            "publication-check",
            ".",
            &cargo,
            &[
                "run",
                "--quiet",
                "-p",
                "lg-native-host",
                "--",
                "security-check",
            ],
        ),
    ]
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Runs the plan, returning the first non-zero exit code (or 0).
fn run_all(plan: &[Suite], root: &Path, mut run: impl FnMut(&Suite, &Path) -> i32) -> i32 {
    for suite in plan {
        println!("\n== {} ==", suite.name);
        let dir = root.join(suite.dir);
        if !dir.is_dir() {
            println!("Required test suite is missing: {}", suite.name);
            return 2;
        }
        let code = run(suite, &dir);
        if code != 0 {
            println!("Suite failed: {} (exit {code})", suite.name);
            return code;
        }
    }
    0
}

fn execute(suite: &Suite, dir: &Path) -> i32 {
    match Command::new(&suite.program)
        .args(&suite.args)
        .current_dir(dir)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            println!("Cannot run {}: {error}", suite.program);
            127
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args != ["test"] {
        eprintln!("usage: cargo xtask test");
        return ExitCode::from(2);
    }
    let code = run_all(&plan(), &root(), execute);
    if code == 0 {
        println!("\nAll offline suites passed (local run; this is not a CI result).");
    }
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_contains_every_suite_and_only_offline_programs() {
        let plan = plan();
        assert_eq!(plan.len(), 7);
        let dirs: Vec<&str> = plan.iter().map(|s| s.dir).collect();
        for dir in [".", "gate", "remote", "runtime"] {
            assert!(dirs.contains(&dir), "{dir}");
        }
        for suite in &plan {
            for word in std::iter::once(&suite.program).chain(&suite.args) {
                assert!(
                    !word.contains("ssh") && !word.contains("luna-send"),
                    "{word}"
                );
            }
        }
        assert!(plan.iter().all(|s| root().join(s.dir).is_dir()));
    }

    #[test]
    fn stops_on_the_first_nonzero_result() {
        let mut calls = 0;
        assert_eq!(
            run_all(&plan(), &root(), |_, _| {
                calls += 1;
                7
            }),
            7
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_missing_suite_directory_fails() {
        let plan = vec![suite("ghost", "does-not-exist", "true", &[])];
        assert_eq!(run_all(&plan, &root(), |_, _| 0), 2);
    }
}
