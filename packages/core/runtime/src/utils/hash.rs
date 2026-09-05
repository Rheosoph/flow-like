use ahash::AHasher;
use flow_like_storage::blake3;
use std::hash::{Hash, Hasher};
use std::{collections::BTreeMap, path::PathBuf};

pub fn hash_file(file_path: &PathBuf) -> String {
    if !file_path.exists() || !file_path.is_file() {
        return String::from("");
    }

    use std::fs::File;
    use std::io::{BufReader, Read};

    let file = File::open(file_path).unwrap();
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();

    let mut buffer = [0; 1024];
    loop {
        let n = reader.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    let result = hasher.finalize();
    let result = result.to_hex();

    result.to_string()
}

pub fn hash_string_non_cryptographic(input: &str) -> u64 {
    let fixed_seed = ahash::RandomState::with_seeds(9657, 9657, 9657, 9657);
    fixed_seed.hash_one(input)
}

pub fn hash_string(input: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let result = result.to_hex();
    result.to_string()
}

pub fn hash_bytes(input: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(input);
    let result = hasher.finalize();
    let result = result.to_hex();
    result.to_string()
}

pub fn hash_btree_map<T: Hash>(map: &BTreeMap<String, T>) -> u64 {
    let mut hasher = AHasher::default();

    for key in map.keys() {
        key.hash(&mut hasher);
    }

    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_file() {
        let input = "Hi there :)";
        let hash1 = hash_string_non_cryptographic(input);
        let hash2 = hash_string_non_cryptographic(input);
        assert_eq!(hash1, hash2);
    }
}
