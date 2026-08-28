//! Prerun manifest: the board-derived, user-independent facts a client needs
//! before starting a run — runtime-configured variables, OAuth requirements,
//! WASM packages and the static element demand — as a tiny binary artifact
//! stored beside the compiled board so prerun routes never load the board.
//!
//! Kept separate from [`super::CompiledBoard`] on purpose: it has its own
//! format version and lifecycle, a format bump only makes readers recompute
//! the manifest (a microsecond board walk), never the compiled board.
//!
//! Envelope (all little-endian):
//! ```text
//! [0..4)  magic "FLPM"
//! [4..6)  MANIFEST_FORMAT_VERSION u16
//! [6]     codec (1 = lz4 size-prepended block, 0 = none)
//! [7]     reserved (0)
//! [8..)   rkyv archive, encoded per codec
//! ```

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use blake3::Hasher;
use flow_like_storage::Path;
use flow_like_types::{Result, Value, anyhow};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use crate::flow::{
    board::{Board, ExecutionMode},
    node::{Node, NodePermission},
};

/// Bump on ANY change to the structs in this file or to what `from_board`
/// extracts. A mismatch makes readers recompute from the board.
pub const MANIFEST_FORMAT_VERSION: u16 = 1;

pub const MANIFEST_MAGIC: [u8; 4] = *b"FLPM";

const HEADER_LEN: usize = 8;
const CODEC_NONE: u8 = 0;
const CODEC_LZ4: u8 = 1;
/// A manifest is 1–3 KB; anything claiming more than this is corrupt.
const MAX_DECOMPRESSED_LEN: usize = 16 * 1024 * 1024;

/// A runtime-configured variable that needs a value before execution.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
)]
pub struct PrerunVariable {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub data_type: String,
    pub value_type: String,
    pub secret: bool,
    pub schema: Option<String>,
}

/// OAuth provider requirement collected from the board's nodes.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
)]
pub struct PrerunOAuthRequirement {
    pub provider_id: String,
    pub scopes: Vec<String>,
}

/// Everything prerun needs that depends only on `(app, board, version)`.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
)]
pub struct PrerunManifest {
    pub runtime_variables: Vec<PrerunVariable>,
    pub oauth_requirements: Vec<PrerunOAuthRequirement>,
    pub requires_local_execution: bool,
    /// serde string form of [`ExecutionMode`], see [`PrerunManifest::execution_mode`].
    pub execution_mode: String,
    pub has_wasm_nodes: bool,
    pub wasm_package_ids: Vec<String>,
    /// `(package_id, NodePermission serde strings)`, sorted by package id.
    pub wasm_package_permissions: Vec<(String, Vec<String>)>,
    /// Element selectors every run of this board reads (literal refs on read pins).
    pub element_selectors: Vec<String>,
    /// The board also reads elements it can only name at run time.
    pub element_reads_dynamic: bool,
    /// blake3 over every field above; clients detect drift by comparing it.
    pub signature: String,
}

impl PrerunManifest {
    pub fn from_board(board: &Board) -> Self {
        let mut runtime_variables: Vec<PrerunVariable> = board
            .variables
            .values()
            .filter(|v| v.runtime_configured)
            .map(|v| PrerunVariable {
                id: v.id.clone(),
                name: v.name.clone(),
                description: v.description.clone(),
                data_type: format!("{:?}", v.data_type),
                value_type: format!("{:?}", v.value_type),
                secret: v.secret,
                schema: v.schema.clone(),
            })
            .collect();
        runtime_variables.sort_by(|a, b| a.id.cmp(&b.id));

        let mut nodes = NodeFacts::default();
        for node in board.nodes.values() {
            nodes.visit(node);
        }
        for layer in board.layers.values() {
            for node in layer.nodes.values() {
                nodes.visit(node);
            }
        }

        let demand = crate::a2ui::element_demand(board);

        let mut manifest = Self {
            runtime_variables,
            oauth_requirements: nodes.oauth_requirements(),
            requires_local_execution: nodes.requires_local_execution,
            execution_mode: execution_mode_string(&board.execution_mode),
            has_wasm_nodes: !nodes.wasm_package_ids.is_empty(),
            wasm_package_ids: nodes.sorted_wasm_package_ids(),
            wasm_package_permissions: nodes.wasm_package_permissions(),
            element_selectors: demand.selectors,
            element_reads_dynamic: demand.dynamic,
            signature: String::new(),
        };
        manifest.signature = manifest.compute_signature();
        manifest
    }

    /// The board's execution mode; `Default` when the stored string is unknown.
    pub fn execution_mode(&self) -> ExecutionMode {
        serde_json::from_value(Value::String(self.execution_mode.clone())).unwrap_or_default()
    }

    /// WASM permissions per package; unknown permission strings are skipped.
    pub fn wasm_permissions(&self) -> HashMap<String, Vec<NodePermission>> {
        self.wasm_package_permissions
            .iter()
            .map(|(package_id, permissions)| {
                let parsed: Vec<NodePermission> = permissions
                    .iter()
                    .filter_map(|p| serde_json::from_value(Value::String(p.clone())).ok())
                    .collect();
                (package_id.clone(), parsed)
            })
            .collect()
    }

    /// Stable hash over every field, ordering collections so neither map
    /// iteration nor node walk order can shift it.
    fn compute_signature(&self) -> String {
        let mut h = Hasher::new();

        h.update(self.execution_mode.as_bytes());
        h.update(&[self.requires_local_execution as u8]);
        h.update(&[self.has_wasm_nodes as u8]);

        let mut vars: Vec<&PrerunVariable> = self.runtime_variables.iter().collect();
        vars.sort_by(|a, b| a.id.cmp(&b.id));
        for v in vars {
            h.update(b"|var|");
            h.update(v.id.as_bytes());
            h.update(b"|");
            h.update(v.name.as_bytes());
            h.update(b"|");
            h.update(v.data_type.as_bytes());
            h.update(b"|");
            h.update(v.value_type.as_bytes());
            h.update(b"|");
            h.update(&[v.secret as u8]);
            h.update(b"|");
            if let Some(d) = &v.description {
                h.update(d.as_bytes());
            }
            h.update(b"|");
            if let Some(s) = &v.schema {
                h.update(s.as_bytes());
            }
        }

        let mut providers: Vec<&PrerunOAuthRequirement> = self.oauth_requirements.iter().collect();
        providers.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
        for p in providers {
            h.update(b"|oauth|");
            h.update(p.provider_id.as_bytes());
            let mut scopes = p.scopes.clone();
            scopes.sort();
            for s in scopes {
                h.update(b"|");
                h.update(s.as_bytes());
            }
        }

        let mut ids = self.wasm_package_ids.clone();
        ids.sort();
        for id in ids {
            h.update(b"|wasm|");
            h.update(id.as_bytes());
        }

        let mut packages: Vec<&(String, Vec<String>)> =
            self.wasm_package_permissions.iter().collect();
        packages.sort_by(|a, b| a.0.cmp(&b.0));
        for (package_id, permissions) in packages {
            h.update(b"|wp|");
            h.update(package_id.as_bytes());
            let mut permissions = permissions.clone();
            permissions.sort();
            for p in permissions {
                h.update(b"|");
                h.update(p.as_bytes());
            }
        }

        h.finalize().to_hex().to_string()
    }
}

/// Per-node facts accumulated over the board walk. Maps are ordered so the
/// manifest is byte-identical for identical boards.
#[derive(Default)]
struct NodeFacts {
    oauth_scopes: BTreeMap<String, Vec<String>>,
    requires_local_execution: bool,
    wasm_package_ids: Vec<String>,
    wasm_permissions: BTreeMap<String, Vec<NodePermission>>,
}

impl NodeFacts {
    fn visit(&mut self, node: &Node) {
        if let Some(wasm) = &node.wasm {
            if !self.wasm_package_ids.contains(&wasm.package_id) {
                self.wasm_package_ids.push(wasm.package_id.clone());
            }
            if !wasm.permissions.is_empty() {
                let entry = self
                    .wasm_permissions
                    .entry(wasm.package_id.clone())
                    .or_default();
                for perm in &wasm.permissions {
                    if !entry.contains(perm) {
                        entry.push(*perm);
                    }
                }
            }
        }
        if node.only_offline {
            self.requires_local_execution = true;
        }
        if let Some(providers) = &node.oauth_providers {
            for provider_id in providers {
                self.oauth_scopes.entry(provider_id.clone()).or_default();
            }
        }
        // required_oauth_scopes only contributes scopes for providers already
        // registered via oauth_providers — it's informational, not a trigger.
        if let Some(required_scopes) = &node.required_oauth_scopes {
            for (provider_id, scopes) in required_scopes {
                if let Some(entry) = self.oauth_scopes.get_mut(provider_id) {
                    for scope in scopes {
                        if !entry.contains(scope) {
                            entry.push(scope.clone());
                        }
                    }
                }
            }
        }
    }

    fn oauth_requirements(&self) -> Vec<PrerunOAuthRequirement> {
        self.oauth_scopes
            .iter()
            .map(|(provider_id, scopes)| {
                let mut scopes = scopes.clone();
                scopes.sort();
                PrerunOAuthRequirement {
                    provider_id: provider_id.clone(),
                    scopes,
                }
            })
            .collect()
    }

    fn sorted_wasm_package_ids(&self) -> Vec<String> {
        let mut ids = self.wasm_package_ids.clone();
        ids.sort();
        ids
    }

    fn wasm_package_permissions(&self) -> Vec<(String, Vec<String>)> {
        self.wasm_permissions
            .iter()
            .map(|(package_id, permissions)| {
                let strings = permissions.iter().map(permission_string).collect();
                (package_id.clone(), strings)
            })
            .collect()
    }
}

fn execution_mode_string(mode: &ExecutionMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{mode:?}"))
}

fn permission_string(permission: &NodePermission) -> String {
    serde_json::to_value(permission)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{permission:?}"))
}

/// Prerun manifest of an immutable board version, beside its compiled artifact.
pub fn manifest_path(board_dir: &Path, board_id: &str, version: (u32, u32, u32)) -> Path {
    super::version_artifact_dir(board_dir, board_id)
        .child(format!("{}_{}_{}.prerun", version.0, version.1, version.2))
}

/// Prerun manifest of a floating draft, beside its compiled artifact and keyed
/// by the same `.board` etag.
pub fn draft_manifest_path(app_id: &str, board_id: &str, e_tag: &str) -> Path {
    super::draft_artifact_dir(app_id, board_id)
        .child(format!("{}.prerun", super::draft_artifact_stem(e_tag)))
}

pub fn encode_manifest(manifest: &PrerunManifest) -> Result<Vec<u8>> {
    let archive = rkyv::to_bytes::<rkyv::rancor::Error>(manifest)
        .map_err(|e| anyhow!("failed to serialize prerun manifest: {e}"))?;
    let compressed = lz4_flex::compress_prepend_size(&archive);

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&MANIFEST_MAGIC);
    out.extend_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
    out.push(CODEC_LZ4);
    out.push(0);
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Fails on wrong magic, a format version this build does not understand, an
/// unknown codec or a truncated payload — callers recompute from the board.
pub fn decode_manifest(bytes: &[u8]) -> Result<PrerunManifest> {
    if bytes.len() < HEADER_LEN {
        return Err(anyhow!(
            "prerun manifest too short: {} bytes, header needs {HEADER_LEN}",
            bytes.len()
        ));
    }
    if bytes[0..4] != MANIFEST_MAGIC {
        return Err(anyhow!("prerun manifest has wrong magic"));
    }
    let format_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if format_version != MANIFEST_FORMAT_VERSION {
        return Err(anyhow!(
            "prerun manifest format v{format_version}, this build reads v{MANIFEST_FORMAT_VERSION}"
        ));
    }

    let payload = &bytes[HEADER_LEN..];
    let raw: Cow<[u8]> = match bytes[6] {
        CODEC_NONE => Cow::Borrowed(payload),
        CODEC_LZ4 => {
            if payload.len() < 4 {
                return Err(anyhow!("prerun manifest payload truncated"));
            }
            let len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
            if len > MAX_DECOMPRESSED_LEN {
                return Err(anyhow!(
                    "prerun manifest claims an implausible {len}-byte payload"
                ));
            }
            Cow::Owned(
                lz4_flex::decompress_size_prepended(payload)
                    .map_err(|e| anyhow!("failed to decompress prerun manifest: {e}"))?,
            )
        }
        codec => return Err(anyhow!("prerun manifest uses unknown codec {codec}")),
    };

    let mut aligned = rkyv::util::AlignedVec::<16>::new();
    aligned.extend_from_slice(&raw);
    let archived = rkyv::access::<rkyv::Archived<PrerunManifest>, rkyv::rancor::Error>(&aligned)
        .map_err(|e| anyhow!("prerun manifest failed validation: {e}"))?;
    rkyv::deserialize::<PrerunManifest, rkyv::rancor::Error>(archived)
        .map_err(|e| anyhow!("failed to deserialize prerun manifest: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{
        board::{Layer, LayerType},
        node::NodeWasm,
        pin::ValueType,
        variable::{Variable, VariableType},
    };
    use flow_like_types::json::json;

    fn sample_manifest() -> PrerunManifest {
        let mut manifest = PrerunManifest {
            runtime_variables: vec![PrerunVariable {
                id: "var-1".into(),
                name: "api_key".into(),
                description: Some("Key".into()),
                data_type: "String".into(),
                value_type: "Normal".into(),
                secret: true,
                schema: None,
            }],
            oauth_requirements: vec![PrerunOAuthRequirement {
                provider_id: "jira".into(),
                scopes: vec!["read:issue".into()],
            }],
            requires_local_execution: true,
            execution_mode: "Local".into(),
            has_wasm_nodes: true,
            wasm_package_ids: vec!["pkg-a".into()],
            wasm_package_permissions: vec![(
                "pkg-a".into(),
                vec!["network:http".into(), "storage:read".into()],
            )],
            element_selectors: vec!["page-1/btn-1".into(), "type:switch".into()],
            element_reads_dynamic: false,
            signature: String::new(),
        };
        manifest.signature = manifest.compute_signature();
        manifest
    }

    fn sample_board(element_id: &str) -> Board {
        let mut board = Board::new_detached(Some("prerun-board".into()), Path::default());
        board.execution_mode = ExecutionMode::Local;

        let mut key = Variable::new("api_key", VariableType::String, ValueType::Normal);
        key.id = "var-1".into();
        key.runtime_configured = true;
        key.secret = true;
        key.description = Some("Key".into());
        board.variables.insert(key.id.clone(), key);

        let mut plain = Variable::new("plain", VariableType::Integer, ValueType::Array);
        plain.id = "var-0".into();
        board.variables.insert(plain.id.clone(), plain);

        let mut offline = Node::new("rpa_click", "Click", "", "RPA");
        offline.id = "offline".into();
        offline.only_offline = true;
        board.nodes.insert(offline.id.clone(), offline);

        let mut oauth = Node::new("jira_issue", "Jira Issue", "", "Jira");
        oauth.id = "oauth".into();
        oauth.oauth_providers = Some(vec!["jira".into()]);
        oauth.required_oauth_scopes = Some(HashMap::from([
            (
                "jira".to_string(),
                vec!["write:issue".to_string(), "read:issue".to_string()],
            ),
            ("github".to_string(), vec!["repo".to_string()]),
        ]));
        board.nodes.insert(oauth.id.clone(), oauth);

        let mut wasm = Node::new("pkg_node", "Package Node", "", "Packages");
        wasm.id = "wasm".into();
        wasm.wasm = Some(NodeWasm {
            package_id: "pkg-a".into(),
            permissions: vec![
                NodePermission::NetworkHttp,
                NodePermission::StorageRead,
                NodePermission::NetworkHttp,
            ],
        });
        let mut layer = Layer::new("layer-1".into(), "Layer".into(), LayerType::Function);
        layer.nodes.insert(wasm.id.clone(), wasm);
        board.layers.insert(layer.id.clone(), layer);

        let mut reader = Node::new(
            "a2ui_get_button_label",
            "Get Button Label",
            "",
            "UI/Elements/Button",
        );
        reader.id = "reader".into();
        reader
            .add_input_pin("element_ref", "Button", "", VariableType::Struct)
            .set_default_value(Some(json!(element_id)));
        board.nodes.insert(reader.id.clone(), reader);

        board
    }

    #[test]
    fn round_trip_preserves_manifest() {
        let manifest = sample_manifest();
        let bytes = encode_manifest(&manifest).unwrap();
        assert_eq!(&bytes[0..4], &MANIFEST_MAGIC);
        assert_eq!(
            u16::from_le_bytes([bytes[4], bytes[5]]),
            MANIFEST_FORMAT_VERSION
        );
        assert_eq!(bytes[6], CODEC_LZ4);
        assert_eq!(decode_manifest(&bytes).unwrap(), manifest);
    }

    #[test]
    fn decodes_uncompressed_payload() {
        let manifest = sample_manifest();
        let archive = rkyv::to_bytes::<rkyv::rancor::Error>(&manifest).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MANIFEST_MAGIC);
        bytes.extend_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
        bytes.push(CODEC_NONE);
        bytes.push(0);
        bytes.extend_from_slice(&archive);
        assert_eq!(decode_manifest(&bytes).unwrap(), manifest);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[0..4].copy_from_slice(b"FLCB");
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("wrong magic"), "{err}");
    }

    #[test]
    fn rejects_other_format_version() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[4..6].copy_from_slice(&(MANIFEST_FORMAT_VERSION + 1).to_le_bytes());
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(
            err.contains(&format!("format v{}", MANIFEST_FORMAT_VERSION + 1)),
            "{err}"
        );
    }

    #[test]
    fn rejects_unknown_codec() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[6] = 7;
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("unknown codec 7"), "{err}");
    }

    #[test]
    fn rejects_truncated_input() {
        let bytes = encode_manifest(&sample_manifest()).unwrap();

        let err = decode_manifest(&bytes[..5]).unwrap_err().to_string();
        assert!(err.contains("too short"), "{err}");

        let err = decode_manifest(&bytes[..HEADER_LEN + 2])
            .unwrap_err()
            .to_string();
        assert!(err.contains("truncated"), "{err}");

        let err = decode_manifest(&bytes[..bytes.len() - 5])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("decompress") || err.contains("validation"),
            "{err}"
        );
    }

    #[test]
    fn rejects_implausible_length_prefix() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("implausible"), "{err}");
    }

    #[test]
    fn from_board_extracts_every_field() {
        let manifest = PrerunManifest::from_board(&sample_board("page-1/btn-1"));

        assert_eq!(
            manifest.runtime_variables,
            vec![PrerunVariable {
                id: "var-1".into(),
                name: "api_key".into(),
                description: Some("Key".into()),
                data_type: "String".into(),
                value_type: "Normal".into(),
                secret: true,
                schema: None,
            }]
        );
        assert_eq!(
            manifest.oauth_requirements,
            vec![PrerunOAuthRequirement {
                provider_id: "jira".into(),
                scopes: vec!["read:issue".into(), "write:issue".into()],
            }],
            "scopes only attach to providers registered via oauth_providers"
        );
        assert!(manifest.requires_local_execution);
        assert_eq!(manifest.execution_mode, "Local");
        assert_eq!(manifest.execution_mode(), ExecutionMode::Local);
        assert!(manifest.has_wasm_nodes);
        assert_eq!(manifest.wasm_package_ids, vec!["pkg-a".to_string()]);
        assert_eq!(
            manifest.wasm_package_permissions,
            vec![(
                "pkg-a".to_string(),
                vec!["network:http".to_string(), "storage:read".to_string()]
            )]
        );
        assert_eq!(
            manifest.wasm_permissions(),
            HashMap::from([(
                "pkg-a".to_string(),
                vec![NodePermission::NetworkHttp, NodePermission::StorageRead]
            )])
        );
        assert!(
            manifest
                .element_selectors
                .iter()
                .any(|s| s.contains("page-1/btn-1")),
            "literal element_ref becomes a selector: {:?}",
            manifest.element_selectors
        );
        assert!(!manifest.element_reads_dynamic);
        assert_eq!(manifest.signature.len(), 64);
        assert_eq!(manifest.signature, manifest.compute_signature());
    }

    #[test]
    fn from_board_is_deterministic() {
        let a = PrerunManifest::from_board(&sample_board("page-1/btn-1"));
        let b = PrerunManifest::from_board(&sample_board("page-1/btn-1"));
        assert_eq!(a, b);

        let c = PrerunManifest::from_board(&sample_board("page-1/btn-2"));
        assert_ne!(a.element_selectors, c.element_selectors);
        assert_eq!(
            a.signature, c.signature,
            "element refs never move the drift signature"
        );
    }

    #[test]
    fn signature_ignores_element_fields() {
        let base = sample_manifest();

        let mut selector = base.clone();
        selector.element_selectors.push("glob:feed-row-*".into());
        assert_eq!(selector.compute_signature(), base.signature);

        let mut dynamic = base.clone();
        dynamic.element_reads_dynamic = true;
        assert_eq!(dynamic.compute_signature(), base.signature);

        let mut reordered = base.clone();
        reordered.oauth_requirements[0].scopes = vec!["read:issue".into()];
        reordered.wasm_package_permissions[0].1.reverse();
        assert_eq!(
            reordered.compute_signature(),
            base.signature,
            "permission order does not shift the hash"
        );
    }

    #[test]
    fn unknown_strings_parse_leniently() {
        let mut manifest = sample_manifest();
        manifest.execution_mode = "Sideways".into();
        manifest.wasm_package_permissions[0]
            .1
            .push("network:carrier-pigeon".into());
        assert_eq!(manifest.execution_mode(), ExecutionMode::default());
        assert_eq!(
            manifest.wasm_permissions()["pkg-a"],
            vec![NodePermission::NetworkHttp, NodePermission::StorageRead]
        );
    }

    #[test]
    fn manifest_paths_sit_beside_artifacts() {
        let board_dir = Path::from("apps/app-1/meta");
        let artifact = super::super::artifact_path(&board_dir, "board-1", (1, 2, 3)).to_string();
        let manifest = manifest_path(&board_dir, "board-1", (1, 2, 3)).to_string();
        assert_eq!(manifest, "apps/app-1/meta/compiled/board-1/1_2_3.prerun");
        assert_eq!(
            manifest.trim_end_matches(".prerun"),
            artifact.trim_end_matches(".flcb")
        );

        let draft_artifact =
            super::super::draft_artifact_path("app-1", "board-1", "etag").to_string();
        let draft_manifest = draft_manifest_path("app-1", "board-1", "etag").to_string();
        assert!(draft_manifest.ends_with(".prerun"));
        assert_eq!(
            draft_manifest.trim_end_matches(".prerun"),
            draft_artifact.trim_end_matches(".flcb")
        );
    }
}
