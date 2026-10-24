//! A single error type for every subsystem.
//!
//! The variants separate *why* an operation stopped, because callers treat them
//! differently: the remote-control CLI prints policy messages verbatim but hides
//! I/O and data details, and transactions propagate an interruption without
//! collecting it as an ordinary cleanup error.

use std::fmt;
use std::io;

/// Crate-wide result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why an operation failed.
#[derive(Debug)]
pub enum Error {
    /// A fail-closed safety or validation rule rejected the operation.
    Policy(String),
    /// External data (a file, a service response) was malformed.
    Data(String),
    /// An operating-system call failed.
    Io {
        /// What was being attempted, e.g. `"open /var/lib/lg-native/x"`.
        context: String,
        /// The underlying OS error.
        source: io::Error,
    },
    /// A bounded wait or child process exceeded its deadline.
    Timeout(String),
    /// SIGTERM asked the running transaction to unwind.
    Interrupted,
}

impl Error {
    /// Builds an [`Error::Io`] with a short description of the failed action.
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }

    /// True when the failure means "the path does not exist".
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Io { source, .. } if source.kind() == io::ErrorKind::NotFound)
    }

    /// Short machine-friendly label used in JSON error reports.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Policy(_) => "PolicyError",
            Error::Data(_) => "DataError",
            Error::Io { .. } => "IoError",
            Error::Timeout(_) => "TimeoutError",
            Error::Interrupted => "Interrupted",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Policy(message) | Error::Data(message) | Error::Timeout(message) => {
                f.write_str(message)
            }
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Interrupted => f.write_str("Terminated; unwinding Home transaction"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Fails with [`Error::Policy`] unless `condition` holds.
///
/// The message is only formatted on failure, so hot paths do not allocate.
#[macro_export]
macro_rules! ensure {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err($crate::error::Error::Policy(format!($($message)+)));
        }
    };
}

/// Returns an [`Error::Policy`] built from a format string.
#[macro_export]
macro_rules! policy {
    ($($message:tt)+) => {
        $crate::error::Error::Policy(format!($($message)+))
    };
}
