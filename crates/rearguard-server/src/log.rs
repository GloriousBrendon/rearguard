// SPDX-License-Identifier: MIT OR Apache-2.0

//! Minimal logging. Callers pass only non-secret values (session, match and connection
//! identifiers, counts, scores): seeds, the master secret and resume tokens never reach
//! a log line, and the tests check the output of whole sessions for them.

use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

type Sink = dyn Fn(&str) + Send + Sync;

/// Where log lines go.
#[derive(Clone)]
pub struct Logger(Arc<Sink>);

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Logger")
    }
}

impl Logger {
    /// Writes lines to standard error. A failed write (for example, the reading end of
    /// a pipe has closed) is ignored: logging must never take the server down, whereas
    /// `eprintln!` would panic.
    #[must_use]
    pub fn stderr() -> Self {
        Self(Arc::new(|line| {
            use std::io::Write as _;
            let _ = writeln!(std::io::stderr().lock(), "{line}");
        }))
    }

    /// Keeps lines in memory, for tests.
    #[must_use]
    pub fn capture() -> (Self, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let logger = Self(Arc::new(move |line: &str| {
            if let Ok(mut v) = sink.lock() {
                v.push(line.to_owned());
            }
        }));
        (logger, lines)
    }

    /// Logs one line with a millisecond Unix timestamp.
    pub fn info(&self, message: impl Display) {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        (self.0)(&format!("{ms} rearguard-server: {message}"));
    }
}
