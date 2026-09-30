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

    /// Reads a key written as 64 hexadecimal digits (surrounding whitespace ignored),
    /// as in a key file. Returns `None` for anything else; no partial key is kept.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.trim().as_bytes();
        if digits.len() != 2 * SECRET_KEY_LEN {
            return None;
        }
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        };
        let mut bytes = [0u8; SECRET_KEY_LEN];
        for (i, pair) in digits.chunks_exact(2).enumerate() {
            match (nibble(pair[0]), nibble(pair[1])) {
                (Some(hi), Some(lo)) => bytes[i] = hi << 4 | lo,
                _ => {
                    bytes.zeroize();
                    return None;
                }
            }
        }
        Some(Self::from_bytes(&mut bytes))
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
    fn from_hex_reads_64_digits_only() {
        let hex = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";
        let key = SecretKey::from_hex(&format!(" {hex}\n")).unwrap();
        assert_eq!(key.expose_secret()[1], 0x11);
        assert_eq!(key.expose_secret()[31], 0xff);
        for bad in ["", &hex[..62], &format!("{hex}0"), &hex.replace('a', "g")] {
            assert!(SecretKey::from_hex(bad).is_none(), "{bad}");
        }
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
