//! SHA-256 helpers. Hashing streams through a fixed stack buffer so even large
//! QML files never need a heap copy.

use sha2::{Digest, Sha256};
use std::io::Read;

/// Read buffer used for streaming hashes and copies (fits easily on the stack).
pub const BLOCK: usize = 16 * 1024;

/// Incremental SHA-256.
#[derive(Clone, Debug, Default)]
pub struct Hasher(Sha256);

impl Hasher {
    /// A fresh hasher.
    pub fn new() -> Self {
        Hasher(Sha256::new())
    }

    /// Adds bytes.
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Finishes and returns lowercase hexadecimal.
    pub fn hex(self) -> String {
        to_hex(&self.0.finalize())
    }
}

/// Lowercase hexadecimal SHA-256 of `bytes`.
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes.as_ref());
    hasher.hex()
}

/// Hashes everything readable from `reader`.
pub fn hex_reader(mut reader: impl Read) -> std::io::Result<String> {
    let mut hasher = Hasher::new();
    let mut block = [0u8; BLOCK];
    loop {
        match reader.read(&mut block) {
            Ok(0) => return Ok(hasher.hex()),
            Ok(n) => hasher.update(&block[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// Lowercase hexadecimal encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0xF)] as char);
    }
    out
}

/// True for exactly 64 lowercase hexadecimal characters.
pub fn is_hex64(text: &str) -> bool {
    is_lower_hex(text, 64)
}

/// True for exactly `len` lowercase hexadecimal characters.
pub fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = vec![b'a'; BLOCK * 3 + 7];
        assert_eq!(hex_reader(&long[..]).ok(), Some(hex(&long)));
    }

    #[test]
    fn hex_validation_is_exact_and_lowercase() {
        assert!(is_hex64(&"a".repeat(64)));
        for bad in [
            "",
            "A".repeat(64).as_str(),
            "g".repeat(64).as_str(),
            "a".repeat(63).as_str(),
        ] {
            assert!(!is_hex64(bad));
        }
        assert!(!is_hex64(&format!("{}\n", "a".repeat(64))));
    }
}
