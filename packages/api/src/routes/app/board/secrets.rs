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

use flow_like::flow::{board::Board, pin::Pin};

/// Walk a `Board` in place and clear `default_value` on every
/// `Variable` whose `secret` flag is set and on every `Pin` whose
/// options mark it `sensitive` — at both board level and inside every
/// layer. Idempotent.
///
/// The write side honours this: `UpdateNode`, `UpsertPin` and
/// `UpsertVariable` treat an incoming `None` on a sensitive/secret field
/// as "unchanged", so a client that round-trips a filtered board cannot
/// erase what it never saw.
///
/// This is the API-response counterpart of `strip_board_secrets` in
/// `utils/fork/mod.rs` (which does the same on a `proto::Board`
/// during the fork pipeline).
pub fn filter_board_secrets(board: &mut Board) {
    fn strip_pins(pins: &mut std::collections::HashMap<String, Pin>) {
        for pin in pins.values_mut() {
            if pin.is_sensitive() {
                pin.default_value = None;
            }
        }
    }
    for var in board.variables.values_mut() {
        if var.secret {
            var.default_value = None;
        }
    }
    for node in board.nodes.values_mut() {
        strip_pins(&mut node.pins);
    }
    for layer in board.layers.values_mut() {
        for var in layer.variables.values_mut() {
            if var.secret {
                var.default_value = None;
            }
        }
        strip_pins(&mut layer.pins);
        for node in layer.nodes.values_mut() {
            strip_pins(&mut node.pins);
        }
    }
}
