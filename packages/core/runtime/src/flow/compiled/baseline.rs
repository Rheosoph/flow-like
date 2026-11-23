//! Facts about the source board that its compiled artifact cannot reproduce.
//!
//! A Page contract binds to the source board's content hash and prerun signature, and both
//! cover editor fields the compiled board drops. They are recorded when the artifact is
//! compiled, so a Page starts from the artifact instead of loading the source board and
//! re-deriving every node. The node types the board places are recorded with their
//! fingerprint, so an artifact stays usable when the catalog changes in types the board
//! does not use.
//!
//! The facts travel as an extension frame behind the board archive, like the widget snapshot.

use super::prerun::{
    PrerunManifest, PrerunPageExecution, hash_page_execution_revision, validate_page_execution,
};
use super::{CODEC_ZSTD, append_extension_frame, extension_frames, peek_header};
use crate::flow::board::Board;
use crate::state::FlowNodeRegistryInner;
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const BASELINE_FACTS_FORMAT_VERSION: u16 = 1;
const MAX_BASELINE_FACTS_BYTES: usize = 4 * 1024 * 1024;
const FACTS_MAGIC: &[u8] = b"flow-like/baseline-facts\0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineFacts {
    pub format_version: u16,
    /// [`Board::content_hash`] of the source board.
    pub content_hash: u64,
    /// The source board's own prerun manifest, without Page bindings.
    pub prerun: PrerunManifest,
    /// Every node type the source board places, sorted.
    pub node_types: Vec<String>,
    /// [`FlowNodeRegistryInner::fingerprint_of`] those types in the compiling registry, hex.
    pub node_fingerprint: String,
}

impl BaselineFacts {
    /// `board` must be the prepared source the artifact is compiled from, and `registry` the
    /// one that prepared it.
    pub fn from_source(board: &Board, registry: &FlowNodeRegistryInner) -> Self {
        let node_types: BTreeSet<&str> = board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
            .map(|node| node.name.as_str())
            .collect();
        let node_fingerprint = node_fingerprint(registry, node_types.iter().copied());
        Self {
            format_version: BASELINE_FACTS_FORMAT_VERSION,
            content_hash: board.content_hash(),
            prerun: PrerunManifest::from_board(board),
            node_types: node_types.into_iter().map(str::to_string).collect(),
            node_fingerprint,
        }
    }

    /// Whether `registry` registers every node type of the source board exactly as the
    /// compiling registry did. The compiled board then holds what compiling against
    /// `registry` would produce.
    pub fn holds_for(&self, registry: &FlowNodeRegistryInner) -> bool {
        node_fingerprint(registry, self.node_types.iter().map(String::as_str))
            == self.node_fingerprint
    }

    /// [`super::prerun::page_execution_revision`] of the source board, derived from its
    /// compiled `view`. Extracting and validating a Page execution only reads node ids,
    /// entry flags and page ids, all of which the view keeps.
    pub fn page_execution_revision(
        &self,
        view: &Board,
        execution: &PrerunPageExecution,
    ) -> Result<String> {
        validate_page_execution(view, execution)?;
        Ok(hash_page_execution_revision(
            view,
            execution,
            &self.prerun.signature,
            self.content_hash,
        ))
    }
}

fn node_fingerprint<'a>(
    registry: &FlowNodeRegistryInner,
    node_types: impl IntoIterator<Item = &'a str>,
) -> String {
    blake3::Hash::from_bytes(registry.fingerprint_of(node_types))
        .to_hex()
        .to_string()
}

/// Record `facts` behind a board artifact.
pub fn append_baseline_facts(bytes: &[u8], facts: &BaselineFacts) -> Result<Vec<u8>> {
    if peek_header(bytes)?.codec != CODEC_ZSTD {
        return Err(anyhow!("baseline facts require a Zstd board artifact"));
    }
    let json = serde_json::to_vec(facts)?;
    if FACTS_MAGIC.len() + json.len() > MAX_BASELINE_FACTS_BYTES {
        return Err(anyhow!("baseline facts exceed their byte limit"));
    }
    Ok(append_extension_frame(bytes, FACTS_MAGIC, &json))
}

/// The facts recorded behind a board artifact. Artifacts written before the frame existed,
/// and frames of an unknown version, carry none.
pub fn decode_baseline_facts(bytes: &[u8]) -> Result<Option<BaselineFacts>> {
    let Some(frames) = extension_frames(bytes)? else {
        return Ok(None);
    };
    let mut facts = None;
    for frame in frames {
        let frame = frame?;
        let Some(json) = frame.strip_prefix(FACTS_MAGIC) else {
            continue;
        };
        if frame.len() > MAX_BASELINE_FACTS_BYTES || facts.is_some() {
            return Err(anyhow!(
                "compiled board baseline extension exceeds its bounds"
            ));
        }
        let decoded: BaselineFacts = serde_json::from_slice(json)?;
        if decoded.format_version != BASELINE_FACTS_FORMAT_VERSION {
            return Ok(None);
        }
        facts = Some(decoded);
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::compiled::prerun::{
        PrerunPageEventTarget, PrerunPageSpecialEvents, page_execution_revision,
    };
    use crate::flow::compiled::{
        CompiledRunTemplate, compile_board, decode_artifact, encode_artifact, template_from_bytes,
    };
    use crate::flow::execution::context::ExecutionContext;
    use crate::flow::node::{Node, NodeLogic};
    use flow_like_storage::Path;
    use flow_like_types::async_trait;
    use std::sync::Arc;

    struct Typed {
        name: &'static str,
        friendly_name: &'static str,
    }

    #[async_trait]
    impl NodeLogic for Typed {
        fn get_node(&self) -> Node {
            Node::new(self.name, self.friendly_name, "", "test")
        }

        async fn run(&self, _: &mut ExecutionContext) -> Result<()> {
            Ok(())
        }
    }

    fn registry(types: &[(&'static str, &'static str)]) -> FlowNodeRegistryInner {
        let mut registry = FlowNodeRegistryInner::new(types.len());
        for &(name, friendly_name) in types {
            let logic = Typed {
                name,
                friendly_name,
            };
            registry.insert(logic.get_node(), Arc::new(logic));
        }
        registry
    }

    /// A board with one entry node and one Page. The coordinates are an editor field: the
    /// content hash covers them and the compiled board drops them.
    fn source() -> (Board, String) {
        let mut board = Board::new_detached(Some("board".into()), Path::from("apps").join("app"));
        board.page_ids.push("page".into());
        let mut entry = Node::new("used", "Used", "", "test");
        entry.set_start(true);
        entry.coordinates = Some((12.0, 34.0, 0.0));
        let entry_id = entry.id.clone();
        board.nodes.insert(entry_id.clone(), entry);
        (board, entry_id)
    }

    fn load_of(page_id: &str, node_id: &str) -> PrerunPageExecution {
        PrerunPageExecution {
            page_id: page_id.into(),
            action_events: Vec::new(),
            special_events: PrerunPageSpecialEvents {
                load: Some(PrerunPageEventTarget {
                    node_id: node_id.into(),
                }),
                ..Default::default()
            },
        }
    }

    #[test]
    fn facts_travel_behind_the_artifact_and_leave_its_board_unchanged() {
        let (board, _) = source();
        let registry = registry(&[("used", "Used")]);
        let fingerprint = registry.fingerprint();
        let compiled = compile_board(&board).unwrap();
        let plain = encode_artifact(&compiled, &fingerprint).unwrap();
        assert_eq!(decode_baseline_facts(&plain).unwrap(), None);

        let facts = BaselineFacts::from_source(&board, &registry);
        let bytes = append_baseline_facts(&plain, &facts).unwrap();
        assert_eq!(
            decode_baseline_facts(&bytes).unwrap().as_ref(),
            Some(&facts)
        );
        assert_eq!(
            decode_artifact(&bytes, Some(&fingerprint)).unwrap(),
            compiled
        );
        let template =
            template_from_bytes(&bytes, &fingerprint, &registry, &board.board_dir).unwrap();
        assert_eq!(template.baseline.as_deref(), Some(&facts));

        let mut newer = facts.clone();
        newer.format_version += 1;
        let unknown = append_baseline_facts(&plain, &newer).unwrap();
        assert_eq!(decode_baseline_facts(&unknown).unwrap(), None);
        assert!(decode_baseline_facts(&append_baseline_facts(&bytes, &facts).unwrap()).is_err());
    }

    #[test]
    fn a_page_revision_from_the_artifact_is_the_revision_of_its_source() {
        let (board, entry_id) = source();
        let registry = registry(&[("used", "Used")]);
        let facts = BaselineFacts::from_source(&board, &registry);
        let compiled = compile_board(&board).unwrap();
        let template =
            CompiledRunTemplate::from_compiled(&compiled, &registry, board.board_dir.clone())
                .unwrap();
        let view = &template.board;
        assert_ne!(
            view.content_hash(),
            facts.content_hash,
            "the view alone cannot reproduce the source hash"
        );

        let execution = load_of("page", &entry_id);
        assert_eq!(
            facts.page_execution_revision(view, &execution).unwrap(),
            page_execution_revision(&board, &execution).unwrap()
        );
        for rejected in [load_of("other-page", &entry_id), load_of("page", "missing")] {
            assert!(page_execution_revision(&board, &rejected).is_err());
            assert!(facts.page_execution_revision(view, &rejected).is_err());
        }
    }

    #[test]
    fn facts_hold_while_the_board_s_node_types_are_registered_the_same() {
        let (board, _) = source();
        let compiling = registry(&[("used", "Used")]);
        let extended = registry(&[("used", "Used"), ("unrelated", "Unrelated")]);
        let facts = BaselineFacts::from_source(&board, &compiling);
        assert_eq!(facts.node_types, ["used"]);

        assert_ne!(compiling.fingerprint(), extended.fingerprint());
        assert!(facts.holds_for(&compiling));
        assert!(facts.holds_for(&extended));
        assert!(!facts.holds_for(&registry(&[("used", "Renamed")])));
        assert!(!facts.holds_for(&registry(&[("unrelated", "Unrelated")])));
    }
}
