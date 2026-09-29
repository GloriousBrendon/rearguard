//! Rearguard server: holds probe seeds and judges untrusted client telemetry.
//!
//! This crate must never depend on Godot or any other engine.

#![forbid(unsafe_code)]

/// Version of `rearguard-core` this crate was built against.
pub fn core_version() -> &'static str {
    rearguard_core::VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_against_core() {
        assert_eq!(core_version(), rearguard_core::VERSION);
    }
}
