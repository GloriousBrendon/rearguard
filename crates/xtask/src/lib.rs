// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repository tasks, run with `cargo xtask`. One so far: `cargo xtask eval`, the
//! detector evaluation (task 1.12; see this crate's README).
//!
//! This crate must never depend on Godot or any other engine.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod config;
pub mod eval;
pub mod inputs;
pub mod report;
pub mod split;
pub mod svg;
pub mod trace;
pub mod wilson;
pub mod zip;
