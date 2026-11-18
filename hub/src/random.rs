//! Random codes and tokens, from the OS's secure random source.

use kit_protocol::proof;

/// Letters and digits that can't be mixed up when read off a screen (no 0/O, 1/I/L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

fn bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("the OS random source failed");
    out
}

fn code(len: usize) -> String {
    // Rejection sampling, so every character is equally likely.
    let limit = 256 - (256 % CODE_ALPHABET.len());
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for b in bytes::<32>() {
            if (b as usize) < limit && out.len() < len {
                out.push(CODE_ALPHABET[b as usize % CODE_ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// The 8-character code that lets the first parent set the family up.
pub fn setup_code() -> String {
    code(8)
}

/// A 6-character code a parent makes so a new device can join as a member.
pub fn enroll_code() -> String {
    code(6)
}

/// A device's secret token: 32 random bytes, as hex.
pub fn token() -> String {
    proof::hex(&bytes::<32>())
}

pub fn hash_token(token: &str) -> String {
    proof::token_hash(token)
}

/// Codes are typed by kids: ignore case, spaces and dashes.
pub fn clean_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// Compares two strings in time that doesn't depend on where they differ.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_use_the_readable_alphabet() {
        let c = enroll_code();
        assert_eq!(c.len(), 6);
        assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert_eq!(setup_code().len(), 8);
    }

    #[test]
    fn tokens_are_unique_and_hash_stably() {
        let (a, b) = (token(), token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert_eq!(hash_token(&a), hash_token(&a));
    }

    #[test]
    fn clean_code_forgives_typing() {
        assert_eq!(clean_code(" ab-c 2d3e "), "ABC2D3E");
    }
}
