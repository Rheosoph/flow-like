use crate::flow::{board::Board, node::FnRefs};

pub mod add_node;
pub mod copy_paste;
pub mod move_node;
pub mod remove_node;
pub mod update_node;

/// Validates and deduplicates function references.
/// Removes invalid references (non-existent nodes or nodes that can't be referenced).
/// Returns true if any changes were made.
pub fn validate_and_deduplicate_fn_refs(fn_refs: &mut FnRefs, board: &Board) -> bool {
    // Early exit if node can't reference functions
    if !fn_refs.can_reference_fns {
        let had_refs = !fn_refs.fn_refs.is_empty();
        fn_refs.fn_refs.clear();
        return had_refs;
    }

    let original_len = fn_refs.fn_refs.len();
    let mut seen = std::collections::HashSet::with_capacity(fn_refs.fn_refs.len());

    fn_refs.fn_refs.retain(|ref_id| {
        // Check if we've seen this ID before (deduplication)
        if !seen.insert(ref_id.clone()) {
            return false;
        }

        // Validate reference exists and can be referenced
        board
            .nodes
            .get(ref_id)
            .and_then(|node| node.fn_refs.as_ref())
            .map(|refs| refs.can_be_referenced_by_fns)
            .unwrap_or(false)
    });

    fn_refs.fn_refs.len() != original_len
}
