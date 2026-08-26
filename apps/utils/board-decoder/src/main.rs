use clap::Parser;
use flow_like::flow::board::Board;
use flow_like::utils::compression::from_compressed;
use flow_like_types_proto::FromProto;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    input: String,
}

#[tokio::main]
async fn main() {
    let args = Cli::parse();

    match translate_board(&args.input).await {
        Ok(_) => println!("Board translated successfully"),
        Err(e) => {
            eprintln!("Error translating board: {}", e);
            process::exit(1);
        }
    }
}

async fn translate_board(path: &str) -> anyhow::Result<()> {
    let mut path_buf = PathBuf::from(path);
    let parent = path_buf.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "Parent directory not found")
    })?;
    let parent_string = parent.canonicalize()?.to_string_lossy().to_string();
    let file_name = path_buf
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "File name not found"))?;
    let object_store =
        flow_like_storage::object_store::local::LocalFileSystem::new_with_prefix(parent_string)?;
    let object_store: Arc<dyn flow_like_storage::object_store::ObjectStore> =
        Arc::new(object_store);
    let path = flow_like_storage::Path::from(file_name.to_string_lossy().to_string());
    let board: flow_like_types_proto::proto::Board = from_compressed(object_store, path).await?;
    let board = Board::from_proto(board);
    path_buf.set_extension(".board.json");
    let json = serde_json::to_string_pretty(&board).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize board: {}", e),
        )
    })?;
    fs::write(&path_buf, json).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            format!("Failed to write file: {}", e),
        )
    })?;
    println!("Board written to: {}", path_buf.display());
    Ok(())
}
