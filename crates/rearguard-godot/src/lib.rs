//! Rearguard Godot binding: placeholder for the thin gdext layer over `rearguard-core`.
//!
//! No gdext dependency yet; logic belongs in `rearguard-core`, not here.

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
