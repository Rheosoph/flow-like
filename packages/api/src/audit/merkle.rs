//! Merkle trees in the RFC 6962 / RFC 9162 shape, hashed with BLAKE3.
//!
//! Seals use a tree over their record hashes; epochs use a tree over their seal hashes.
//! An inclusion proof lets an app owner show that a seal is part of an epoch without
//! reading any other customer's seals.

use super::crypto::Hash;

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;

pub fn leaf_hash(data: &Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[LEAF_PREFIX]);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

fn node_hash(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[NODE_PREFIX]);
    hasher.update(left);
    hasher.update(right);
    *hasher.finalize().as_bytes()
}

/// Largest power of two strictly smaller than `n` (`n >= 2`).
fn split(n: usize) -> usize {
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// Merkle tree hash of `leaves`, which are data hashes, not leaf hashes.
pub fn root(leaves: &[Hash]) -> Hash {
    match leaves.len() {
        0 => *blake3::hash(b"").as_bytes(),
        1 => leaf_hash(&leaves[0]),
        n => {
            let k = split(n);
            node_hash(&root(&leaves[..k]), &root(&leaves[k..]))
        }
    }
}

/// Root and the inclusion proof of every leaf, in O(n log n).
pub fn root_with_proofs(leaves: &[Hash]) -> (Hash, Vec<Vec<Hash>>) {
    match leaves.len() {
        0 => (root(leaves), Vec::new()),
        1 => (leaf_hash(&leaves[0]), vec![Vec::new()]),
        n => {
            let k = split(n);
            let (left_root, mut left_proofs) = root_with_proofs(&leaves[..k]);
            let (right_root, mut right_proofs) = root_with_proofs(&leaves[k..]);
            left_proofs
                .iter_mut()
                .for_each(|proof| proof.push(right_root));
            right_proofs
                .iter_mut()
                .for_each(|proof| proof.push(left_root));
            left_proofs.append(&mut right_proofs);
            (node_hash(&left_root, &right_root), left_proofs)
        }
    }
}

/// RFC 9162 section 2.1.3.2.
pub fn verify_inclusion(
    leaf: &Hash,
    index: u64,
    tree_size: u64,
    proof: &[Hash],
    expected_root: &Hash,
) -> bool {
    if index >= tree_size {
        return false;
    }
    let (mut fn_, mut sn) = (index, tree_size - 1);
    let mut r = leaf_hash(leaf);
    for p in proof {
        if sn == 0 {
            return false;
        }
        if fn_ & 1 == 1 || fn_ == sn {
            r = node_hash(p, &r);
            if fn_ & 1 == 0 {
                while fn_ & 1 == 0 && fn_ != 0 {
                    fn_ >>= 1;
                    sn >>= 1;
                }
            }
        } else {
            r = node_hash(&r, p);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    sn == 0 && blake3::Hash::from(r) == blake3::Hash::from(*expected_root)
}

pub fn encode_proof(proof: &[Hash]) -> Vec<u8> {
    proof.iter().flat_map(|hash| hash.iter().copied()).collect()
}

pub fn decode_proof(bytes: &[u8]) -> Option<Vec<Hash>> {
    if !bytes.len().is_multiple_of(32) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(32)
            .map(|chunk| <[u8; 32]>::try_from(chunk).expect("32-byte chunk"))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Hash> {
        (0..n)
            .map(|i| *blake3::hash(&(i as u64).to_be_bytes()).as_bytes())
            .collect()
    }

    #[test]
    fn every_leaf_of_every_size_proves_against_the_root() {
        for n in 1..=70 {
            let data = leaves(n);
            let (root_hash, proofs) = root_with_proofs(&data);
            assert_eq!(root_hash, root(&data), "size {n}");
            assert_eq!(proofs.len(), n);
            for (index, proof) in proofs.iter().enumerate() {
                assert!(
                    verify_inclusion(&data[index], index as u64, n as u64, proof, &root_hash),
                    "size {n} leaf {index}"
                );
                let decoded = decode_proof(&encode_proof(proof)).unwrap();
                assert_eq!(&decoded, proof);
            }
        }
    }

    #[test]
    fn proofs_fail_for_wrong_leaf_index_size_or_root() {
        let data = leaves(13);
        let (root_hash, proofs) = root_with_proofs(&data);
        let proof = &proofs[5];
        assert!(!verify_inclusion(&data[6], 5, 13, proof, &root_hash));
        assert!(!verify_inclusion(&data[5], 6, 13, proof, &root_hash));
        // A size with a different path shape fails. Sizes 12 to 16 share leaf 5's path,
        // so the proof alone does not pin the size; the signed epoch's seal count does.
        assert!(!verify_inclusion(&data[5], 5, 7, proof, &root_hash));
        assert!(!verify_inclusion(&data[5], 5, 13, proof, &[0; 32]));
        assert!(!verify_inclusion(&data[5], 13, 13, proof, &root_hash));
        let mut truncated = proof.clone();
        truncated.pop();
        assert!(!verify_inclusion(&data[5], 5, 13, &truncated, &root_hash));
        assert!(decode_proof(&[0; 31]).is_none());
    }

    #[test]
    fn leaf_and_node_hashes_are_domain_separated() {
        let data = leaves(2);
        let two = root(&data);
        let mut concatenated = [0u8; 32];
        concatenated.copy_from_slice(&leaf_hash(&data[0]));
        assert_ne!(root(&[concatenated]), two);
        assert_ne!(root(&data[..1]), data[0]);
    }

    #[test]
    fn order_matters() {
        let data = leaves(4);
        let mut swapped = data.clone();
        swapped.swap(1, 2);
        assert_ne!(root(&data), root(&swapped));
    }
}
