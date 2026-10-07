//! Externalize inline tensors without decoding or changing their bytes.

use anyhow::{Context, Result, anyhow, ensure};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_METADATA_BYTES: usize = 512 * 1024 * 1024;
const WEIGHTS_NAME: &str = "weights.onnx";

/// A temporary graph whose external tensors reference byte ranges in the original model.
/// Keep this handle until the consumer has finished opening the graph and its tensors.
pub struct ExternalizedModel {
    directory: tempfile::TempDir,
    tensors: usize,
}

impl ExternalizedModel {
    pub fn model_path(&self) -> PathBuf {
        self.directory.path().join("model.onnx")
    }
    pub fn weights_path(&self) -> PathBuf {
        self.directory.path().join(WEIGHTS_NAME)
    }
    pub fn tensor_count(&self) -> usize {
        self.tensors
    }
}

/// Convert dense top-level initializers to external data for ONNX protobufs above 2 GiB.
/// Each external offset points directly at the original TensorProto.raw_data payload.
/// Unknown fields remain byte-for-byte intact. Oversized nested/typed tensors are rejected.
pub fn externalize_oversized_model(path: impl AsRef<Path>) -> Result<ExternalizedModel> {
    let source = path.as_ref().canonicalize()?;
    let mut file = File::open(&source)?;
    let length = file.metadata()?.len();
    let mut tensors = 0;
    let mut rewritten = Vec::new();
    let mut graphs = 0;
    while let Some(field) = next_field(&mut file, length)? {
        if field.number == 7 && field.wire == 2 {
            graphs += 1;
            let graph = rewrite_graph(&mut file, field.end, &mut tensors)?;
            bytes_field(&mut rewritten, 7, &graph)?;
        } else {
            copy_field(&mut file, field, &mut rewritten)?;
        }
    }
    ensure!(graphs == 1, "ONNX model must contain exactly one graph");
    ensure!(
        tensors > 0,
        "No inline dense initializers found; cannot externalize oversized ONNX graph"
    );
    let directory = tempfile::Builder::new()
        .prefix("flow-like-onnx-")
        .tempdir()?;
    let result = ExternalizedModel { directory, tensors };
    // A hardlink shares the original bytes and costs no duplicate tensor storage.
    // Different filesystems fall back to a file copy with the same byte offsets.
    if std::fs::hard_link(&source, result.weights_path()).is_err() {
        std::fs::copy(&source, result.weights_path())
            .context("Could not materialize external ONNX weights")?;
    }
    std::fs::write(result.model_path(), rewritten)?;
    Ok(result)
}

fn rewrite_graph(file: &mut File, end: u64, tensors: &mut usize) -> Result<Vec<u8>> {
    let mut graph = Vec::new();
    while let Some(field) = next_field(file, end)? {
        if field.number == 5 && field.wire == 2 {
            let tensor = rewrite_tensor(file, field.end, tensors)?;
            bytes_field(&mut graph, 5, &tensor)?;
        } else {
            copy_field(file, field, &mut graph)?;
        }
    }
    Ok(graph)
}

fn rewrite_tensor(file: &mut File, end: u64, tensors: &mut usize) -> Result<Vec<u8>> {
    let mut tensor = Vec::new();
    let mut raw = None;
    while let Some(field) = next_field(file, end)? {
        if field.number == 9 && field.wire == 2 && field.end > field.payload {
            ensure!(
                raw.is_none(),
                "ONNX tensor contains multiple raw_data payloads"
            );
            raw = Some((field.payload, field.end - field.payload));
            file.seek(SeekFrom::Start(field.end))?;
        } else if field.number == 14 && field.wire == 0 {
            file.seek(SeekFrom::Start(field.payload))?;
            ensure!(
                read_varint(file, field.end)? == 0,
                "Oversized graph already contains external tensor metadata"
            );
        } else {
            // Mixed existing external tensors need their own artifact resolver. A partial
            // rewrite must never change the directory used to resolve those files.
            ensure!(
                field.number != 13 && field.number != 14,
                "Oversized graph already contains external tensor metadata"
            );
            copy_field(file, field, &mut tensor)?;
        }
    }
    if let Some((offset, length)) = raw {
        for (key, value) in [
            ("location", WEIGHTS_NAME.to_owned()),
            ("offset", offset.to_string()),
            ("length", length.to_string()),
        ] {
            let mut entry = Vec::new();
            bytes_field(&mut entry, 1, key.as_bytes())?;
            bytes_field(&mut entry, 2, value.as_bytes())?;
            bytes_field(&mut tensor, 13, &entry)?;
        }
        varint(&mut tensor, 14 << 3);
        varint(&mut tensor, 1);
        *tensors += 1;
    }
    Ok(tensor)
}

#[derive(Clone, Copy)]
struct Field {
    start: u64,
    payload: u64,
    end: u64,
    number: u64,
    wire: u8,
}

fn next_field(file: &mut File, limit: u64) -> Result<Option<Field>> {
    let start = file.stream_position()?;
    if start == limit {
        return Ok(None);
    }
    ensure!(start < limit, "Malformed ONNX protobuf field boundary");
    let key = read_varint(file, limit)?;
    let number = key >> 3;
    let wire = (key & 7) as u8;
    ensure!(
        number > 0 && number < (1 << 29),
        "Invalid ONNX protobuf field number"
    );
    let (payload, end) = match wire {
        0 => {
            let payload = file.stream_position()?;
            read_varint(file, limit)?;
            (payload, file.stream_position()?)
        }
        1 | 5 => {
            let payload = file.stream_position()?;
            (
                payload,
                payload
                    .checked_add(if wire == 1 { 8 } else { 4 })
                    .context("ONNX field length overflow")?,
            )
        }
        2 => {
            let length = read_varint(file, limit)?;
            let payload = file.stream_position()?;
            (
                payload,
                payload
                    .checked_add(length)
                    .context("ONNX field length overflow")?,
            )
        }
        _ => return Err(anyhow!("Unsupported ONNX protobuf wire type {wire}")),
    };
    ensure!(
        end <= limit,
        "ONNX protobuf field exceeds its parent message"
    );
    Ok(Some(Field {
        start,
        payload,
        end,
        number,
        wire,
    }))
}

fn copy_field(file: &mut File, field: Field, output: &mut Vec<u8>) -> Result<()> {
    let length = usize::try_from(field.end - field.start)?;
    ensure!(
        output
            .len()
            .checked_add(length)
            .is_some_and(|length| length <= MAX_METADATA_BYTES),
        "ONNX metadata exceeds 512 MiB after externalizing top-level dense initializers; nested or typed tensor data requires a separate export"
    );
    let start = output.len();
    output.resize(start + length, 0);
    file.seek(SeekFrom::Start(field.start))?;
    file.read_exact(&mut output[start..])?;
    Ok(())
}

fn read_varint(file: &mut File, limit: u64) -> Result<u64> {
    let mut value = 0;
    for index in 0..10 {
        ensure!(
            file.stream_position()? < limit,
            "Truncated ONNX protobuf varint"
        );
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        ensure!(
            index < 9 || byte[0] <= 1,
            "ONNX protobuf varint exceeds u64"
        );
        value |= u64::from(byte[0] & 0x7f) << (7 * index);
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(anyhow!("Malformed ONNX protobuf varint"))
}

fn varint(output: &mut Vec<u8>, mut value: u64) {
    while value > 127 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn bytes_field(output: &mut Vec<u8>, number: u64, bytes: &[u8]) -> Result<()> {
    ensure!(
        output
            .len()
            .checked_add(bytes.len() + 20)
            .is_some_and(|length| length <= MAX_METADATA_BYTES),
        "Externalized ONNX graph metadata exceeds 512 MiB"
    );
    varint(output, (number << 3) | 2);
    varint(output, bytes.len() as u64);
    output.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(raw: &[u8]) -> Vec<u8> {
        let mut tensor = Vec::new();
        bytes_field(&mut tensor, 8, b"weight").unwrap();
        bytes_field(&mut tensor, 9, raw).unwrap();
        varint(&mut tensor, 14 << 3);
        varint(&mut tensor, 0);
        let mut graph = Vec::new();
        bytes_field(&mut graph, 2, b"test").unwrap();
        bytes_field(&mut graph, 5, &tensor).unwrap();
        let mut model = Vec::new();
        varint(&mut model, 8);
        varint(&mut model, 9);
        bytes_field(&mut model, 7, &graph).unwrap();
        model
    }

    #[test]
    fn external_weights_keep_the_original_bytes_and_offsets() {
        let raw: Vec<u8> = (0..10000).map(|value| value as u8).collect();
        let original = model(&raw);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.onnx");
        std::fs::write(&path, &original).unwrap();
        let external = externalize_oversized_model(&path).unwrap();
        assert_eq!(external.tensor_count(), 1);
        assert_eq!(std::fs::read(external.weights_path()).unwrap(), original);
        let graph = std::fs::read(external.model_path()).unwrap();
        assert!(graph.len() < 200);
        assert!(
            graph
                .windows(WEIGHTS_NAME.len())
                .any(|bytes| bytes == WEIGHTS_NAME.as_bytes())
        );
        let raw_offset = original
            .windows(raw.len())
            .position(|bytes| bytes == raw)
            .unwrap();
        assert!(
            graph
                .windows(raw_offset.to_string().len())
                .any(|bytes| bytes == raw_offset.to_string().as_bytes())
        );
    }

    #[test]
    fn malformed_protobuf_is_rejected_before_any_bundle_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.onnx");
        for bytes in [vec![0], vec![0x3a, 0xff], vec![0x3a, 10, 1], vec![0xff; 10]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(externalize_oversized_model(&path).is_err());
        }
    }
}
