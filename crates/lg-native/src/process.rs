//! Bounded, single-threaded child-process execution.
//!
//! Children get `/dev/null` as stdin, a fixed `PATH` and no shell. Output pipes
//! are drained with `poll(2)` from one thread (no reader threads, no extra
//! stacks), capped in size, and the child is killed when the deadline passes or
//! — for interruptible calls — when SIGTERM arrives.

use crate::error::{Error, Result};
use crate::paths::SAFE_PATH;
use crate::signals;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How to run one child process.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Wall-clock budget for the whole call.
    pub timeout: Duration,
    /// Largest accepted stdout (and, separately, stderr) size in bytes.
    pub max_output: usize,
    /// Capture stderr (otherwise it goes to `/dev/null`).
    pub capture_stderr: bool,
    /// Abort (killing the child) when SIGTERM is pending.
    pub interruptible: bool,
}

/// The result of a completed child process.
#[derive(Debug)]
pub struct Output {
    /// Exit status.
    pub status: ExitStatus,
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr (empty when not captured).
    pub stderr: Vec<u8>,
}

struct Pipe<R> {
    reader: Option<R>,
    data: Vec<u8>,
}

#[allow(unsafe_code)]
fn set_nonblocking(fd: i32) -> std::io::Result<()> {
    // SAFETY: fcntl on a valid, owned descriptor with integer arguments.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn stop(child: &mut Child) {
    // The child may already have exited; either way it must be reaped.
    let _ = child.kill();
    let _ = child.wait();
}

enum DrainError {
    Limit,
    Io(std::io::Error),
}

/// Reads whatever is available; returns `false` at end of stream.
fn drain<R: Read>(pipe: &mut Pipe<R>, limit: usize) -> std::result::Result<bool, DrainError> {
    let Some(reader) = pipe.reader.as_mut() else {
        return Ok(false);
    };
    let mut block = [0u8; 4096];
    loop {
        match reader.read(&mut block) {
            Ok(0) => {
                pipe.reader = None;
                return Ok(false);
            }
            Ok(n) => {
                if pipe.data.len() + n > limit {
                    return Err(DrainError::Limit);
                }
                pipe.data.extend_from_slice(&block[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(DrainError::Io(e)),
        }
    }
}

/// Runs `program` (resolved through the fixed safe `PATH`) with `args`.
#[allow(unsafe_code)]
pub fn run(program: &str, args: &[&str], limits: Limits) -> Result<Output> {
    let started = Instant::now();
    let deadline = started + limits.timeout;
    let mut child = Command::new(program)
        .args(args)
        .env("PATH", SAFE_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if limits.capture_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(|e| Error::io(format!("run {program}"), e))?;
    let mut out = Pipe {
        reader: child.stdout.take(),
        data: Vec::new(),
    };
    let mut err = Pipe {
        reader: child.stderr.take(),
        data: Vec::new(),
    };
    for fd in [
        out.reader.as_ref().map(|r| r.as_raw_fd()),
        err.reader.as_ref().map(|r| r.as_raw_fd()),
    ]
    .into_iter()
    .flatten()
    {
        if let Err(e) = set_nonblocking(fd) {
            stop(&mut child);
            return Err(Error::io(format!("configure {program} pipe"), e));
        }
    }

    let fail = |child: &mut Child, error: Error| -> Result<Output> {
        stop(child);
        Err(error)
    };

    while out.reader.is_some() || err.reader.is_some() {
        if limits.interruptible && signals::take() {
            return fail(&mut child, Error::Interrupted);
        }
        let now = Instant::now();
        if now >= deadline {
            return fail(
                &mut child,
                Error::Timeout(format!("{program} timed out after {:?}", limits.timeout)),
            );
        }
        let wait = (deadline - now).min(Duration::from_millis(100));
        let mut fds = Vec::with_capacity(2);
        for fd in [
            out.reader.as_ref().map(|r| r.as_raw_fd()),
            err.reader.as_ref().map(|r| r.as_raw_fd()),
        ]
        .into_iter()
        .flatten()
        {
            fds.push(libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            });
        }
        // SAFETY: `fds` is a valid, correctly sized array of pollfd structures.
        let ready = unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                fds.len() as libc::nfds_t,
                wait.as_millis() as libc::c_int,
            )
        };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return fail(&mut child, Error::io(format!("poll {program}"), error));
        }
        for result in [
            drain(&mut out, limits.max_output),
            drain(&mut err, limits.max_output),
        ] {
            match result {
                Ok(_) => {}
                Err(DrainError::Limit) => {
                    let error = Error::Data(format!(
                        "{program} output exceeds {} bytes",
                        limits.max_output
                    ));
                    return fail(&mut child, error);
                }
                Err(DrainError::Io(e)) => {
                    return fail(&mut child, Error::io(format!("read {program} output"), e));
                }
            }
        }
    }

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Output {
                    status,
                    stdout: out.data,
                    stderr: err.data,
                })
            }
            Ok(None) => {}
            Err(e) => return fail(&mut child, Error::io(format!("wait {program}"), e)),
        }
        if limits.interruptible && signals::take() {
            return fail(&mut child, Error::Interrupted);
        }
        if Instant::now() >= deadline {
            return fail(
                &mut child,
                Error::Timeout(format!("{program} timed out after {:?}", limits.timeout)),
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(timeout_ms: u64) -> Limits {
        Limits {
            timeout: Duration::from_millis(timeout_ms),
            max_output: 1024,
            capture_stderr: true,
            interruptible: false,
        }
    }

    #[test]
    fn captures_stdout_stderr_and_status_without_a_shell_for_the_caller() {
        let output = run(
            "sh",
            &["-c", "printf out; printf err >&2; exit 3"],
            limits(5000),
        )
        .unwrap();
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn stdin_is_null_and_path_is_fixed() {
        let output = run("sh", &["-c", "cat; printf %s \"$PATH\""], limits(5000)).unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), SAFE_PATH);
    }

    #[test]
    fn deadline_kills_a_hung_child() {
        let started = Instant::now();
        let result = run("sh", &["-c", "sleep 5"], limits(200));
        assert!(matches!(result, Err(Error::Timeout(_))), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
        // A child that closes its pipes but keeps running is also bounded.
        let result = run("sh", &["-c", "exec >/dev/null 2>&1; sleep 5"], limits(200));
        assert!(matches!(result, Err(Error::Timeout(_))), "{result:?}");
    }

    #[test]
    fn oversized_output_is_refused() {
        let result = run("sh", &["-c", "head -c 5000 /dev/zero"], limits(5000));
        assert!(matches!(result, Err(Error::Data(_))), "{result:?}");
    }

    #[test]
    fn missing_program_is_an_io_error() {
        let result = run("lg-native-definitely-missing", &[], limits(1000));
        assert!(matches!(result, Err(Error::Io { .. })), "{result:?}");
    }
}
