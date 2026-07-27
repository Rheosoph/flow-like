use std::io::Result;
fn main() -> Result<()> {
    prost_build::compile_protos(
        &[
            "src/protobufs/a2ui.proto",
            "src/protobufs/app.proto",
            "src/protobufs/metadata.proto",
            "src/protobufs/board.proto",
            "src/protobufs/comment.proto",
            "src/protobufs/node.proto",
            "src/protobufs/pin.proto",
            "src/protobufs/variable.proto",
            "src/protobufs/event.proto",
        ],
        &["src/protobufs/"],
    )?;
    Ok(())
}
