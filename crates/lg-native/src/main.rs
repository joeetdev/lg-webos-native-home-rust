//! `lg-native`: the single static device binary.
//!
//! One short-lived process per command; nothing stays resident except the
//! optional 240-second trial guard. Run `lg-native --help` for the commands.

use std::io::Write;

const USAGE: &str = "\
usage: lg-native <command>

Native Home overrides (replaces nativeapply.py / rollback.py):
  home apply [--boot] [--staging]   apply the eight read-only overrides
      --boot      one attempt per boot, bounded 45 s readiness wait
      --staging   permit 3-8 distinct known QML targets
  home rollback [--background]      release the gate, then restore stock Home
      --background  always relaunch stock Home with preload=full

Presentation gate (replaces native_gate_control.py / gate_install.py):
  gate disable                remove only the exact owned gate filter
  gate recover-if-unready     remove the gate unless all eight overlays are healthy
  gate install <package>      install gate-package-{trial,final}.json with a trial guard
  gate check                  report readiness and the key-filter chain
  gate commit                 retire the trial guard after final verification
  gate guard --spawn          (boot hook) arm the trial guard if a trial is pending
  gate guard --watch <token>  (internal) the detached 240 s trial watcher

Remote controls (replaces remote_controls.py):
  remote install <package>    install the remote key filter
  remote disable              remove only owned remote entries
  remote check                verify integrity and position

Derived Home checkpoints (replaces clear-home-checkpoint.py):
  checkpoint --dry-run | --apply

All commands require root on the TV and must run as /var/lib/lg-native/lg-native.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    std::process::exit(dispatch(&words));
}

#[cfg_attr(not(unix), allow(dead_code))]
fn usage_error() -> i32 {
    let _ = write!(std::io::stderr(), "{USAGE}");
    2
}

fn help() -> i32 {
    let _ = write!(std::io::stdout(), "{USAGE}");
    0
}

#[cfg(unix)]
fn dispatch(words: &[&str]) -> i32 {
    if words.iter().any(|w| *w == "--help" || *w == "-h") {
        return help();
    }
    if words == ["--version"] {
        println!("lg-native {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let mut out = std::io::stdout().lock();
    match words {
        ["home", rest @ ..] => home(rest, &mut out),
        ["gate", rest @ ..] => gate(rest, &mut out),
        ["remote", rest @ ..] => remote(rest, &mut out),
        ["checkpoint", mode] if *mode == "--dry-run" || *mode == "--apply" => {
            let result = lg_native::device::preflight()
                .and_then(|()| lg_native::checkpoint::run(*mode == "--apply", &mut out));
            report_text(result)
        }
        _ => usage_error(),
    }
}

#[cfg(not(unix))]
fn dispatch(words: &[&str]) -> i32 {
    if words.iter().any(|w| *w == "--help" || *w == "-h") {
        return help();
    }
    eprintln!("Requires root on Linux; not a host installer");
    1
}

#[cfg(unix)]
fn report_text(result: lg_native::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}: {error}", error.kind());
            1
        }
    }
}

#[cfg(target_os = "linux")]
fn home(words: &[&str], out: &mut impl Write) -> i32 {
    use lg_native::home::{self, linux::LinuxOps, Action};
    let flags = |allowed: &[&str]| words[1..].iter().all(|w| allowed.contains(w));
    let has = |flag: &str| words[1..].contains(&flag);
    let action = match words.first() {
        Some(&"apply") if flags(&["--boot", "--staging"]) => Action::Apply {
            boot: has("--boot"),
            staging: has("--staging"),
        },
        Some(&"rollback") if flags(&["--background"]) => Action::Rollback {
            background: has("--background"),
        },
        _ => return usage_error(),
    };
    let factory = || {
        lg_native::device::preflight()?;
        LinuxOps::new()
    };
    let mut err = std::io::stderr();
    match action {
        // Releasing the gate is a safety step: like every gate-release path
        // it needs only root, never the installed-binary preflight.
        Action::Rollback { background } => home::rollback_releasing_gate::<LinuxOps, _>(
            || {
                lg_native::gate::require_root()
                    .and_then(|()| lg_native::gate::release_for_rollback())
                    .map(drop)
            },
            background,
            factory,
            out,
            &mut err,
        ),
        apply => home::run::<LinuxOps, _>(apply, factory, out, &mut err),
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn home(_words: &[&str], _out: &mut impl Write) -> i32 {
    eprintln!("Requires root on Linux; not a host installer");
    1
}

#[cfg(unix)]
fn gate(words: &[&str], out: &mut impl Write) -> i32 {
    use lg_native::gate::{self, guard, install, Command};
    // Gate-release paths (disable, recover-if-unready, the trial watcher) are
    // the safety net that keeps Home visible: they need only root. A failed
    // installed-binary preflight makes the readiness proof fail instead, which
    // releases the gate rather than leaving Home hidden.
    let load = || {
        gate::require_root()?;
        gate::load_state()
    };
    let json_error = |out: &mut dyn Write, error: lg_native::Error| {
        let report = lg_native::json_object! {"error" => format!("{}: {error}", error.kind())};
        let _ = writeln!(out, "{}", gate::report(&report));
        1
    };
    let result = match words {
        ["disable"] => {
            return gate::run(
                Command::Disable,
                load,
                &mut gate::luna_call,
                gate::device_ready,
                out,
            )
        }
        ["recover-if-unready"] => {
            return gate::run(
                Command::RecoverIfUnready,
                load,
                &mut gate::luna_call,
                gate::device_ready,
                out,
            )
        }
        ["install", package] => {
            lg_native::device::preflight().and_then(|()| install::install_package(package, out))
        }
        ["check"] => lg_native::device::preflight().and_then(|()| install::check(out)),
        ["commit"] => lg_native::device::preflight().and_then(|()| install::commit(out)),
        ["guard", "--spawn"] => lg_native::device::preflight().and_then(|()| guard::spawn(out)),
        ["guard", "--watch", token] => {
            gate::require_root().and_then(|()| guard::run_watch(token, out))
        }
        _ => return usage_error(),
    };
    match result {
        Ok(()) => 0,
        Err(error) => json_error(out, error),
    }
}

#[cfg(unix)]
fn remote(words: &[&str], out: &mut impl Write) -> i32 {
    use lg_native::remote::{self, Command, LinuxIo};
    let command = match words {
        ["install", package] => Command::Install((*package).to_owned()),
        ["disable"] => Command::Disable,
        ["check"] => Command::Check,
        _ => return usage_error(),
    };
    let io = LinuxIo::new().and_then(|io| lg_native::device::preflight().map(|()| io));
    remote::run(&command, io, out)
}
