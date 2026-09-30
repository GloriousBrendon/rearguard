//! Rearguard sim: the closed test environment for synthetic players and test cheats.
//!
//! Simulated players (a closed-loop human model, a recoil macro and five aimbots)
//! play a Rust mirror of the Godot aim range under the input-probe drift, and emit the
//! same telemetry a real client would ([`rearguard_core::telemetry`]).
//!
//! **Simulated humans are not real humans.** See `README.md`.
//!
//! Test cheats run only inside this simulator. Nothing here injects real input,
//! touches another process, or talks to a server.
//!
//! This crate must never depend on Godot or any other engine.

#![forbid(unsafe_code)]

pub mod aimbot;
pub mod generate;
pub mod human;
pub mod seed;
pub mod session;
pub mod validation;
pub mod world;

/// Version of `rearguard-core` this crate was built against.
pub fn core_version() -> &'static str {
    rearguard_core::VERSION
}

#[cfg(test)]
mod tests;
