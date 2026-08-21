//! Scoping a `node_updates` sweep to the nodes an edit can actually reach.
//!
//! `NodeLogic::on_update` takes an **immutable** board, so within a pass only the node currently
//! being updated can change. That is what makes it sound to re-run `on_update` on the nodes an edit
//! reached instead of every node on the board — the difference between O(edit) and O(board) per
//! keystroke, which on a thousand-node board decides whether the editor keeps up.
//!
//! Correctness rests on enumerating every way an `on_update` can read state outside its own node:
//!
//! - **wired** — it reads a pin it is connected to (`variable_get`/`variable_set` resolving a
//!   companion's type, `break_struct` and `make_from_schema` resolving a struct schema, the
//!   `utils/*_ref` family). These need no registration: the sweep follows `connected_to` and
//!   `depends_on` through the board's pin index.
//! - **referenced** — it resolves a layer or node named in a *pin value* rather than reached over a
//!   wire (`control_call_function`, `control_call_reference`). The target cannot be recovered
//!   without decoding every pin on the board, so these re-evaluate whenever anything of the kind
//!   they point at moved.
//! - **variable** — it resolves a board or layer variable by identity.
//! - **whole board** — it scans every node with no particular id, so no edit can be attributed to
//!   it and it must re-evaluate on every sweep.
//!
//! The last three are declared in [`external_read`]. Two things bound the cost of a channel that
//! list misses: `Board::load` still runs the full sweep, so a stale derivation self-heals on the
//! next load rather than persisting; and `dirty_sweep_matches_full_sweep` in the board tests
//! asserts both sweeps leave identical boards.

use std::collections::{HashMap, HashSet};

use super::{Board, PinOwner};

/// What a node type's `on_update` reads besides its own node and its wired neighbours.
///
/// Membership lives in one place because each entry is a claim about an `on_update` body and costs
/// a re-evaluation whenever its kind is touched. Wired reads are deliberately absent — the sweep
/// follows those structurally, so declaring them would only slow it down.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ExternalRead {
    /// Resolves another node named in a pin value.
    Node,
    /// Resolves a board or layer variable.
    Variable,
    /// Reads state no edit can be attributed to, so it re-derives on every sweep.
    Board,
}

/// Every kind, so a sweep can seed the ones an edit implicates without hardcoding the list twice.
const EXTERNAL_READS: [ExternalRead; 3] = [
    ExternalRead::Node,
    ExternalRead::Variable,
    ExternalRead::Board,
];

/// How `node_type`'s `on_update` reaches outside its own node and wires, if it does at all.
pub fn external_read(node_type: &str) -> Option<ExternalRead> {
    Some(match node_type {
        // Mirrors the pins of the node named in its reference pin.
        "control_call_reference" => ExternalRead::Node,

        // Adopt the type and schema of the variable they name. The `*_ref` family writes only
        // `node.error`, which `Node::hash` does not cover — they can never be observed to have
        // changed, so being seeded on a variable edit is the only thing that keeps their
        // validation message current.
        "variable_get"
        | "variable_set"
        | "array_clear_ref"
        | "array_extend_ref"
        | "array_pop_ref"
        | "array_push_ref"
        | "array_remove_index_ref"
        | "array_set_index_ref"
        | "map_clear_ref"
        | "map_remove_ref"
        | "map_set_ref"
        | "set_clear_ref"
        | "set_discard_ref"
        | "set_insert_ref" => ExternalRead::Variable,

        // These do not derive from board state a wire can reach, so no edit implies them and they
        // re-run every sweep:
        // - the widget nodes resolve the app's widget list and installed packages through
        //   `board.app_state`, and deliberately keep their last-good shape while that registry is
        //   unavailable, so they must re-run once it comes back;
        // - `a2ui_get_element` reads the board's *pages* out of object storage;
        // - `events_widget_action` finds its partner by scanning for the widget node whose
        //   `fn_refs` name it, which is a reverse edge no forward channel reaches;
        // - `a2ui_widget_query` and `a2ui_widget_update_inputs` trace an unbounded upstream chain
        //   across reroutes and layer bridges to read a distant node's `widget_selector`, which
        //   one-hop propagation cannot follow;
        // - `control_call_function` mirrors the pins of the function layer it names, and `cleanup`
        //   rewrites layer pins *after* the sweep that would have read them. It is therefore always
        //   one derivation behind and is healed by the next sweep, whichever edit triggers it —
        //   attributing it to an edit would leave it stale instead.
        "a2ui_instantiate_widget"
        | "a2ui_widget_query"
        | "a2ui_widget_update_inputs"
        | "a2ui_get_element"
        | "events_widget_action"
        | "control_call_function" => ExternalRead::Board,

        _ => return None,
    })
}

/// What a command batch wrote directly, before any propagation.
///
/// This only names what the commands themselves touched; [`DirtyIndex::seed`] expands it into the
/// set that actually has to be re-evaluated.
#[derive(Default, Debug)]
pub struct Touched {
    pub nodes: HashSet<String>,
    pub variables: HashSet<String>,
    pub layers: HashSet<String>,
}

impl Touched {
    /// Whether the batch wrote anything a node could derive from.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.variables.is_empty() && self.layers.is_empty()
    }
}

/// The reverse lookups a dirty sweep needs, built once per sweep.
///
/// Both directions an edit propagates in — "who reads this kind of thing" and "who lives in this
/// layer" — are answered from here so the sweep itself never scans the board.
pub struct DirtyIndex {
    external_readers: HashMap<ExternalRead, Vec<String>>,
    layer_members: HashMap<String, Vec<String>>,
}

impl DirtyIndex {
    pub fn build(board: &Board) -> Self {
        let mut external_readers: HashMap<ExternalRead, Vec<String>> = HashMap::new();
        let mut layer_members: HashMap<String, Vec<String>> = HashMap::new();

        for (node_id, node) in &board.nodes {
            if let Some(kind) = external_read(&node.name) {
                external_readers
                    .entry(kind)
                    .or_default()
                    .push(node_id.clone());
            }
            if let Some(layer_id) = node.layer.as_deref() {
                layer_members
                    .entry(layer_id.to_string())
                    .or_default()
                    .push(node_id.clone());
            }
        }

        DirtyIndex {
            external_readers,
            layer_members,
        }
    }

    fn readers_of(&self, kind: ExternalRead) -> &[String] {
        self.external_readers
            .get(&kind)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The nodes filed under a layer. An edit to a layer reaches its body, which is wired to the
    /// layer's own pins.
    pub fn members_of(&self, layer_id: &str) -> &[String] {
        self.layer_members
            .get(layer_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The nodes sitting at the other end of `node_id`'s wires.
    ///
    /// A wire into a layer's *own* pin crosses a function boundary, and the layer's body reads that
    /// pin, so the body re-derives with it.
    pub fn wired_neighbours(&self, board: &Board, node_id: &str, out: &mut HashSet<String>) {
        let Some(node) = board.nodes.get(node_id) else {
            return;
        };
        for pin in node.pins.values() {
            for counterpart in pin.connected_to.iter().chain(pin.depends_on.iter()) {
                match board.pin_owner(counterpart) {
                    Some(PinOwner::Node(owner)) => {
                        if owner != node_id {
                            out.insert(owner.clone());
                        }
                    }
                    Some(PinOwner::LayerPin(layer_id)) => {
                        out.extend(self.members_of(layer_id).iter().cloned());
                    }
                    Some(PinOwner::LayerNode { .. }) | None => {}
                }
            }
        }
    }

    /// The nodes a batch's direct writes force us to re-evaluate.
    ///
    /// Includes the wired neighbours of everything the batch wrote: a command can retype a pin
    /// without any `on_update` running, so the neighbour that reads that pin would otherwise never
    /// be told. Changes made *by* `on_update` propagate later, as the sweep observes them.
    pub fn seed(&self, board: &Board, touched: &Touched) -> HashSet<String> {
        let mut seed: HashSet<String> = HashSet::new();

        for node_id in &touched.nodes {
            if board.nodes.contains_key(node_id) {
                seed.insert(node_id.clone());
            }
            // A removed node is already gone from the board, but its former neighbours still
            // reference it; the command carries them, so they are seeded by the loop above.
            self.wired_neighbours(board, node_id, &mut seed);
        }

        for layer_id in &touched.layers {
            seed.extend(self.members_of(layer_id).iter().cloned());
        }

        // These read through an id held in a pin value or by scanning, neither of which can be
        // reversed, so anything of the kind that moved re-evaluates all of its readers.
        for kind in EXTERNAL_READS {
            let implicated = match kind {
                ExternalRead::Node => !touched.nodes.is_empty(),
                ExternalRead::Variable => !touched.variables.is_empty(),
                ExternalRead::Board => true,
            };
            if implicated {
                seed.extend(self.readers_of(kind).iter().cloned());
            }
        }

        seed
    }
}
