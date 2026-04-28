use crate::flow::board::Board;
use crate::flow::node::NodeLogic;
use ahash::AHashMap;
use highway::{HighwayHash, HighwayHasher};
use std::sync::Arc;
use std::time::Instant;

/// Precompiled board cache for a specific board version.
///
/// Caches **only stateless, immutable data** from graph construction:
/// - `NodeLogic` instances (`Arc<dyn NodeLogic>` — stateless trait objects)
/// - Content hash for cache invalidation
///
/// Each execution run creates its own fresh `InternalPin` and `InternalNode`
/// instances with independent mutable state (pin values, exec counters),
/// ensuring full isolation between concurrent runs.
///
/// # What This Skips
///
/// The expensive `registry.instantiate()` call for each node is skipped on
/// cache hits. For WASM nodes this can involve module loading; for native
/// nodes it's a registry lookup + factory call. All other graph construction
/// (pin creation, OnceLock wiring, node assembly) runs fresh each time to
/// guarantee thread safety.
///
/// # Thread Safety
///
/// - `NodeLogic` is `Send + Sync` by trait bound — safe to share across runs
/// - No mutable runtime state is stored in this struct
/// - Each `InternalRun` gets its own pins, nodes, variables, and cache
pub struct CompiledBoard {
    /// Content hash of the board at compilation time (HighwayHash).
    /// Used as the cache invalidation key — if the board changes,
    /// its hash changes and triggers recompilation.
    pub content_hash: u64,

    /// Board version tuple `(major, minor, patch)` at compile time.
    pub board_version: (u32, u32, u32),

    /// Cached `NodeLogic` instances keyed by node ID.
    /// These are stateless `Arc<dyn NodeLogic>` trait objects produced by
    /// `registry.instantiate()`. Safe to share across concurrent runs
    /// because `NodeLogic::run()` takes `&self` (not `&mut self`).
    pub node_logic: AHashMap<String, Arc<dyn NodeLogic>>,

    /// Timestamp when this compilation occurred, for diagnostics.
    pub compiled_at: Instant,
}

impl CompiledBoard {
    /// Check if this compiled board is still valid for the given board.
    ///
    /// Compares the stored content hash against a freshly computed one.
    /// If any node, pin, layer, or variable has changed, the hash will
    /// differ and this returns `false`.
    #[inline]
    pub fn is_valid_for(&self, board: &Board) -> bool {
        let current_hash = Self::compute_board_hash(board);
        self.content_hash == current_hash
    }

    /// Compute a content hash for the entire board topology.
    ///
    /// Combines individual node hashes (already computed via `Node::hash()`)
    /// with layer hashes, variable hashes, and structural metadata to produce
    /// a single `u64` fingerprint. Any change to the board's graph structure
    /// will produce a different hash.
    pub fn compute_board_hash(board: &Board) -> u64 {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0706050403020100,
            0x0f0e0d0c0b0a0908,
            0x1716151413121110,
            0x1f1e1d1c1b1a1918,
        ]));

        hasher.append(board.id.as_bytes());
        hasher.append(&board.version.0.to_le_bytes());
        hasher.append(&board.version.1.to_le_bytes());
        hasher.append(&board.version.2.to_le_bytes());

        Self::hash_nodes(&mut hasher, board);
        Self::hash_layers(&mut hasher, board);
        Self::hash_variables(&mut hasher, board);

        hasher.finalize64()
    }

    fn hash_nodes(hasher: &mut HighwayHasher, board: &Board) {
        let mut ids: Vec<&String> = board.nodes.keys().collect();
        ids.sort();
        for id in ids {
            hasher.append(id.as_bytes());
            if let Some(node) = board.nodes.get(id) {
                let bytes = node
                    .hash
                    .map(|h| h.to_le_bytes().to_vec())
                    .unwrap_or_else(|| node.name.as_bytes().to_vec());
                hasher.append(&bytes);
            }
        }
    }

    fn hash_layers(hasher: &mut HighwayHasher, board: &Board) {
        let mut ids: Vec<&String> = board.layers.keys().collect();
        ids.sort();
        for id in ids {
            hasher.append(id.as_bytes());
            if let Some(layer) = board.layers.get(id) {
                if let Some(hash) = layer.hash {
                    hasher.append(&hash.to_le_bytes());
                }
            }
        }
    }

    fn hash_variables(hasher: &mut HighwayHasher, board: &Board) {
        let mut ids: Vec<&String> = board.variables.keys().collect();
        ids.sort();
        for id in ids {
            hasher.append(id.as_bytes());
            if let Some(var) = board.variables.get(id) {
                if let Some(hash) = var.hash {
                    hasher.append(&hash.to_le_bytes());
                }
            }
        }
    }
}
