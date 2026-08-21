use flow_like::flow::board::Board;
use flow_like_types::{FromProto, Message};
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tmp/ie6j0ph9szad636m0kz9xeft-mopo-monitor-main-v1.board".to_string());
    let compressed = std::fs::read(&path).expect("read board file");
    println!("compressed size: {} bytes", compressed.len());

    let t = Instant::now();
    let raw = lz4_flex::decompress_size_prepended(&compressed).expect("lz4 decompress");
    println!("decompress: {:?} ({} bytes raw)", t.elapsed(), raw.len());

    let t = Instant::now();
    let proto = flow_like_types::proto::Board::decode(&raw[..]).expect("proto decode");
    println!("prost decode: {:?}", t.elapsed());
    println!(
        "nodes: {}, variables: {}, comments: {}, layers: {}",
        proto.nodes.len(),
        proto.variables.len(),
        proto.comments.len(),
        proto.layers.len()
    );
    let pin_count: usize = proto.nodes.values().map(|n| n.pins.len()).sum();
    println!("total pins: {}", pin_count);

    let t = Instant::now();
    let board = Board::from_proto(proto);
    println!("Board::from_proto: {:?}", t.elapsed());

    let t = Instant::now();
    let json = flow_like_types::json::to_vec(&board).expect("json");
    println!(
        "(ref) serde_json size: {} bytes in {:?}",
        json.len(),
        t.elapsed()
    );

    // repeat hot to remove page-cache noise
    for i in 0..3 {
        let t = Instant::now();
        let raw = lz4_flex::decompress_size_prepended(&compressed).unwrap();
        let proto = flow_like_types::proto::Board::decode(&raw[..]).unwrap();
        let _board = Board::from_proto(proto);
        println!(
            "hot iter {}: decompress+decode+from_proto = {:?}",
            i,
            t.elapsed()
        );
    }
}
