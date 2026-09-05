use super::ops::{bytes_input, bytes_node, bytes_output};
#[cfg(feature = "execute")]
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BytesGzipCompressNode {}

impl BytesGzipCompressNode {
    pub fn new() -> Self {
        BytesGzipCompressNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesGzipCompressNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_gzip_compress",
            "Gzip Compress",
            "Compresses a byte buffer with gzip",
        );
        node.set_flowscript_name("bytes", "gzipCompress");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");
        node.add_input_pin(
            "level",
            "Level",
            "Compression level from 0 (store) to 9 (smallest)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(6)));

        bytes_output(&mut node, "result", "Bytes", "The compressed bytes");
        node.add_output_pin(
            "ratio",
            "Ratio",
            "Compressed size divided by original size",
            VariableType::Float,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::io::Write;

        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let level: i64 = context.evaluate_pin("level").await?;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::new(level.clamp(0, 9) as u32));
        encoder.write_all(&bytes)?;
        let compressed = encoder.finish()?;

        let ratio = if bytes.is_empty() {
            0.0
        } else {
            compressed.len() as f64 / bytes.len() as f64
        };

        context.set_pin_value("ratio", json!(ratio)).await?;
        context.set_pin_value("result", json!(compressed)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Gzip compression requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesGzipDecompressNode {}

impl BytesGzipDecompressNode {
    pub fn new() -> Self {
        BytesGzipDecompressNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesGzipDecompressNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_gzip_decompress",
            "Gzip Decompress",
            "Restores a gzip compressed byte buffer",
        );
        node.set_flowscript_name("bytes", "gzipDecompress");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Compressed Bytes");
        node.add_input_pin(
            "max_size",
            "Max Size",
            "Refuse to expand beyond this many bytes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(268_435_456)));

        bytes_output(&mut node, "result", "Bytes", "The restored bytes");

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::io::Read;

        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let max_size: i64 = context.evaluate_pin("max_size").await?;
        let max_size = max_size.max(0) as u64;

        // A gzip stream states no honest size, so a small payload can expand without
        // bound. Read through a limit instead of trusting the header.
        let mut decoder = GzDecoder::new(&bytes[..]).take(max_size.saturating_add(1));
        let mut restored = Vec::new();
        decoder.read_to_end(&mut restored)?;

        if restored.len() as u64 > max_size {
            return Err(flow_like_types::anyhow!(
                "Decompressed data exceeds the {max_size} byte limit"
            ));
        }

        context.set_pin_value("result", json!(restored)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Gzip decompression requires the 'execute' feature"
        ))
    }
}
