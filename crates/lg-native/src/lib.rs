//! Device-side implementation of the LG webOS native Home customisation.
//!
//! Everything here runs as short-lived, one-shot commands from a single static
//! binary: there is no daemon, no async runtime and no thread pool. The design
//! goals, in order, are: never leave the TV without a usable Home, never touch
//! state this project does not own, and use as little memory as possible.
//!
//! * [`home`] — the transactional bind-mount runtime for the eight Home files.
//! * [`gate`] — the early-presentation gate: recovery, installer, trial guard.
//! * [`remote`] — ownership of the remote-control key filter.
//! * [`checkpoint`] — removal of derived Home CRIU checkpoints only.
//!
//! Pure validation logic is platform independent and unit tested; Linux I/O
//! lives behind small traits so the transactions run against fakes in tests.

// `unsafe` is denied everywhere except the few audited libc calls (geteuid,
// sigaction, fcntl/poll, mount/umount2, kill, setsid, flock), each of which is
// individually allowed and documented with a SAFETY comment.
#![deny(unsafe_code)]

pub mod error;
pub mod json;
pub mod paths;
pub mod sha;

pub mod fsx;
pub mod luna;
#[cfg(unix)]
pub mod process;
pub mod signals;

pub mod checkpoint;
#[cfg(unix)]
pub mod device;
pub mod gate;
pub mod home;
pub mod remote;

pub use error::{Error, Result};
