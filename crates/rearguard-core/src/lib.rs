//! Rearguard core: engine-agnostic probe, protocol and detection logic.
//!
//! This crate must never depend on Godot or any other engine.

#![forbid(unsafe_code)]

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

/// DELIBERATE CLIPPY WARNING: proves CI fails on warnings. Reverted in the next commit.
pub fn deliberate_clippy_warning() -> u32 {
    return 1;
}
