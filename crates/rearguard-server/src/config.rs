// SPDX-License-Identifier: MIT OR Apache-2.0

//! Server configuration, read from a JSON file. There are no defaults: every limit and
//! detector threshold is the operator's choice, and none of it is ever sent to a client.

use std::net::SocketAddr;
use std::path::PathBuf;

use rearguard_core::detect::DetectorSet;
use rearguard_core::probe::Amplitude;
use serde::Deserialize;

/// Everything the server needs to run.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Address to listen on. Must be a loopback address until TLS and authentication
    /// exist (task 3.3). Port 0 picks a free port.
    pub listen: SocketAddr,
    /// File holding the master secret (64 hex digits); see `rearguard-server gen-secret`.
    pub master_secret_file: PathBuf,
    /// SQLite database for sessions and evidence (created if missing).
    pub database: PathBuf,
    /// Probe drift amplitude handed to clients, ppm.
    pub amplitude_ppm: u32,
    /// Detector thresholds: one set for every scenario, or one set per scenario
    /// (`{"scenarios": {"flick": {...}, "spray": {...}}}`, task 1.12). With one set per
    /// scenario, a session whose telemetry names a scenario without a set is refused.
    pub detector: DetectorSet,
    /// Per-connection and server-wide limits.
    pub limits: Limits,
}

/// Limits that protect the server from oversized, abusive or abandoned clients.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Largest accepted frame payload, bytes. Longer frames close the connection before
    /// their payload is read.
    pub max_frame_bytes: usize,
    /// Most telemetry records in one message.
    pub max_records_per_message: usize,
    /// Sustained messages per second per connection.
    pub messages_per_second: f64,
    /// Message burst allowance per connection.
    pub message_burst: f64,
    /// Sustained payload bytes per second per connection.
    pub bytes_per_second: f64,
    /// Byte burst allowance per connection.
    pub byte_burst: f64,
    /// A connection that sends no complete frame for this long is closed (half-open or
    /// stalled peers), milliseconds.
    pub idle_timeout_ms: u64,
    /// A session whose connection dropped may be resumed for this long; after that it is
    /// closed as abandoned and its evidence stored, milliseconds.
    pub resume_timeout_ms: u64,
    /// Most sessions open at once.
    pub max_open_sessions: usize,
}

impl Config {
    /// Parses a configuration from JSON and checks it.
    ///
    /// # Errors
    /// Unknown or missing fields, or a value out of range (see [`Config::validate`]).
    pub fn from_json(text: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }

    /// Checks ranges, every detector configuration, and that the listen address is
    /// loopback.
    ///
    /// # Errors
    /// A description of the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        if !self.listen.ip().is_loopback() {
            return Err(format!(
                "listen address {} is not loopback; only localhost is allowed until TLS and authentication (task 3.3)",
                self.listen
            ));
        }
        Amplitude::from_ppm(self.amplitude_ppm).map_err(|e| e.to_string())?;
        self.detector.validate().map_err(|e| e.to_string())?;
        let l = &self.limits;
        let positive = [
            l.messages_per_second,
            l.message_burst,
            l.bytes_per_second,
            l.byte_burst,
        ];
        if positive.iter().any(|v| !(v.is_finite() && *v > 0.0)) {
            return Err("rate limits must be positive".to_owned());
        }
        if l.max_frame_bytes < 64
            || l.max_records_per_message == 0
            || l.idle_timeout_ms == 0
            || l.resume_timeout_ms == 0
            || l.max_open_sessions == 0
        {
            return Err("limits must be positive (max_frame_bytes at least 64)".to_owned());
        }
        if l.byte_burst < l.max_frame_bytes as f64 {
            return Err("byte_burst must allow at least one full frame".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn example_json() -> String {
        include_str!("../config/server.example.json").to_owned()
    }

    #[test]
    fn the_example_config_is_valid() {
        let config = Config::from_json(&example_json()).unwrap();
        assert!(config.listen.ip().is_loopback());
    }

    #[test]
    fn non_loopback_and_bad_limits_are_refused() {
        for (from, to) in [
            ("\"127.0.0.1:7461\"", "\"0.0.0.0:7461\""),
            ("\"127.0.0.1:7461\"", "\"192.168.1.10:7461\""),
            ("\"amplitude_ppm\": 5000", "\"amplitude_ppm\": 30000"),
            (
                "\"messages_per_second\": 400.0",
                "\"messages_per_second\": 0.0",
            ),
            ("\"max_frame_bytes\": 262144", "\"max_frame_bytes\": 8"),
        ] {
            let text = example_json().replace(from, to);
            assert_ne!(text, example_json(), "{from}");
            assert!(Config::from_json(&text).is_err(), "{to}");
        }
        let extra = example_json().replacen('{', "{\"surprise\": 1,", 1);
        assert!(Config::from_json(&extra).is_err(), "unknown field");
    }

    /// Task 1.12: one threshold set per scenario.
    #[test]
    fn the_detector_may_have_one_threshold_set_per_scenario() {
        let single = Config::from_json(&example_json()).unwrap();
        assert!(matches!(single.detector, DetectorSet::Single(_)));

        let one = r#"{"kappa_bound": 2.0, "error_flag_score": @, "steps_flag_score": 4.25,
            "min_pairs": 20, "window_ms": 30000, "min_step_counts": 100.0}"#;
        let per_scenario = format!(
            r#""detector": {{"scenarios": {{"flick": {}, "spray": {}}}}}, "limits""#,
            one.replace('@', "11.25"),
            one.replace('@', "6.5")
        );
        let text = example_json();
        let start = text.find("\"detector\"").unwrap();
        let end = text.find("\"limits\"").unwrap() + "\"limits\"".len();
        let text = format!("{}{per_scenario}{}", &text[..start], &text[end..]);
        let config = Config::from_json(&text).unwrap();
        let score = |scenario: &str| {
            config
                .detector
                .for_scenario(scenario)
                .map(|c| c.error_flag_score)
        };
        assert_eq!(score("flick"), Some(11.25));
        assert_eq!(score("spray"), Some(6.5));
        assert_eq!(score("tracking"), None);

        // Out-of-range thresholds are refused when the configuration is read, in either
        // form, not when the first session arrives.
        for bad in [
            text.replace("\"min_pairs\": 20", "\"min_pairs\": 1"),
            example_json().replace("\"window_ms\": 30000", "\"window_ms\": 0"),
            text.replace("\"scenarios\": {", "\"scenarios\": {}, \"other\": {"),
        ] {
            assert!(Config::from_json(&bad).is_err(), "{bad}");
        }
    }
}
