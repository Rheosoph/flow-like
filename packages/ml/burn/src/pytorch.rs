use crate::{Error, Result};
use pytorch_reader::{DType, PickleValue, PytorchReader};
use safetensors::{Dtype, tensor::TensorView};
use std::{
    collections::HashSet,
    io::{Cursor, Write},
};

const MAX_PICKLE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;

/// Convert a modern torch.save ZIP archive into the explicit tensor format used by importers.
/// The caller records the original checkpoint digest before conversion.
pub(crate) fn convert_pytorch_weights(
    bytes: &[u8],
    maximum_bytes: u64,
) -> Result<(Vec<u8>, String)> {
    preflight_archive(bytes, maximum_bytes)?;
    // The reader uses lazy file-backed tensors. Give it a private immutable snapshot of the
    // already hashed bytes, so reopening a caller's path cannot change the imported weights.
    let mut checkpoint = tempfile::NamedTempFile::new()?;
    checkpoint.write_all(bytes)?;
    checkpoint.flush()?;
    let metadata = PytorchReader::read_pickle_data(checkpoint.path(), None)
        .map_err(|error| Error::Record(format!("Invalid PyTorch metadata: {error}")))?;
    let (prefix, selected) = select_state_dict(&metadata)?;
    if selected.is_empty() || selected.len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::Invalid(
            "PyTorch state dictionary has an invalid tensor count".into(),
        ));
    }
    for (name, value) in selected {
        if name == "_metadata" {
            validate_module_metadata(value)?;
            continue;
        }
        if name.is_empty()
            || name.len() > 1024
            || name.contains('\0')
            || !matches!(value, PickleValue::Unsupported(kind) if kind == "torch.Tensor")
        {
            return Err(Error::Invalid(format!(
                "PyTorch state dictionary entry {name:?} must be a named tensor"
            )));
        }
    }
    let reader = PytorchReader::new(checkpoint.path())
        .map_err(|error| Error::Record(format!("Invalid PyTorch weights: {error}")))?;
    let mut total = 0u64;
    let mut tensors = Vec::with_capacity(selected.len());
    let mut names = selected
        .keys()
        .filter(|name| name.as_str() != "_metadata")
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(Error::Invalid(
            "PyTorch state dictionary contains no tensors".into(),
        ));
    }
    names.sort_unstable();
    for name in names {
        let full_name = format!("{prefix}{name}");
        let tensor = reader
            .get(&full_name)
            .ok_or_else(|| Error::Invalid(format!("Missing PyTorch tensor {full_name}")))?;
        if tensor.shape().len() > 8 || tensor.shape().contains(&0) {
            return Err(Error::Invalid(format!(
                "Invalid PyTorch tensor shape for {name}"
            )));
        }
        let dtype = match tensor.dtype() {
            DType::F32 => Dtype::F32,
            DType::I64 => Dtype::I64,
            DType::Bool => Dtype::BOOL,
            other => {
                return Err(Error::Invalid(format!(
                    "PyTorch tensor {name} has unsupported dtype {other:?}; import needs F32 weights"
                )));
            }
        };
        total = total
            .checked_add(tensor.byte_len() as u64)
            .ok_or_else(|| Error::Invalid("PyTorch tensor byte count overflow".into()))?;
        if total > maximum_bytes {
            return Err(Error::Invalid(
                "Expanded PyTorch tensors exceed the import byte limit".into(),
            ));
        }
        tensors.push((name.clone(), dtype, tensor));
    }
    // Validate every shape and the total expansion before materializing even one storage.
    let tensors = tensors
        .into_iter()
        .map(|(name, dtype, tensor)| Ok((name, dtype, tensor.shape().to_vec(), tensor.read()?)))
        .collect::<Result<Vec<_>>>()?;
    let views = tensors
        .iter()
        .map(|(name, dtype, shape, bytes)| {
            TensorView::new(*dtype, shape.clone(), bytes)
                .map(|view| (name.as_str(), view))
                .map_err(|error| {
                    Error::Record(format!("Invalid converted PyTorch tensor: {error}"))
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let converted = safetensors::serialize(views, None)
        .map_err(|error| Error::Record(format!("Cannot convert PyTorch tensors: {error}")))?;
    if converted.len() as u64 > maximum_bytes {
        return Err(Error::Invalid(
            "Converted PyTorch weights exceed the import byte limit".into(),
        ));
    }
    let selected = if prefix.is_empty() {
        "state_dict"
    } else {
        prefix.trim_end_matches('.')
    };
    Ok((converted, format!("pytorch_zip:{selected}")))
}

fn validate_module_metadata(value: &PickleValue) -> Result<()> {
    let PickleValue::Dict(modules) = value else {
        return Err(Error::Invalid(
            "PyTorch _metadata must contain module versions".into(),
        ));
    };
    if modules.len() > MAX_ARCHIVE_ENTRIES || modules.iter().any(|(name, value)| {
        name.len() > 1024 || name.contains('\0') || !matches!(value,
            PickleValue::Dict(fields) if fields.len() == 1
                && matches!(fields.get("version"), Some(PickleValue::Int(version)) if *version >= 0)
        )
    }) {
        return Err(Error::Invalid("Unrecognized PyTorch module version metadata".into()));
    }
    Ok(())
}

fn select_state_dict(
    metadata: &PickleValue,
) -> Result<(
    &'static str,
    &std::collections::HashMap<String, PickleValue>,
)> {
    let PickleValue::Dict(root) = metadata else {
        return Err(Error::Invalid(
            "PyTorch checkpoint root must be a dictionary".into(),
        ));
    };
    if let Some(ema) = root.get("ema") {
        let PickleValue::Dict(ema) = ema else {
            return Err(Error::Invalid(
                "PyTorch ema must contain a module dictionary".into(),
            ));
        };
        let Some(PickleValue::Dict(module)) = ema.get("module") else {
            return Err(Error::Invalid(
                "PyTorch ema.module must be a tensor dictionary".into(),
            ));
        };
        return Ok(("ema.module.", module));
    }
    for (key, prefix) in [("model", "model."), ("state_dict", "state_dict.")] {
        if let Some(value) = root.get(key) {
            return match value {
                PickleValue::Dict(values) => Ok((prefix, values)),
                _ => Err(Error::Invalid(format!(
                    "PyTorch {key} must be a tensor dictionary"
                ))),
            };
        }
    }
    Ok(("", root))
}

fn preflight_archive(bytes: &[u8], maximum_bytes: u64) -> Result<()> {
    if bytes.len() as u64 > maximum_bytes {
        return Err(Error::Invalid(
            "PyTorch checkpoint exceeds the import byte limit".into(),
        ));
    }
    if !bytes.starts_with(b"PK\x03\x04") {
        return Err(Error::Invalid(
            "Native PyTorch import requires a modern torch.save ZIP checkpoint".into(),
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| Error::Record(format!("Invalid PyTorch ZIP: {error}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::Invalid(
            "PyTorch checkpoint contains too many archive entries".into(),
        ));
    }
    let mut names = HashSet::new();
    let mut root = None::<String>;
    let mut pickle = false;
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| Error::Record(format!("Invalid PyTorch ZIP member: {error}")))?;
        let name = entry.name();
        if !names.insert(name.to_string()) || entry.is_dir() || entry.encrypted() {
            return Err(Error::Invalid(
                "PyTorch ZIP has duplicate, directory or encrypted entries".into(),
            ));
        }
        let (directory, relative) = name.split_once('/').ok_or_else(|| {
            Error::Invalid("PyTorch ZIP entries must share one archive directory".into())
        })?;
        if directory.is_empty()
            || directory == "."
            || directory == ".."
            || directory.contains('\\')
            || root.as_ref().is_some_and(|root| root != directory)
        {
            return Err(Error::Invalid(
                "Invalid PyTorch ZIP archive directory".into(),
            ));
        }
        root.get_or_insert_with(|| directory.to_string());
        if relative == "data.pkl" {
            pickle = true;
            if entry.size() > MAX_PICKLE_BYTES.min(maximum_bytes) {
                return Err(Error::Invalid(
                    "PyTorch pickle metadata exceeds the import limit".into(),
                ));
            }
        } else if let Some(storage) = relative.strip_prefix("data/") {
            if storage.is_empty() || !storage.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(Error::Invalid(
                    "Invalid PyTorch tensor storage entry".into(),
                ));
            }
        } else if ![
            "byteorder",
            "version",
            ".data/serialization_id",
            ".format_version",
            ".storage_alignment",
        ]
        .contains(&relative)
            || entry.size() > 4096
        {
            return Err(Error::Invalid(format!(
                "Unrecognized PyTorch archive entry {name}"
            )));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| Error::Invalid("Expanded PyTorch archive byte count overflow".into()))?;
        if expanded > maximum_bytes {
            return Err(Error::Invalid(
                "Expanded PyTorch archive exceeds the import byte limit".into(),
            ));
        }
    }
    if !pickle {
        return Err(Error::Invalid("PyTorch archive has no data.pkl".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(pickle: &mut Vec<u8>, value: &str) {
        pickle.push(b'X');
        pickle.extend((value.len() as u32).to_le_bytes());
        pickle.extend(value.as_bytes());
    }

    fn integer(pickle: &mut Vec<u8>, value: u32) {
        pickle.push(b'J');
        pickle.extend(value.to_le_bytes());
    }

    fn fixture(section: &str, elements: u32, extra_value: bool) -> Vec<u8> {
        let mut pickle = vec![0x80, 2, b'}', b'('];
        for field in section.split('.').filter(|field| !field.is_empty()) {
            string(&mut pickle, field);
            pickle.extend([b'}', b'(']);
        }
        string(&mut pickle, "linear.weight");
        pickle.extend(b"ctorch._utils\n_rebuild_tensor_v2\n((");
        string(&mut pickle, "storage");
        pickle.extend(b"ctorch\nFloatStorage\n");
        string(&mut pickle, "0");
        string(&mut pickle, "cpu");
        integer(&mut pickle, 1);
        pickle.extend([b't', b'Q']);
        integer(&mut pickle, 0);
        pickle.push(b'(');
        integer(&mut pickle, elements);
        pickle.extend([b't', b'(']);
        integer(&mut pickle, 0);
        pickle.extend([b't', 0x89, b'}', b't', b'R']);
        if extra_value {
            string(&mut pickle, "unexpected");
            integer(&mut pickle, 3);
        }
        for _ in 0..=section.split('.').filter(|field| !field.is_empty()).count() {
            pickle.push(b'u');
        }
        pickle.push(b'.');
        archive(&pickle, None)
    }

    fn archive(pickle: &[u8], extra: Option<(&str, &[u8])>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("weights/data.pkl", pickle),
            ("weights/data/0", 3.5f32.to_le_bytes().as_slice()),
            ("weights/version", b"3".as_slice()),
            ("weights/byteorder", b"little".as_slice()),
        ]
        .into_iter()
        .chain(extra)
        {
            writer.start_file(name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn native_checkpoint_envelopes_preserve_tensor_values_and_names() {
        for section in ["", "model", "ema.module", "state_dict"] {
            let (bytes, format) =
                convert_pytorch_weights(&fixture(section, 2, false), 1024 * 1024).unwrap();
            let tensors = safetensors::SafeTensors::deserialize(&bytes).unwrap();
            assert_eq!(tensors.names(), vec!["linear.weight"]);
            let tensor = tensors.tensor("linear.weight").unwrap();
            assert_eq!(tensor.shape(), &[2]);
            assert_eq!(
                tensor.data(),
                [3.5f32.to_le_bytes(), 3.5f32.to_le_bytes()].concat()
            );
            assert!(format.starts_with("pytorch_zip:"));
        }
    }

    #[test]
    fn import_provenance_hashes_original_checkpoint_and_checks_expected_digest() {
        use sha2::{Digest, Sha256};
        let bytes = fixture("model", 2, false);
        let expected = format!("{:x}", Sha256::digest(&bytes));
        let mut checkpoint = tempfile::NamedTempFile::new().unwrap();
        checkpoint.write_all(&bytes).unwrap();
        let mut metadata = crate::PretrainedWeightsMetadata {
            source: "test fixture".into(),
            license: None,
            weights_license: None,
            revision: None,
            expected_sha256: Some(expected.clone()),
        };
        let (converted, digest, format) =
            crate::pretrained::read_import_weights(checkpoint.path(), &metadata, 1024 * 1024)
                .unwrap();
        assert_eq!(digest, expected);
        assert_eq!(format, "pytorch_zip:model");
        assert_ne!(digest, format!("{:x}", Sha256::digest(&converted)));
        metadata.expected_sha256 = Some("0".repeat(64));
        assert!(
            crate::pretrained::read_import_weights(checkpoint.path(), &metadata, 1024 * 1024)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
    }

    #[test]
    fn rejects_expanded_tensor_views_and_unknown_state_payloads() {
        assert!(
            convert_pytorch_weights(&fixture("model", 1024 * 1024, false), 4096)
                .unwrap_err()
                .to_string()
                .contains("Expanded PyTorch tensors")
        );
        assert!(
            convert_pytorch_weights(&fixture("model", 2, true), 1024 * 1024)
                .unwrap_err()
                .to_string()
                .contains("must be a named tensor")
        );
    }

    #[test]
    fn rejects_compressed_storage_expansion_unknown_members_and_legacy_pickle() {
        let storage = vec![0u8; 1024 * 1024];
        let bytes = archive(b"\x80\x02}.", Some(("weights/data/1", &storage)));
        assert!(bytes.len() < 4096);
        assert!(
            convert_pytorch_weights(&bytes, 4096)
                .unwrap_err()
                .to_string()
                .contains("Expanded PyTorch archive")
        );
        let bytes = archive(
            b"\x80\x02}.",
            Some(("weights/code.py", b"arbitrary payload")),
        );
        assert!(
            convert_pytorch_weights(&bytes, 4096)
                .unwrap_err()
                .to_string()
                .contains("Unrecognized PyTorch archive entry")
        );
        assert!(convert_pytorch_weights(b"\x80\x02}.", 4096).is_err());
    }

    #[test]
    #[ignore = "Requires an external official checkpoint provided by FLOW_LIKE_PYTORCH_CHECKPOINT"]
    fn converts_official_checkpoint() {
        let path = std::env::var("FLOW_LIKE_PYTORCH_CHECKPOINT").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let (converted, format) = convert_pytorch_weights(&bytes, 1024 * 1024 * 1024).unwrap();
        let tensors = safetensors::SafeTensors::deserialize(&converted).unwrap();
        assert!(!tensors.names().is_empty());
        eprintln!("Converted {} tensors from {format}", tensors.names().len());
    }
}
