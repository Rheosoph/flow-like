//! Deterministic destination ids for forks.
//!
//! Every row, node, pin and layer a fork creates gets its id from
//! `(seed, source id)` instead of a random draw, so re-running any fork
//! step after a crash or a lost commit produces the very same ids and the
//! DB writes collapse into `ON CONFLICT DO NOTHING`. The output has the
//! shape of [`flow_like_types::create_id`] (a leading letter followed by
//! base-36 digits, 24 characters) so nothing downstream can tell the two
//! apart.

use flow_like_types::create_id;

const ID_LENGTH: usize = 24;
const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// The destination id for `source_id` inside the fork identified by `seed`.
pub fn derive_id(seed: &str, source_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(seed.as_bytes());
    hasher.update(&[0]);
    hasher.update(source_id.as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();

    let mut id = String::with_capacity(ID_LENGTH);
    id.push((b'a' + bytes[0] % 26) as char);
    let mut body = u128::from_be_bytes(bytes[1..17].try_into().expect("16 hash bytes"));
    for _ in 1..ID_LENGTH {
        id.push(ALPHABET[(body % 36) as usize] as char);
        body /= 36;
    }
    id
}

/// A fresh, random seed for a fork that has no job row (offline bundles).
pub fn fresh_seed() -> String {
    create_id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ids_are_stable_and_scoped_to_the_seed() {
        let a = derive_id("job-a", "src");
        assert_eq!(a, derive_id("job-a", "src"));
        assert_ne!(a, derive_id("job-b", "src"));
        assert_ne!(a, derive_id("job-a", "other"));
    }

    #[test]
    fn derived_ids_look_like_cuids() {
        let random = create_id();
        for id in [derive_id("seed", "x"), derive_id("", "y")] {
            assert_eq!(id.len(), random.len());
            assert!(id.as_bytes()[0].is_ascii_lowercase());
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            );
        }
    }

    #[test]
    fn the_separator_keeps_prefixes_apart() {
        assert_ne!(derive_id("ab", "c"), derive_id("a", "bc"));
    }
}
