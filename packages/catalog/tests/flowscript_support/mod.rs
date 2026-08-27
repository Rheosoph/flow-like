//! Shared harness for the FlowScript integration tests: the real catalog, the product's
//! dynamic-pin enricher, and fixture loading.
//!
//! Both `handwritten_flowscript.rs` (text -> board) and `render_contract_catalog.rs`
//! (board -> text -> board) need the same catalog and the same enricher. Building them twice
//! meant the two suites could silently drift apart and test different engines.

#![allow(dead_code)]

use flow_like::flow::ast::{MetadataEnricher, node_names};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{NodeMetadata, node_to_metadata};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinType;
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_catalog::CatalogBuilder;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

/// Node types whose `on_update` derives pins from their own literal arguments. Mirrors
/// `ENRICH_ALLOWLIST` in `packages/core/src/flow/ast/apply.rs`: the product apply path enriches
/// through it, so a harness that skipped it would under-test every dynamic-pin node.
pub fn enrich_allowlist() -> Vec<&'static str> {
    let mut list = vec![
        "string_format",
        "string_render_template",
        "a2ui_push_csv_to_chart",
        "df_sql_query",
        "df_sql_query_cached",
        "df_execute_sql",
        "df_write_delta",
        "graph_sql_query",
        "control_switch",
        "struct_break",
        "struct_make_from_schema",
        "ml_apply_transform",
        "ml_predict",
    ];
    list.extend(ML_FIT_NODES);
    list
}

pub const ML_FIT_NODES: &[&str] = &[
    "fit_adaboost",
    "fit_dbscan",
    "fit_decision_tree",
    "fit_elastic_net",
    "fit_feature_scaler",
    "fit_gaussian_mixture",
    "fit_glm",
    "fit_kmeans",
    "fit_knn_classifier",
    "fit_knn_regressor",
    "fit_linear_regression",
    "fit_logistic_regression",
    "fit_multinomial_naive_bayes",
    "fit_naive_bayes",
    "fit_one_class_svm",
    "fit_pca",
    "fit_random_forest",
    "fit_svm_multi_class",
    "fit_svm_regression",
    "fit_tfidf_vectorizer",
    "fit_tsne",
];

/// `pin_name_matches` is crate-private, so this mirrors it: compare ignoring case and any
/// `_`/space separators, which makes `output_col` match `outputCol` and `Input Col`.
pub fn loose_pin_match(left: &str, right: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    norm(left) == norm(right)
}

/// Build the same enricher the product apply path builds: seed a scratch node with the call's
/// literal arguments, run its `on_update`, and read the pins back.
pub fn build_enricher(logic: &[Arc<dyn NodeLogic>]) -> MetadataEnricher {
    let allow = enrich_allowlist();
    let logic_by_type: HashMap<String, Arc<dyn NodeLogic>> = logic
        .iter()
        .map(|logic| (logic.get_node().name, logic.clone()))
        .filter(|(name, _)| allow.contains(&name.as_str()))
        .collect();
    Box::new(
        move |meta: &NodeMetadata, args: &[(String, flow_like_types::Value)], board: &Board| {
            let logic = logic_by_type.get(&meta.name)?;
            let mut scratch = logic.get_node();
            let mut seeded = false;
            for (arg_name, value) in args {
                let pin_id = scratch
                    .pins
                    .iter()
                    .find(|(_, pin)| {
                        pin.pin_type == PinType::Input
                            && (loose_pin_match(&pin.name, arg_name)
                                || loose_pin_match(&pin.friendly_name, arg_name))
                    })
                    .map(|(id, _)| id.clone());
                if let Some(pin_id) = pin_id
                    && let Some(pin) = scratch.pins.get_mut(&pin_id)
                    && let Ok(bytes) = flow_like_types::json::to_vec(value)
                {
                    pin.default_value = Some(bytes);
                    seeded = true;
                }
            }
            if !seeded {
                return None;
            }
            run_on_update(logic.on_update(&mut scratch, board));
            Some(node_to_metadata(&scratch))
        },
    )
}

/// `on_update` is async but the enricher is a synchronous callback, so it has to block. The
/// runtime is process-wide rather than per-enricher because a runtime dropped inside an async test
/// panics ("cannot drop a runtime in a context where blocking is not allowed").
static ON_UPDATE_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime for on_update")
});

/// Drive one `on_update` to completion from a synchronous context.
///
/// Reconcile is synchronous, so an async test calling it ends up blocking on a worker thread that
/// is driving tasks — which tokio refuses outright. `block_in_place` hands the worker's other tasks
/// off first, making the block legal; outside a runtime there is nothing to hand off.
fn run_on_update(future: impl std::future::Future<Output = ()>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| ON_UPDATE_RUNTIME.block_on(future));
    } else {
        ON_UPDATE_RUNTIME.block_on(future);
    }
}

pub struct CatalogFixture {
    pub logic: Vec<Arc<dyn NodeLogic>>,
    pub nodes: Vec<Node>,
    pub metadata: Vec<NodeMetadata>,
}

pub static CATALOG: LazyLock<CatalogFixture> = LazyLock::new(|| {
    let logic: Vec<Arc<dyn NodeLogic>> = CatalogBuilder::new().build();
    let nodes: Vec<Node> = logic.iter().map(|logic| logic.get_node()).collect();
    let metadata = nodes.iter().map(node_to_metadata).collect();
    CatalogFixture {
        logic,
        nodes,
        metadata,
    }
});

pub fn catalog() -> (Vec<NodeMetadata>, MetadataEnricher) {
    (CATALOG.metadata.clone(), build_enricher(&CATALOG.logic))
}

pub async fn catalog_state() -> Arc<FlowLikeState> {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    state
        .node_registry()
        .write()
        .await
        .push_nodes(CATALOG.logic.clone());
    state
}

pub fn board_node_and_layer_ids(board: &Board) -> HashSet<String> {
    let mut ids = board.nodes.keys().cloned().collect::<HashSet<_>>();
    for layer in board.layers.values() {
        ids.insert(layer.id.clone());
        ids.extend(layer.nodes.keys().cloned());
    }
    ids
}

/// Every node on the board, whichever map it is filed under.
pub fn all_nodes(board: &Board) -> Vec<&Node> {
    let mut nodes: Vec<&Node> = board.nodes.values().collect();
    for layer in board.layers.values() {
        nodes.extend(layer.nodes.values());
    }
    nodes
}

/// `<repo>/tests/ast`, the shared FlowScript fixture tree.
pub fn ast_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/ast")
}

pub fn handwritten_fixture_dir() -> PathBuf {
    ast_fixture_dir().join("handwritten")
}

/// Collect every file with `extension` under `dir`, descending into subdirectories.
pub fn collect_files(dir: &PathBuf, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, extension, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

/// Decode one committed `.board` fixture and give its nodes the FlowScript names a loaded board
/// gets from the catalog.
///
/// The fixtures predate explicit FlowScript names on placed nodes. In the product
/// `sync_board_node_schemas` stamps them on every load; here the real catalog stands in for it, so
/// a fixture lowers exactly as the same board would in the app.
pub async fn load_board_fixture(path: &std::path::Path) -> Board {
    use flow_like_types::FromProto;

    let dir = ast_fixture_dir()
        .canonicalize()
        .expect("tests/ast directory should exist");
    let relative = path
        .canonicalize()
        .expect("fixture path")
        .strip_prefix(&dir)
        .expect("fixture lives under tests/ast")
        .to_string_lossy()
        .to_string();
    let store: Arc<dyn flow_like_storage::object_store::ObjectStore> = Arc::new(
        flow_like_storage::object_store::local::LocalFileSystem::new_with_prefix(&dir)
            .expect("local object store over tests/ast"),
    );
    let proto: flow_like_types::proto::Board = flow_like::utils::compression::from_compressed(
        store,
        flow_like_storage::Path::from(relative.clone()),
    )
    .await
    .unwrap_or_else(|error| panic!("decode {relative}: {error}"));

    let names: HashMap<&str, flow_like_ast::NodeNames> = CATALOG
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node_names(node)))
        .collect();
    let stamp = |node: &mut Node| {
        if let Some(names) = names.get(node.name.as_str()) {
            node.set_flowscript_name(&names.namespace, &names.alias);
            node.set_receiver(names.receiver.as_deref().unwrap_or(""));
            node.category.clone_from(&names.category);
        }
    };
    let mut board = Board::from_proto(proto);
    board.nodes.values_mut().for_each(stamp);
    for layer in board.layers.values_mut() {
        layer.nodes.values_mut().for_each(stamp);
    }
    board
}
