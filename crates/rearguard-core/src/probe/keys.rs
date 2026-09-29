//! Probe key hierarchy.
//!
//! ```text
//! RootSeed ─match(id)─▶ MatchKey ─player(id)─▶ PlayerKey ─epoch(n)─▶ EpochSeed ─stream(s, shape)─▶ stream key
//! ```
//!
//! Each arrow is HKDF-SHA256 (RFC 5869) with the parent key as input keying material,
//! a fixed protocol salt, and an `info` string that names the step and its context, so
//! no two steps or contexts can produce the same key. Everything left of `EpochSeed`
//! stays on the server. The `EpochSeed` is what a client receives for one epoch
//! (decision D5, pending); stream keys are derived from it on both sides.

use hkdf::Hkdf;
use sha2::Sha256;

use crate::secret::{SECRET_KEY_LEN, SecretKey};

/// HKDF salt shared by every derivation step. Changing it changes every key.
const KDF_SALT: &[u8] = b"rearguard/probe-kdf/v0";

/// Derives a child key: `HKDF-SHA256(salt, ikm = parent, info = label || context)`.
///
/// `info` is `len(label) as u64 LE || label || len(context) as u64 LE || context`, so
/// distinct (label, context) pairs never encode to the same bytes.
fn derive(parent: &SecretKey, label: &str, context: &[u8]) -> SecretKey {
    let hkdf = Hkdf::<Sha256>::new(Some(KDF_SALT), parent.expose_secret());
    let label_len = (label.len() as u64).to_le_bytes();
    let context_len = (context.len() as u64).to_le_bytes();
    let mut okm = [0u8; SECRET_KEY_LEN];
    hkdf.expand_multi_info(
        &[&label_len, label.as_bytes(), &context_len, context],
        &mut okm,
    )
    .expect("32 bytes is a valid HKDF-SHA256 output length");
    SecretKey::from_bytes(&mut okm)
}

/// The server's root secret. Never leaves the server.
#[derive(Debug)]
pub struct RootSeed(SecretKey);

impl RootSeed {
    /// Takes the root seed from `bytes` and overwrites `bytes` with zeros.
    #[must_use]
    pub fn from_bytes(bytes: &mut [u8; SECRET_KEY_LEN]) -> Self {
        Self(SecretKey::from_bytes(bytes))
    }

    /// The key for one match. `match_id` is any stable identifier, such as a UUID.
    #[must_use]
    pub fn match_key(&self, match_id: &[u8]) -> MatchKey {
        MatchKey(derive(&self.0, "match", match_id))
    }
}

/// Per-match key. Server only.
#[derive(Debug)]
pub struct MatchKey(SecretKey);

impl MatchKey {
    /// The key for one player in this match.
    #[must_use]
    pub fn player_key(&self, player_id: u64) -> PlayerKey {
        PlayerKey(derive(&self.0, "player", &player_id.to_le_bytes()))
    }
}

/// Per-player key within a match. Server only.
#[derive(Debug)]
pub struct PlayerKey(SecretKey);

impl PlayerKey {
    /// The seed for epoch `epoch` (see [`EpochSchedule`](super::EpochSchedule)).
    /// In v0 there is a single epoch, number 0, covering the whole match.
    #[must_use]
    pub fn epoch_seed(&self, epoch: u64) -> EpochSeed {
        EpochSeed(derive(&self.0, "epoch", &epoch.to_le_bytes()))
    }
}

/// Seed for one player's probe signal during one epoch. The only secret a client
/// ever holds: knowing it reveals nothing about other epochs, players or matches.
#[derive(Debug)]
pub struct EpochSeed(SecretKey);

impl EpochSeed {
    /// Takes the seed from `bytes` (for example, as received by the client) and
    /// overwrites `bytes` with zeros.
    #[must_use]
    pub fn from_bytes(bytes: &mut [u8; SECRET_KEY_LEN]) -> Self {
        Self(SecretKey::from_bytes(bytes))
    }

    /// The raw seed, only for delivering it to the client it belongs to. Never log it.
    #[must_use]
    pub fn expose_secret(&self) -> &[u8; SECRET_KEY_LEN] {
        self.0.expose_secret()
    }

    /// The key for one stream (sensitivity or recoil) under one signal shape.
    pub(crate) fn stream_key(&self, stream_tag: u8, shape_tag: u8) -> SecretKey {
        derive(&self.0, "stream", &[stream_tag, shape_tag])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> RootSeed {
        RootSeed::from_bytes(&mut [7; SECRET_KEY_LEN])
    }

    #[test]
    fn derivation_is_deterministic() {
        let a = root().match_key(b"m").player_key(1).epoch_seed(0);
        let b = root().match_key(b"m").player_key(1).epoch_seed(0);
        assert_eq!(a.expose_secret(), b.expose_secret());
    }

    #[test]
    fn every_context_gives_a_distinct_seed() {
        let root = root();
        let seeds = [
            root.match_key(b"m").player_key(1).epoch_seed(0),
            root.match_key(b"n").player_key(1).epoch_seed(0),
            root.match_key(b"m").player_key(2).epoch_seed(0),
            root.match_key(b"m").player_key(1).epoch_seed(1),
            // Length prefixes keep "m" + player 1 apart from other splits of the bytes.
            root.match_key(b"").player_key(1).epoch_seed(0),
        ];
        for (i, a) in seeds.iter().enumerate() {
            for b in &seeds[i + 1..] {
                assert_ne!(a.expose_secret(), b.expose_secret());
            }
        }
    }

    #[test]
    fn stream_keys_are_separated_by_stream_and_shape() {
        let seed = root().match_key(b"m").player_key(1).epoch_seed(0);
        let keys = [
            seed.stream_key(0, 0),
            seed.stream_key(1, 0),
            seed.stream_key(0, 1),
        ];
        assert_ne!(keys[0].expose_secret(), keys[1].expose_secret());
        assert_ne!(keys[0].expose_secret(), keys[2].expose_secret());
        assert_ne!(keys[1].expose_secret(), keys[2].expose_secret());
        assert_ne!(keys[0].expose_secret(), seed.expose_secret());
    }
}
