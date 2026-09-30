//! Rearguard server: issues session seeds, ingests untrusted client telemetry, runs the
//! input-probe detector and stores the evidence (task 1.6).
//!
//! - Seeds are derived from a master secret (HKDF, `rearguard_core::probe`) and never
//!   stored; the database holds only `(match_id, player_id)`.
//! - Transport: TCP on loopback only, length-prefixed postcard frames with a version
//!   byte (`rearguard_core::protocol`). TLS and authentication are task 3.3.
//! - Evidence: SQLite ([`store`]).
//!
//! This crate must never depend on Godot or any other engine.

#![forbid(unsafe_code)]

pub mod client;
pub mod config;
mod limit;
pub mod log;
pub mod server;
pub mod store;

use std::io;
use std::path::Path;

use rearguard_core::probe::RootSeed;
use zeroize::Zeroize;

/// Version of `rearguard-core` this crate was built against.
pub fn core_version() -> &'static str {
    rearguard_core::VERSION
}

/// Reads the master secret from its file (64 hex digits). The file's text is wiped
/// from memory after parsing.
///
/// # Errors
/// I/O errors, or a file that is not 64 hex digits.
pub fn load_master_secret(path: &Path) -> io::Result<RootSeed> {
    let mut text = std::fs::read_to_string(path)?;
    let root = RootSeed::from_hex(&text);
    text.zeroize();
    root.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "master secret file is not 64 hex digits",
        )
    })
}

/// Creates a new master secret file (it must not exist), from the OS CSPRNG, readable
/// only by its owner on Unix.
///
/// # Errors
/// I/O errors, including an existing file.
pub fn create_master_secret(path: &Path) -> io::Result<()> {
    use std::io::Write as _;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let mut text: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    bytes.zeroize();
    text.push('\n');
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let result = options
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()));
    text.zeroize();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_against_core() {
        assert_eq!(core_version(), rearguard_core::VERSION);
    }
}
