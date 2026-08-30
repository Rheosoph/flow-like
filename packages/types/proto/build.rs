use std::io::Result;

const PROTOS: &[&str] = &[
    "protobufs/a2ui.proto",
    "protobufs/app.proto",
    "protobufs/bit.proto",
    "protobufs/metadata.proto",
    "protobufs/board.proto",
    "protobufs/execution-plan.proto",
    "protobufs/comment.proto",
    "protobufs/node.proto",
    "protobufs/pin.proto",
    "protobufs/variable.proto",
    "protobufs/event.proto",
];

fn main() -> Result<()> {
    for proto in PROTOS {
        println!("cargo:rerun-if-changed={proto}");
    }
    prost_build::compile_protos(PROTOS, &["protobufs/"])?;
    Ok(())
}
