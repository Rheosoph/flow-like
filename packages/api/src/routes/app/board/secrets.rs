//! Centralized board-secret stripping for API responses.
//!
//! `Board.variables: HashMap<String, Variable>` and every
//! `Board.layers[].variables: HashMap<String, Variable>` can carry
//! `Variable { secret: true, default_value: Some(bytes) }`. We must
//! never return those byte values to a client over the API — the
//! audit pipeline + role-permission checks already gate *who* can
//! load a board, but the secret values themselves are not part of
//! anything except server-side execution.
//!
//! Three return sites used to inline this manually and miss layer
//! variables; this helper is the single source of truth so adding a
//! new return path doesn't re-introduce the bug.

use flow_like::flow::board::Board;

/// Walk a `Board` in place and clear `default_value` on every
/// `Variable` whose `secret` flag is set — at both board level and
/// inside every layer. Idempotent.
///
/// This is the API-response counterpart of `strip_board_secrets` in
/// `utils/fork/mod.rs` (which does the same on a `proto::Board`
/// during the fork pipeline).
pub fn filter_board_secrets(board: &mut Board) {
    for var in board.variables.values_mut() {
        if var.secret {
            var.default_value = None;
        }
    }
    for layer in board.layers.values_mut() {
        for var in layer.variables.values_mut() {
            if var.secret {
                var.default_value = None;
            }
        }
    }
}
