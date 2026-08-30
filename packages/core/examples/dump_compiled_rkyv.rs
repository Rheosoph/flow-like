//! Dump a board's compiled rkyv payload (uncompressed) for codec experiments:
//!   cargo run -p flow-like --example dump_compiled_rkyv -- <board-file> <out-file>

use flow_like::flow::board::Board;
use flow_like::flow::compiled::compile_board;
use flow_like_types::{FromProto, Message};

fn main() {
    let board_path = std::env::args().nth(1).expect("board file");
    let out_path = std::env::args().nth(2).expect("out file");
    let compressed = std::fs::read(&board_path).expect("read board file");
    let raw = lz4_flex::decompress_size_prepended(&compressed).expect("lz4");
    let proto = flow_like_types::proto::Board::decode(&raw[..]).expect("proto");
    let board = Board::from_proto(proto);
    let compiled = compile_board(&board).expect("compile");
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&compiled).expect("rkyv");
    std::fs::write(&out_path, bytes.as_slice()).expect("write");
    println!("wrote {} bytes to {}", bytes.len(), out_path);

    let t = std::time::Instant::now();
    let artifact =
        flow_like::flow::compiled::encode_artifact(&compiled, &[0u8; 32]).expect("encode");
    let encode_elapsed = t.elapsed();
    let t = std::time::Instant::now();
    let decoded = flow_like::flow::compiled::decode_artifact(&artifact, None).expect("decode");
    println!(
        "artifact: {} bytes (encode {:?}, decode {:?})",
        artifact.len(),
        encode_elapsed,
        t.elapsed()
    );
    assert_eq!(decoded, compiled);
}
