//! Secret key material that redacts itself and zeroizes on drop.
//!
//! Every seed and derived secret in Rearguard lives in a [`SecretKey`] (or a typed
//! wrapper around one). `Debug` and `Display` never show the bytes, and the bytes are
//! overwritten with zeros when the key is dropped.
//!
//! Limits: Rust may copy a value when it is moved, and those stale copies are not
//! wiped. Keep secrets in place (borrow them, don't move them around) where possible.

use core::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Length of every Rearguard secret key, in bytes.
pub const SECRET_KEY_LEN: usize = 32;

/// 32 bytes of secret key material. Not `Clone`, not `Copy`, not comparable.
pub struct SecretKey {
    bytes: [u8; SECRET_KEY_LEN],
}

impl SecretKey {
    /// Takes the key from `bytes` and overwrites `bytes` with zeros.
    #[must_use]
    pub fn from_bytes(bytes: &mut [u8; SECRET_KEY_LEN]) -> Self {
        let key = Self { bytes: *bytes };
        bytes.zeroize();
        key
    }

    /// The raw key bytes. Only for key derivation, or for sending a seed to a client
    /// where the protocol requires it. Never log or print the result.
    #[must_use]
    pub fn expose_secret(&self) -> &[u8; SECRET_KEY_LEN] {
        &self.bytes
    }
}

impl Zeroize for SecretKey {
    fn zeroize(&mut self) {
        self.bytes.zeroize();
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for SecretKey {}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bytes_wipes_the_source() {
        let mut source = [0xA7; SECRET_KEY_LEN];
        let key = SecretKey::from_bytes(&mut source);
        assert_eq!(source, [0; SECRET_KEY_LEN]);
        assert_eq!(key.expose_secret(), &[0xA7; SECRET_KEY_LEN]);
    }

    #[test]
    fn zeroize_wipes_the_key() {
        let mut key = SecretKey::from_bytes(&mut [0xA7; SECRET_KEY_LEN]);
        key.zeroize();
        assert_eq!(key.expose_secret(), &[0; SECRET_KEY_LEN]);
    }

    #[test]
    fn is_zeroize_on_drop() {
        fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<SecretKey>();
    }

    #[test]
    fn debug_and_display_are_redacted() {
        let key = SecretKey::from_bytes(&mut [0xA7; SECRET_KEY_LEN]);
        for shown in [format!("{key:?}"), format!("{key:#?}"), format!("{key}")] {
            assert!(shown.contains("<redacted>"), "{shown}");
            assert!(!shown.contains("167"), "{shown}");
            assert!(!shown.to_lowercase().contains("a7"), "{shown}");
        }
    }
}
