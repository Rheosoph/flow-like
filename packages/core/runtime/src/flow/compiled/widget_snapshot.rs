use super::{CODEC_ZSTD, CompiledBoard, HEADER_LEN, PAYLOAD_LEN_PREFIX, peek_header};
use crate::a2ui::widget::Widget;
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const WIDGET_SNAPSHOT_FORMAT_VERSION: u16 = 1;
pub const MAX_WIDGET_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
const SKIPPABLE_MAGIC: u32 = 0x184D_2A50;
const SNAPSHOT_MAGIC: &[u8] = b"flow-like/widget-snapshot\0";

/// Definitions selected from the app's current widgets when an artifact is dispatched.
/// Their storage revisions are part of the derived artifact's identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactWidgetSnapshot {
    pub format_version: u16,
    pub app_id: String,
    pub board_id: String,
    pub board_identity: String,
    pub revision: String,
    pub widgets: Vec<ArtifactWidgetEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactWidgetEntry {
    pub selector: String,
    pub widget_id: String,
    pub source_etag: String,
    pub widget: Widget,
}

impl ArtifactWidgetSnapshot {
    fn validate(&self) -> Result<()> {
        if self.app_id.is_empty()
            || self.board_id.is_empty()
            || self.board_identity.is_empty()
            || self.revision.is_empty()
        {
            return Err(anyhow!(
                "widget snapshot has no complete execution identity"
            ));
        }
        let mut selectors = BTreeSet::new();
        for entry in &self.widgets {
            if entry.selector.is_empty()
                || entry.selector.starts_with("pkg:")
                || entry.widget_id.is_empty()
                || entry.widget_id != entry.widget.id
                || entry.source_etag.is_empty()
                || !selectors.insert(&entry.selector)
            {
                return Err(anyhow!(
                    "widget snapshot contains an invalid selector entry"
                ));
            }
        }
        Ok(())
    }
}

/// Only literal declarative selectors can be resolved before execution.
pub fn static_widget_selectors(board: &CompiledBoard) -> Vec<String> {
    board
        .nodes
        .iter()
        .filter(|node| node.name == "a2ui_instantiate_widget")
        .filter_map(|node| {
            let pin = node
                .pins
                .iter()
                .filter_map(|index| board.pins.get(*index as usize))
                .find(|pin| pin.name == "widget_selector")?;
            if !pin.depends_on.is_empty() {
                return None;
            }
            let selector: String = serde_json::from_slice(pin.default_value.as_ref()?).ok()?;
            (!selector.is_empty() && !selector.starts_with("pkg:")).then_some(selector)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Append a Zstd skippable frame. Existing v4 readers still decompress the same
/// rkyv board bytes; the optional JSON extension does not change that layout.
pub fn append_widget_snapshot(bytes: &[u8], snapshot: &ArtifactWidgetSnapshot) -> Result<Vec<u8>> {
    if peek_header(bytes)?.codec != CODEC_ZSTD {
        return Err(anyhow!("widget snapshots require a Zstd board artifact"));
    }
    if snapshot.format_version != WIDGET_SNAPSHOT_FORMAT_VERSION {
        return Err(anyhow!("unsupported widget snapshot format"));
    }
    snapshot.validate()?;
    let value = canonical_value(serde_json::to_value(snapshot)?);
    let json = serde_json::to_vec(&value)?;
    let length = SNAPSHOT_MAGIC.len() + json.len();
    if length > MAX_WIDGET_SNAPSHOT_BYTES {
        return Err(anyhow!("widget snapshot exceeds its byte limit"));
    }
    let mut output = Vec::with_capacity(bytes.len() + 8 + length);
    output.extend_from_slice(bytes);
    output.extend_from_slice(&SKIPPABLE_MAGIC.to_le_bytes());
    output.extend_from_slice(&(length as u32).to_le_bytes());
    output.extend_from_slice(SNAPSHOT_MAGIC);
    output.extend_from_slice(&json);
    Ok(output)
}

fn canonical_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical_value(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_value).collect())
        }
        value => value,
    }
}

/// Older artifacts and unknown extension versions keep the live widget lookup.
pub fn decode_widget_snapshot(bytes: &[u8]) -> Result<Option<ArtifactWidgetSnapshot>> {
    if peek_header(bytes)?.codec != CODEC_ZSTD {
        return Ok(None);
    }
    let payload = bytes
        .get(HEADER_LEN + PAYLOAD_LEN_PREFIX..)
        .ok_or_else(|| anyhow!("compiled board artifact payload truncated"))?;
    let first_frame = zstd::zstd_safe::find_frame_compressed_size(payload)
        .map_err(|_| anyhow!("compiled board Zstd frame is invalid"))?;
    let mut remaining = &payload[first_frame..];
    let mut snapshot = None;
    while !remaining.is_empty() {
        if remaining.len() < 8 {
            return Err(anyhow!("compiled board extension header truncated"));
        }
        let magic = u32::from_le_bytes(remaining[..4].try_into()?);
        if magic & 0xFFFF_FFF0 != SKIPPABLE_MAGIC {
            return Err(anyhow!("compiled board has an unsupported trailing frame"));
        }
        let length = u32::from_le_bytes(remaining[4..8].try_into()?) as usize;
        let end = 8_usize
            .checked_add(length)
            .ok_or_else(|| anyhow!("compiled board extension length overflowed"))?;
        let frame = remaining
            .get(8..end)
            .ok_or_else(|| anyhow!("compiled board extension payload truncated"))?;
        remaining = &remaining[end..];
        if let Some(json) = frame.strip_prefix(SNAPSHOT_MAGIC) {
            if length > MAX_WIDGET_SNAPSHOT_BYTES || snapshot.is_some() {
                return Err(anyhow!(
                    "compiled board widget extension exceeds its bounds"
                ));
            }
            let decoded: ArtifactWidgetSnapshot = serde_json::from_slice(json)?;
            if decoded.format_version != WIDGET_SNAPSHOT_FORMAT_VERSION {
                return Ok(None);
            }
            decoded.validate()?;
            snapshot = Some(decoded);
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{
        board::{Board, Layer, LayerType},
        node::Node,
        variable::VariableType,
    };
    use crate::state::FlowNodeRegistryInner;
    use flow_like_storage::Path;

    fn artifact() -> Vec<u8> {
        let board = Board::new_detached(Some("board-1".into()), Path::default());
        let compiled = super::super::compile_board(&board).unwrap();
        super::super::encode_artifact(&compiled, &[7; 32]).unwrap()
    }

    fn snapshot() -> ArtifactWidgetSnapshot {
        ArtifactWidgetSnapshot {
            format_version: WIDGET_SNAPSHOT_FORMAT_VERSION,
            app_id: "app-1".into(),
            board_id: "board-1".into(),
            board_identity: "latest@board-etag".into(),
            revision: "widgets-revision".into(),
            widgets: vec![ArtifactWidgetEntry {
                selector: "History".into(),
                widget_id: "widget-1".into(),
                source_etag: "widget-etag".into(),
                widget: Widget::new("widget-1", "History", "root"),
            }],
        }
    }

    fn extension(bytes: &mut Vec<u8>, json: &[u8]) {
        bytes.extend_from_slice(&SKIPPABLE_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&((SNAPSHOT_MAGIC.len() + json.len()) as u32).to_le_bytes());
        bytes.extend_from_slice(SNAPSHOT_MAGIC);
        bytes.extend_from_slice(json);
    }

    #[test]
    fn old_artifacts_and_old_decoders_keep_the_same_board() {
        let original = artifact();
        assert!(decode_widget_snapshot(&original).unwrap().is_none());
        let bundled = append_widget_snapshot(&original, &snapshot()).unwrap();
        // decode_artifact is deliberately unchanged: it is also the old reader.
        assert_eq!(
            super::super::decode_artifact(&original, Some(&[7; 32])).unwrap(),
            super::super::decode_artifact(&bundled, Some(&[7; 32])).unwrap()
        );
        assert_eq!(
            decode_widget_snapshot(&bundled).unwrap().unwrap().widgets[0].selector,
            "History"
        );
    }

    #[test]
    fn snapshot_encoding_is_deterministic_and_rejects_invalid_entries() {
        let original = artifact();
        let mut snapshot = snapshot();
        assert_eq!(
            append_widget_snapshot(&original, &snapshot).unwrap(),
            append_widget_snapshot(&original, &snapshot).unwrap()
        );
        snapshot.widgets[0].widget_id = "different-widget".into();
        assert!(append_widget_snapshot(&original, &snapshot).is_err());
    }

    #[test]
    fn legacy_lz4_artifacts_keep_live_widget_lookup() {
        let original = artifact();
        let length = u32::from_le_bytes(
            original[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN_PREFIX]
                .try_into()
                .unwrap(),
        ) as usize;
        let archive =
            zstd::bulk::decompress(&original[HEADER_LEN + PAYLOAD_LEN_PREFIX..], length).unwrap();
        let mut legacy = original[..HEADER_LEN].to_vec();
        legacy[6] = super::super::CODEC_LZ4;
        legacy.extend_from_slice(&lz4_flex::compress_prepend_size(&archive));
        assert!(decode_widget_snapshot(&legacy).unwrap().is_none());
        assert_eq!(
            super::super::decode_artifact(&legacy, Some(&[7; 32])).unwrap(),
            super::super::decode_artifact(&original, Some(&[7; 32])).unwrap()
        );
    }

    #[test]
    fn unknown_snapshot_versions_keep_live_widget_lookup() {
        let mut bytes = artifact();
        let mut future = snapshot();
        future.format_version = WIDGET_SNAPSHOT_FORMAT_VERSION + 1;
        extension(&mut bytes, &serde_json::to_vec(&future).unwrap());
        assert!(decode_widget_snapshot(&bytes).unwrap().is_none());
        let template = super::super::resolver::template_from_bytes(
            &bytes,
            &[7; 32],
            &FlowNodeRegistryInner::new(0),
            &Path::default(),
        )
        .unwrap();
        assert!(template.widget_snapshot.is_none());
    }

    #[test]
    fn malformed_optional_snapshot_does_not_prevent_template_loading() {
        let mut bytes = artifact();
        extension(&mut bytes, b"{invalid json");
        assert!(decode_widget_snapshot(&bytes).is_err());
        let template = super::super::resolver::template_from_bytes(
            &bytes,
            &[7; 32],
            &FlowNodeRegistryInner::new(0),
            &Path::default(),
        )
        .unwrap();
        assert_eq!(template.board.id, "board-1");
        assert!(template.widget_snapshot.is_none());
    }

    #[test]
    fn static_selectors_include_function_bodies_and_exclude_wired_and_package_pins() {
        fn widget_node(selector: &str) -> Node {
            let mut node = Node::new("a2ui_instantiate_widget", "Widget", "", "test");
            node.add_input_pin("widget_selector", "Widget", "", VariableType::String)
                .set_default_value(Some(serde_json::json!(selector)));
            node
        }
        let mut board = Board::new_detached(Some("board-1".into()), Path::default());
        for selector in ["Report", "pkg:package/widget", ""] {
            let node = widget_node(selector);
            board.nodes.insert(node.id.clone(), node);
        }
        let mut source = Node::new("string_source", "Source", "", "test");
        let output = source.add_output_pin("value", "Value", "", VariableType::String);
        let mut dynamic = widget_node("Dynamic default");
        let input = dynamic.get_pin_mut_by_name("widget_selector").unwrap();
        input.depends_on.insert(output.id.clone());
        output.connected_to.insert(input.id.clone());
        board.nodes.insert(source.id.clone(), source);
        board.nodes.insert(dynamic.id.clone(), dynamic);
        let mut layer = Layer::new("function-1".into(), "Function".into(), LayerType::Function);
        for selector in ["Article", "Report"] {
            let node = widget_node(selector);
            layer.nodes.insert(node.id.clone(), node);
        }
        board.layers.insert(layer.id.clone(), layer);
        let compiled = super::super::compile_board(&board).unwrap();
        assert_eq!(static_widget_selectors(&compiled), ["Article", "Report"]);
    }
}
