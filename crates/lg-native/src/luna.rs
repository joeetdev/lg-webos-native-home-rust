//! The `luna-send` boundary to webOS LS2 services.
//!
//! LS2 is reached through the firmware's own `luna-send` client, one call per
//! process (`-n 1`), never through a shell. Response interpretation differs per
//! subsystem (error wording, duplicate-key policy), so this module only builds
//! the argument vector and runs the bounded process.

use crate::json::{self, Value};

/// The LS2 command-line client shipped with webOS.
pub const PROGRAM: &str = "luna-send";

/// Arguments for one call: `-n 1 luna://<method> <compact JSON payload>`.
pub fn argv(method: &str, payload: &Value) -> [String; 4] {
    [
        "-n".into(),
        "1".into(),
        format!("luna://{method}"),
        json::compact(payload),
    ]
}

#[cfg(unix)]
pub use unix::send;

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::error::Result;
    use crate::process::{self, Limits, Output};

    /// Runs one bounded `luna-send` call and returns the raw process output.
    pub fn send(method: &str, payload: &Value, limits: Limits) -> Result<Output> {
        let args = argv(method, payload);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        process::run(PROGRAM, &refs, limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_object;

    #[test]
    fn argv_is_exact_and_payload_is_compact_in_insertion_order() {
        let payload = json_object! {"id" => "com.webos.app.home", "preload" => "full"};
        assert_eq!(
            argv("com.webos.applicationManager/launch", &payload),
            [
                "-n".to_owned(),
                "1".to_owned(),
                "luna://com.webos.applicationManager/launch".to_owned(),
                r#"{"id":"com.webos.app.home","preload":"full"}"#.to_owned(),
            ]
        );
    }
}
