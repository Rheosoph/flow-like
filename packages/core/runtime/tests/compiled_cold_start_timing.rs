//! Where a cold start from the compiled baseline spends its time, on a real board.
//!
//! Ignored by default because it reads a board from a local object-store root. Run with:
//!   FLOW_LIKE_BENCH_ROOT="$HOME/Library/Application Support/flow-like/projects" \
//!   FLOW_LIKE_BENCH_APP=<app id> FLOW_LIKE_BENCH_BOARD=<board id> \
//!   cargo test -p flow-like-runtime --profile release-fast \
//!     --test compiled_cold_start_timing -- --ignored --nocapture
//!
//! Node types are registered as stubs and the board is taken as stored, without `node_updates`.
//! The numbers therefore describe the artifact path and the board-derived page checks, not the
//! compile-from-source path a run pays once after an edit.

use flow_like_runtime::{
    flow::{
        board::Board,
        compiled::{
            BaselineFacts, CompiledBoard, CompiledRunTemplate, PrerunManifest, codes,
            compile_board, decode_artifact, encode_artifact,
            format::NONE_IDX,
            prerun::{PrerunPageExecution, page_execution_revision},
            template_from_bytes,
            view::reconstruct_board,
        },
        execution::{InternalRun, RunPayload, context::ExecutionContext},
        node::{Node, NodeLogic},
        pin::{PinType, resolve_schema},
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState, FlowNodeRegistryInner},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::{FromProto, Value, async_trait, intercom::BufferedInterComHandler};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const HEADER_LEN: usize = 40;
const RUNS: usize = 15;

struct StubLogic {
    name: String,
}

#[async_trait]
impl NodeLogic for StubLogic {
    fn get_node(&self) -> Node {
        Node::new(&self.name, &self.name, "bench stub", "Bench")
    }

    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Ok(())
    }
}

/// First call (cold caches and allocator) and the fastest of `RUNS`.
struct Timing {
    first: Duration,
    best: Duration,
}

fn time<T>(mut step: impl FnMut() -> T) -> (Timing, T) {
    let started = Instant::now();
    let mut value = step();
    let first = started.elapsed();
    let mut best = first;
    for _ in 1..RUNS {
        let started = Instant::now();
        value = step();
        best = best.min(started.elapsed());
    }
    (Timing { first, best }, value)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn report(step: &str, timing: &Timing) {
    println!(
        "{step:<54} first {:>8.2} ms   best {:>8.2} ms",
        ms(timing.first),
        ms(timing.best)
    );
}

fn all_nodes(board: &Board) -> impl Iterator<Item = &Node> {
    board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
}

fn stub_registry(board: &Board) -> FlowNodeRegistryInner {
    let names: HashSet<&str> = all_nodes(board).map(|node| node.name.as_str()).collect();
    let mut registry = FlowNodeRegistryInner::new(names.len());
    for name in names {
        let logic = Arc::new(StubLogic {
            name: name.to_string(),
        });
        registry.insert(logic.get_node(), logic);
    }
    registry
}

/// Nodes a run seeded at `start` can reach without a dynamic reference: execution edges forward,
/// data edges backward, layer relay pins followed through, and `fn_refs` targets included.
fn statically_reachable(compiled: &CompiledBoard, start: usize) -> usize {
    let execution = codes::variable_type_code(&VariableType::Execution);
    let output = codes::pin_type_code(&PinType::Output);
    let node_by_id: HashMap<&str, usize> = compiled
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect();
    let mut seen_nodes = HashSet::from([start]);
    let mut seen_pins = HashSet::new();
    let mut nodes = vec![start];
    let mut relay_pins: Vec<(u32, bool)> = Vec::new();

    let visit = |pin: u32,
                 forward: bool,
                 seen_nodes: &mut HashSet<usize>,
                 seen_pins: &mut HashSet<(u32, bool)>,
                 nodes: &mut Vec<usize>,
                 relay_pins: &mut Vec<(u32, bool)>| {
        let owner = compiled.pins[pin as usize].owner_node;
        if owner == NONE_IDX {
            if seen_pins.insert((pin, forward)) {
                relay_pins.push((pin, forward));
            }
        } else if seen_nodes.insert(owner as usize) {
            nodes.push(owner as usize);
        }
    };

    loop {
        if let Some(node) = nodes.pop() {
            let compiled_node = &compiled.nodes[node];
            for &pin_index in &compiled_node.pins {
                let pin = &compiled.pins[pin_index as usize];
                if pin.pin_type == output && pin.data_type == execution {
                    for &target in &pin.connected_to {
                        visit(
                            target,
                            true,
                            &mut seen_nodes,
                            &mut seen_pins,
                            &mut nodes,
                            &mut relay_pins,
                        );
                    }
                } else if pin.pin_type != output && pin.data_type != execution {
                    for &source in &pin.depends_on {
                        visit(
                            source,
                            false,
                            &mut seen_nodes,
                            &mut seen_pins,
                            &mut nodes,
                            &mut relay_pins,
                        );
                    }
                }
            }
            for target in compiled_node
                .fn_refs
                .iter()
                .flat_map(|refs| refs.fn_refs.iter())
            {
                if let Some(&index) = node_by_id.get(target.as_str())
                    && seen_nodes.insert(index)
                {
                    nodes.push(index);
                }
            }
            continue;
        }
        let Some((pin, forward)) = relay_pins.pop() else {
            break;
        };
        let relay = &compiled.pins[pin as usize];
        let next = if forward {
            &relay.connected_to
        } else {
            &relay.depends_on
        };
        for &pin in next {
            visit(
                pin,
                forward,
                &mut seen_nodes,
                &mut seen_pins,
                &mut nodes,
                &mut relay_pins,
            );
        }
    }
    seen_nodes.len()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn cold_start_from_the_compiled_baseline() {
    let (Ok(root), Ok(app_id), Ok(board_id)) = (
        std::env::var("FLOW_LIKE_BENCH_ROOT"),
        std::env::var("FLOW_LIKE_BENCH_APP"),
        std::env::var("FLOW_LIKE_BENCH_BOARD"),
    ) else {
        eprintln!("set FLOW_LIKE_BENCH_ROOT, FLOW_LIKE_BENCH_APP and FLOW_LIKE_BENCH_BOARD");
        return;
    };

    let store = FlowLikeStore::Local(Arc::new(
        LocalObjectStore::new(PathBuf::from(root)).unwrap(),
    ));
    let mut config = FlowLikeConfig::new();
    config.register_bits_store(store.clone());
    config.register_user_store(store.clone());
    config.register_app_storage_store(store.clone());
    config.register_app_meta_store(store.clone());
    let (http_client, _refetch) = HTTPClient::new();
    let state = Arc::new(FlowLikeState::new(config, http_client));
    let board_dir = Path::from("apps").join(app_id.clone());

    println!("== source board (what a run pays once after an edit, minus node_updates) ==");
    let started = Instant::now();
    let proto = Board::load_proto(store.as_generic(), &board_dir, &board_id, None)
        .await
        .unwrap();
    println!(
        "{:<54} {:>14.2} ms",
        "read + lz4 + protobuf decode",
        ms(started.elapsed())
    );
    let started = Instant::now();
    let mut board = Board::from_proto(proto);
    println!("{:<54} {:>14.2} ms", "from_proto", ms(started.elapsed()));
    board.board_dir = board_dir.clone();
    board.app_state = Some(state.clone());
    let pins: usize = all_nodes(&board).map(|node| node.pins.len()).sum();
    println!(
        "nodes {} · pins {} · layers {} · variables {} · pages {} · node types {}",
        all_nodes(&board).count(),
        pins,
        board.layers.len(),
        board.variables.len(),
        board.page_ids.len(),
        all_nodes(&board)
            .map(|node| node.name.as_str())
            .collect::<HashSet<_>>()
            .len(),
    );

    let schemas: Vec<&str> = all_nodes(&board)
        .flat_map(|node| node.pins.values())
        .chain(board.layers.values().flat_map(|layer| layer.pins.values()))
        .filter_map(|pin| pin.schema.as_deref())
        .filter_map(|schema| resolve_schema(schema, &board.refs).ok())
        .collect();
    let distinct: HashSet<&str> = schemas.iter().copied().collect();
    println!(
        "format version {} · {} pins carry a schema ({} B resolved) · {} distinct schemas ({} B) · {} refs",
        board.format_version,
        schemas.len(),
        schemas.iter().map(|schema| schema.len()).sum::<usize>(),
        distinct.len(),
        distinct.iter().map(|schema| schema.len()).sum::<usize>(),
        board.refs.len(),
    );
    let (timing, _) = time(|| board.required_format_version());
    report("Board::required_format_version", &timing);

    let registry = stub_registry(&board);
    let fingerprint = registry.fingerprint();

    let (timing, compiled) = time(|| compile_board(&board).unwrap());
    report("compile_board", &timing);
    let (timing, bytes) = time(|| encode_artifact(&compiled, &fingerprint).unwrap());
    report("encode_artifact (rkyv + zstd-3)", &timing);
    let archive_len =
        u32::from_le_bytes(bytes[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap()) as usize;
    println!(
        "artifact {} B compressed · {} B rkyv archive · {} compiled nodes · {} compiled pins",
        bytes.len(),
        archive_len,
        compiled.nodes.len(),
        compiled.pins.len()
    );

    println!("\n== cold start from the artifact (per process, per board revision) ==");
    let decompress = || {
        let mut aligned = rkyv::util::AlignedVec::<16>::with_capacity(archive_len);
        aligned.resize(archive_len, 0);
        let written =
            zstd::bulk::decompress_to_buffer(&bytes[HEADER_LEN + 4..], aligned.as_mut_slice())
                .unwrap();
        aligned.resize(written, 0);
        aligned
    };
    let (timing, aligned) = time(decompress);
    report("zstd decompress", &timing);
    let (timing, _) = time(|| {
        rkyv::access::<rkyv::Archived<CompiledBoard>, rkyv::rancor::Error>(&aligned)
            .unwrap()
            .nodes
            .len()
    });
    report("rkyv access (validates the whole archive)", &timing);
    let archived =
        rkyv::access::<rkyv::Archived<CompiledBoard>, rkyv::rancor::Error>(&aligned).unwrap();
    let (timing, _) =
        time(|| rkyv::deserialize::<CompiledBoard, rkyv::rancor::Error>(archived).unwrap());
    report("rkyv deserialize into owned CompiledBoard", &timing);
    let (timing, _) = time(|| compiled.required_board_format_version());
    report("CompiledBoard::required_board_format_version", &timing);
    let (timing, _) = time(|| compiled.validate().unwrap());
    report(
        "CompiledBoard::validate (format version + arena bounds)",
        &timing,
    );
    let (timing, _) = time(|| decode_artifact(&bytes, Some(&fingerprint)).unwrap());
    report("decode_artifact (the four steps above)", &timing);
    let (timing, _) =
        time(|| reconstruct_board(&compiled, board_dir.clone(), Some(&registry)).unwrap());
    report("reconstruct_board (Board view)", &timing);
    let defaults: Vec<&Vec<u8>> = compiled
        .pins
        .iter()
        .filter_map(|pin| pin.default_value.as_ref())
        .collect();
    let (timing, _) = time(|| {
        defaults
            .iter()
            .filter_map(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
            .count()
    });
    report(
        &format!(
            "parse {} pin defaults ({} B of JSON)",
            defaults.len(),
            defaults.iter().map(|bytes| bytes.len()).sum::<usize>()
        ),
        &timing,
    );
    let (timing, _) = time(|| {
        CompiledRunTemplate::from_compiled(&compiled, &registry, board_dir.clone()).unwrap()
    });
    report("from_compiled (view + template arenas)", &timing);
    let (timing, template) =
        time(|| template_from_bytes(&bytes, &fingerprint, &registry, &board_dir).unwrap());
    report("template_from_bytes (everything above)", &timing);

    println!("\n== every run ==");
    let start = compiled
        .nodes
        .iter()
        .find(|node| node.start)
        .unwrap_or(&compiled.nodes[0]);
    let payload = RunPayload {
        id: start.id.clone(),
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };
    let profile = Profile::default();
    let callback = BufferedInterComHandler::new(
        Arc::new(move |_event| Box::pin(async move { Ok(()) })),
        Some(100),
        Some(400),
        Some(true),
    )
    .into_callback();
    let mut first = Duration::ZERO;
    let mut best = Duration::MAX;
    for index in 0..RUNS {
        let started = Instant::now();
        let run = InternalRun::from_template(
            &app_id,
            template.clone(),
            None,
            &state,
            &profile,
            &payload,
            false,
            callback.clone(),
            None,
            None,
            HashMap::new(),
            None,
            None,
        )
        .await
        .unwrap();
        let elapsed = started.elapsed();
        if index == 0 {
            first = elapsed;
        }
        best = best.min(elapsed);
        drop(run);
    }
    report("InternalRun::from_template", &Timing { first, best });

    let starts: Vec<usize> = compiled
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.start)
        .map(|(index, _)| index)
        .collect();
    let mut reachable: Vec<usize> = starts
        .iter()
        .map(|&start| statically_reachable(&compiled, start))
        .collect();
    reachable.sort_unstable();
    if let (Some(&smallest), Some(&largest)) = (reachable.first(), reachable.last()) {
        println!(
            "{} entry nodes · statically reachable nodes per entry: min {} · median {} · max {} of {}",
            starts.len(),
            smallest,
            reachable[reachable.len() / 2],
            largest,
            compiled.nodes.len()
        );
    }

    println!("\n== page contract: from the source board, then from the artifact's facts ==");
    let (timing, facts) = time(|| BaselineFacts::from_source(&board, &registry));
    report("BaselineFacts::from_source (once per compile)", &timing);
    let (timing, _) = time(|| board.clone());
    report("Board deep clone", &timing);
    let (timing, _) = time(|| board.content_hash());
    report("Board::content_hash (clone + rehash)", &timing);
    let (timing, _) = time(|| PrerunManifest::from_board(&board));
    report("PrerunManifest::from_board", &timing);
    for page_id in board.page_ids.clone() {
        let started = Instant::now();
        let page = match board.load_page(&page_id, None).await {
            Ok(page) => page,
            Err(error) => {
                println!("page {page_id}: unreadable ({error})");
                continue;
            }
        };
        println!(
            "{:<54} {:>14.2} ms",
            format!("load page {page_id}"),
            ms(started.elapsed())
        );
        let execution = match PrerunPageExecution::from_page(&board, &page) {
            Ok(execution) => execution,
            Err(error) => {
                println!("page {page_id}: no execution contract ({error})");
                continue;
            }
        };
        let (timing, _) = time(|| PrerunPageExecution::from_page(&board, &page).unwrap());
        report("PrerunPageExecution::from_page", &timing);
        match page_execution_revision(&board, &execution) {
            Ok(from_source) => {
                let (timing, _) = time(|| page_execution_revision(&board, &execution).unwrap());
                report("page_execution_revision (source board)", &timing);
                let view = &template.board;
                let (timing, from_view) =
                    time(|| PrerunPageExecution::from_page(view, &page).unwrap());
                report("PrerunPageExecution::from_page (artifact view)", &timing);
                let (timing, from_facts) =
                    time(|| facts.page_execution_revision(view, &from_view).unwrap());
                report("page_execution_revision (artifact facts)", &timing);
                assert_eq!(
                    from_view, execution,
                    "the view yields the source's Page map"
                );
                assert_eq!(from_facts, from_source, "and the source's revision");
            }
            Err(error) => println!("page {page_id}: no revision ({error})"),
        }
        println!(
            "page {page_id}: {} actions · load {} · unload {} · interval {}",
            execution.action_events.len(),
            execution.special_events.load.is_some(),
            execution.special_events.unload.is_some(),
            execution.special_events.interval.is_some()
        );
    }
}
