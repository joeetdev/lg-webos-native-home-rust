//! The trial guard: an independent, time-bounded watchdog armed before a gate
//! installation. Unless the installation is committed (which removes the token)
//! within 240 seconds, it removes the gate again. It is a separate detached
//! process, so it survives the SSH session or installer that armed it.

use crate::error::{Error, Result};
use crate::json::Value;
use std::time::Duration;

/// Pending-trial token file.
pub const PENDING: &str = "/var/lib/lg-native/native-home-gate.pending";
/// How long an unconfirmed trial may stay installed.
pub const TRIAL_SECONDS: u64 = 240;

/// True for a 32-character lowercase hexadecimal token.
pub fn is_token(text: &str) -> bool {
    crate::sha::is_lower_hex(text, 32)
}

/// Extracts and validates the token from the pending file's JSON.
pub fn parse_token(value: &Value) -> Result<String> {
    let token = value
        .get("token")
        .ok_or_else(|| Error::Policy("Invalid trial token".into()))?;
    token
        .as_str()
        .filter(|t| is_token(t))
        .map(str::to_owned)
        .ok_or_else(|| Error::Policy("Invalid trial token".into()))
}

/// Outcome of a watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    /// The token changed or disappeared (committed or re-armed).
    Cancelled,
    /// The trial expired and the gate was removed.
    Recovered,
}

impl Watch {
    /// Text reported by the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Watch::Cancelled => "cancelled",
            Watch::Recovered => "recovered",
        }
    }
}

/// Polls the token once per second until `timeout`; removes the gate only if
/// the SAME token is still pending at the deadline.
pub fn watch(
    token: &str,
    mut current: impl FnMut() -> Result<Option<String>>,
    disable: impl FnOnce() -> Result<()>,
    mut now: impl FnMut() -> Duration,
    mut sleep: impl FnMut(Duration),
    timeout: Duration,
) -> Result<Watch> {
    let deadline = now() + timeout;
    while now() < deadline {
        if current()?.as_deref() != Some(token) {
            return Ok(Watch::Cancelled);
        }
        sleep(Duration::from_secs(1));
    }
    if current()?.as_deref() != Some(token) {
        return Ok(Watch::Cancelled);
    }
    disable()?;
    Ok(Watch::Recovered)
}

#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::ensure;
    use crate::fsx::{self, FileRule};
    use crate::json;
    use crate::paths::BINARY;
    use std::io::Write;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    const TOKEN_FILE: FileRule = FileRule {
        forbidden_mode: 0o022,
        single_link: true,
        max_size: 4095,
        nonempty: false,
    };

    /// Reads the pending token; `None` when no trial is pending.
    pub fn read_token() -> Result<Option<String>> {
        let before = match fsx::lstat(PENDING) {
            Ok(meta) => meta,
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        ensure!(TOKEN_FILE.accepts(&before), "Untrusted trial token");
        let (mut file, _) = fsx::open_verified(
            PENDING,
            &before,
            true,
            |m| TOKEN_FILE.accepts(m),
            "Untrusted trial token",
            "Untrusted trial token",
        )?;
        let data = fsx::read_capped(&mut file, 4095, PENDING)?;
        let value = json::parse_bytes(&data).map_err(|e| Error::Data(e.to_string()))?;
        parse_token(&value).map(Some)
    }

    /// True while `pid` exists.
    #[allow(unsafe_code)]
    pub fn is_alive(pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks existence and permission.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// Starts `lg-native gate guard --watch <token>` in a new session with
    /// `/dev/null` stdio, detached from the caller.
    #[allow(unsafe_code)]
    pub fn spawn_watcher(token: &str) -> Result<u32> {
        ensure!(is_token(token), "Invalid trial token");
        crate::device::preflight()?;
        let mut command = Command::new(BINARY);
        command
            .args(["gate", "guard", "--watch", token])
            .env("PATH", crate::paths::SAFE_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: setsid is async-signal-safe and touches no Rust state.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command
            .spawn()
            .map_err(|e| Error::io("spawn trial guard", e))?;
        Ok(child.id())
    }

    /// `lg-native gate guard --spawn`: arm a watcher if a trial is pending.
    pub fn spawn(out: &mut impl Write) -> Result<()> {
        let token = read_token()?;
        let pid = match &token {
            Some(token) => Some(spawn_watcher(token)?),
            None => None,
        };
        let report =
            crate::json_object! {"guardPid" => pid.map(u64::from), "armed" => token.is_some()};
        let _ = writeln!(
            out,
            "{}",
            json::to_string(&report, json::Style::Spaced, false)
        );
        Ok(())
    }

    /// `lg-native gate guard --watch <token>`.
    pub fn run_watch(token: &str, out: &mut impl Write) -> Result<()> {
        ensure!(is_token(token), "Invalid trial token");
        let started = Instant::now();
        let result = watch(
            token,
            read_token,
            || {
                let state = crate::gate::load_state()?;
                crate::gate::disable_gate(state.as_ref(), &mut crate::gate::luna_call).map(drop)
            },
            || started.elapsed(),
            std::thread::sleep,
            Duration::from_secs(TRIAL_SECONDS),
        )?;
        let report = crate::json_object! {"result" => result.as_str()};
        let _ = writeln!(
            out,
            "{}",
            json::to_string(&report, json::Style::Spaced, false)
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn expiry_disables_only_the_same_unconfirmed_trial() {
        let now = Cell::new(Duration::ZERO);
        let mut calls = Vec::new();
        let result = watch(
            "token",
            || Ok(Some("token".to_owned())),
            || {
                calls.push("disabled");
                Ok(())
            },
            || now.get(),
            |d| now.set(now.get() + d),
            Duration::from_secs(3),
        );
        assert_eq!(result.ok(), Some(Watch::Recovered));
        assert_eq!(calls, ["disabled"]);
    }

    #[test]
    fn commit_cancels_the_guard_immediately_without_waiting() {
        let now = Cell::new(Duration::ZERO);
        let mut disabled = false;
        let result = watch(
            "token",
            || Ok(None),
            || {
                disabled = true;
                Ok(())
            },
            || now.get(),
            |d| now.set(now.get() + d),
            Duration::from_secs(3),
        );
        assert_eq!(result.ok(), Some(Watch::Cancelled));
        assert_eq!((now.get(), disabled), (Duration::ZERO, false));
    }

    #[test]
    fn a_replaced_token_cancels_and_a_read_error_never_disables() {
        let now = Cell::new(Duration::ZERO);
        let result = watch(
            "token",
            || Ok(Some("other".to_owned())),
            || Err(Error::Policy("must not run".into())),
            || now.get(),
            |d| now.set(now.get() + d),
            Duration::from_secs(3),
        );
        assert_eq!(result.ok(), Some(Watch::Cancelled));
        let result = watch(
            "token",
            || Err(Error::Policy("Untrusted trial token".into())),
            || Err(Error::Policy("must not run".into())),
            || now.get(),
            |d| now.set(now.get() + d),
            Duration::from_secs(3),
        );
        assert!(matches!(result, Err(Error::Policy(m)) if m == "Untrusted trial token"));
    }

    #[test]
    fn token_must_be_32_lowercase_hex_characters() {
        let ok = crate::json_object! {"token" => "a".repeat(32)};
        assert_eq!(parse_token(&ok).ok(), Some("a".repeat(32)));
        for bad in [
            Value::Null,
            Value::from(12u64),
            Value::from("short"),
            Value::from("A".repeat(32)),
            Value::from(format!("../{}", "a".repeat(32))),
        ] {
            let value = crate::json_object! {"token" => bad};
            assert!(parse_token(&value).is_err());
        }
        assert!(parse_token(&crate::json_object! {}).is_err());
    }
}
