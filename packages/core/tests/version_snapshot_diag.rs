use flow_like::flow::board::Board;
use flow_like::utils::compression::from_compressed;
use flow_like_storage::Path;
use flow_like_storage::object_store::ObjectStore;
use flow_like_types::FromProto;
use std::sync::Arc;

const APP_DIR: &str =
    "/Users/felix/Library/Application Support/flow-like/projects/apps/kdx5aylm4sh49a6hf8y5xoou";

async fn store() -> Arc<dyn ObjectStore> {
    Arc::new(
        flow_like_storage::object_store::local::LocalFileSystem::new_with_prefix(APP_DIR).unwrap(),
    )
}

async fn load(path: Path) -> Board {
    let proto: flow_like_types::proto::Board = from_compressed(store().await, path).await.unwrap();
    Board::from_proto(proto)
}

fn content_hash(board: &Board, version: (u32, u32, u32)) -> u64 {
    let mut b = board.clone();
    b.version = version;
    b.hash();
    b.hash.unwrap()
}

fn report(label: &str, a: &Board, b: &Board) {
    println!("--- {label} ---");
    println!("nodes {} vs {}", a.nodes.len(), b.nodes.len());
    println!("layers {} vs {}", a.layers.len(), b.layers.len());
    println!("vars {} vs {}", a.variables.len(), b.variables.len());
    println!("comments {} vs {}", a.comments.len(), b.comments.len());
    println!("refs {:?} vs {:?}", a.refs.len(), b.refs.len());
    println!("page_ids {:?} vs {:?}", a.page_ids, b.page_ids);
    println!("name {:?} vs {:?}", a.name, b.name);
    println!(
        "desc len {} vs {}",
        a.description.len(),
        b.description.len()
    );
    println!("viewport {:?} vs {:?}", a.viewport, b.viewport);
    println!("stage {:?} vs {:?}", a.stage, b.stage);
    println!("exec mode {:?} vs {:?}", a.execution_mode, b.execution_mode);

    let mut ah = a.clone();
    ah.hash();
    let mut bh = b.clone();
    bh.hash();

    for (id, node) in &ah.nodes {
        match bh.nodes.get(id) {
            None => println!("node only in A: {id} ({})", node.name),
            Some(other) => {
                if other.hash != node.hash {
                    println!("node hash differs: {id} ({})", node.name);
                    println!("   A pins {} B pins {}", node.pins.len(), other.pins.len());
                    for (pid, p) in &node.pins {
                        match other.pins.get(pid) {
                            None => {
                                println!("      pin only in A: {pid} {} idx {}", p.name, p.index)
                            }
                            Some(op) => {
                                let a = format!("{:?}", p);
                                let b = format!("{:?}", op);
                                if a != b {
                                    println!("      pin differs: {pid} {}", p.name);
                                    println!("         A: {a}");
                                    println!("         B: {b}");
                                }
                            }
                        }
                    }
                    for pid in other.pins.keys() {
                        if !node.pins.contains_key(pid) {
                            println!("      pin only in B: {pid} {}", other.pins[pid].name);
                        }
                    }
                }
            }
        }
    }
    for id in bh.nodes.keys() {
        if !ah.nodes.contains_key(id) {
            println!("node only in B: {id} ({})", bh.nodes[id].name);
        }
    }
    for (id, layer) in &ah.layers {
        match bh.layers.get(id) {
            None => println!("layer only in A: {id} ({})", layer.name),
            Some(other) => {
                if other.hash != layer.hash {
                    println!(
                        "layer hash differs: {id} ({}) pins {} vs {}",
                        layer.name,
                        layer.pins.len(),
                        other.pins.len()
                    );
                    for (pid, p) in &layer.pins {
                        if !other.pins.contains_key(pid) {
                            println!(
                                "      layer pin only in A: {pid} {} {:?}",
                                p.name, p.pin_type
                            );
                        }
                    }
                    for (pid, p) in &other.pins {
                        if !layer.pins.contains_key(pid) {
                            println!(
                                "      layer pin only in B: {pid} {} {:?}",
                                p.name, p.pin_type
                            );
                        }
                    }
                }
            }
        }
    }
    for id in bh.layers.keys() {
        if !ah.layers.contains_key(id) {
            println!("layer only in B: {id} ({})", bh.layers[id].name);
        }
    }
}

#[tokio::test]
#[ignore]
async fn consecutive_orphan_snapshots_should_be_identical() {
    let a = load(Path::from(
        "versions/w3hd3eg9d6pxo8qsyczfhwug/0_0_4720.board",
    ))
    .await;
    let b = load(Path::from(
        "versions/w3hd3eg9d6pxo8qsyczfhwug/0_0_4721.board",
    ))
    .await;
    let ha = content_hash(&a, (0, 0, 0));
    let hb = content_hash(&b, (0, 0, 0));
    println!("stored hash A {:?} B {:?}", a.hash, b.hash);
    println!("recomputed A {ha} B {hb}");
    if ha != hb {
        report("4720 vs 4721", &a, &b);
    }
    assert_eq!(ha, hb, "two snapshots written from the same draft differ");
}

#[tokio::test]
#[ignore]
async fn floating_draft_matches_last_orphan_snapshot() {
    let floating = load(Path::from("w3hd3eg9d6pxo8qsyczfhwug.board")).await;
    let snap = load(Path::from(
        "versions/w3hd3eg9d6pxo8qsyczfhwug/0_0_4721.board",
    ))
    .await;
    let hf = content_hash(&floating, (0, 0, 0));
    let hs = content_hash(&snap, (0, 0, 0));
    println!("floating {hf} snapshot {hs}");
    if hf != hs {
        report("floating vs 4721", &floating, &snap);
    }
    assert_eq!(hf, hs);
}

#[tokio::test]
#[ignore]
async fn jrednwnd_floating_matches_orphan() {
    let floating = load(Path::from("jrednwnd3spe9jzcwiym92ol.board")).await;
    let snap = load(Path::from("versions/jrednwnd3spe9jzcwiym92ol/0_0_1.board")).await;
    println!(
        "floating version {:?} snapshot version {:?}",
        floating.version, snap.version
    );
    let hf = content_hash(&floating, (0, 0, 0));
    let hs = content_hash(&snap, (0, 0, 0));
    println!("floating {hf} snapshot {hs}");
    if hf != hs {
        report("jrednwnd floating vs 0.0.1", &floating, &snap);
    }
    assert_eq!(hf, hs);
}

#[tokio::test]
#[ignore]
async fn proto_roundtrip_is_lossless_for_the_content_hash() {
    for name in [
        "w3hd3eg9d6pxo8qsyczfhwug.board",
        "jrednwnd3spe9jzcwiym92ol.board",
        "k5ptc3rndugz6o85gjxqo2ui.board",
        "uty90yiejz98ihht0o2p7v41.board",
    ] {
        let a = load(Path::from(name)).await;
        let b = Board::from_proto(flow_like_types::ToProto::to_proto(&a));
        let ha = content_hash(&a, (0, 0, 0));
        let hb = content_hash(&b, (0, 0, 0));
        println!("{name}: roundtrip {ha} vs {hb} -> {}", ha == hb);
        if ha != hb {
            report(&format!("{name} roundtrip"), &a, &b);
        }
    }
}

#[tokio::test]
#[ignore]
async fn cleanup_is_a_fixed_point_on_a_saved_board() {
    for name in [
        "w3hd3eg9d6pxo8qsyczfhwug.board",
        "jrednwnd3spe9jzcwiym92ol.board",
        "k5ptc3rndugz6o85gjxqo2ui.board",
        "uty90yiejz98ihht0o2p7v41.board",
    ] {
        let a = load(Path::from(name)).await;
        let mut b = a.clone();
        b.cleanup();
        let ha = content_hash(&a, (0, 0, 0));
        let hb = content_hash(&b, (0, 0, 0));
        println!("{name}: cleanup {ha} vs {hb} -> {}", ha == hb);
        if ha != hb {
            report(&format!("{name} cleanup"), &a, &b);
        }
    }
}
