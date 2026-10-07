use std::{fs, path::PathBuf};

use flow_like_model_provider::embedding::interface::{
    EmbeddingBatch, EmbeddingDescriptor, EmbeddingSpec,
};

fn main() -> anyhow::Result<()> {
    let base = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("packages/schema"));
    for (path, schema) in [
        (
            "bit/bit/embedding-spec.json",
            schemars::schema_for!(EmbeddingSpec),
        ),
        (
            "llm/embedding-descriptor.json",
            schemars::schema_for!(EmbeddingDescriptor),
        ),
        (
            "llm/embedding-batch.json",
            schemars::schema_for!(EmbeddingBatch),
        ),
    ] {
        let path = base.join(path);
        fs::create_dir_all(path.parent().expect("schema parent"))?;
        fs::write(path, serde_json::to_string_pretty(&schema)?)?;
    }
    Ok(())
}
