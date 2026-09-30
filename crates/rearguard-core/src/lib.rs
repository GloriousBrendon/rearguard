// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rearguard core: engine-agnostic probe, telemetry protocol and detection logic.
//!
//! This crate must never depend on Godot or any other engine.

#![forbid(unsafe_code)]

pub mod detect;
pub mod probe;
pub mod protocol;
pub mod secret;
pub mod study;
pub mod telemetry;
pub mod uplink;

/// Version of the core crate, used by dependants to prove they link against it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_manifest() {
        assert_eq!(VERSION, "0.0.0");
    }
}
