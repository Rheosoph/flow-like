use crate::{Error, Result};
use pytorch_reader::{DType, PickleValue, PytorchReader};
use safetensors::{Dtype, tensor::TensorView};
use std::{
    collections::HashSet,
    io::{Cursor, Write},
};

const MAX_PICKLE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
/// Storage bytes all tensor reads together may touch, as a multiple of the import byte limit.
const MAX_READ_AMPLIFICATION: u64 = 64;

/// Legacy torch.save output: a raw pickle stream (PyTorch 0.1.10 to 1.5, or
/// `_use_new_zipfile_serialization=False`) or a TAR archive (before 0.1.10). A SafeTensors
/// file never matches, since its ninth byte opens the JSON header.
pub(crate) fn is_legacy_checkpoint(bytes: &[u8]) -> bool {
    let pickle = matches!(bytes, [0x80, 2..=5, ..]);
    let tar = bytes.get(257..262) == Some(b"ustar".as_slice());
    (pickle || tar) && bytes.get(8) != Some(&b'{')
}

/// Convert a modern torch.save ZIP archive into the explicit tensor format used by importers.
/// The caller records the original checkpoint digest before conversion.
pub(crate) fn convert_pytorch_weights(
    bytes: &[u8],
    maximum_bytes: u64,
) -> Result<(Vec<u8>, String)> {
    let largest_storage = preflight_archive(bytes, maximum_bytes)?;
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
    // Each real tensor is one torch.Tensor leaf and one uniquely named reader entry. A forged
    // leaf passes the entry check above, and a real tensor elsewhere could supply its name.
    let typed = tensor_typed_leaves(&metadata);
    if typed != reader.len() {
        return Err(Error::Invalid(format!(
            "PyTorch checkpoint holds values typed torch.Tensor that are not tensors ({typed} typed, {} readable)",
            reader.len()
        )));
    }
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
    // A tensor reads only inside its own uncompressed storage entry, but views can share one
    // large storage, so the bytes read are bounded by the largest entry per tensor.
    let read_limit = maximum_bytes.saturating_mul(MAX_READ_AMPLIFICATION);
    if (names.len() as u64).saturating_mul(largest_storage) > read_limit {
        return Err(Error::Invalid(format!(
            "PyTorch checkpoint has {} tensors over storages of up to {largest_storage} bytes, which may read more than the {read_limit} byte import read limit",
            names.len()
        )));
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

/// Values typed torch.Tensor in the dicts and lists the reader collects tensors from.
fn tensor_typed_leaves(root: &PickleValue) -> usize {
    let mut pending = vec![root];
    let mut count = 0;
    while let Some(value) = pending.pop() {
        match value {
            PickleValue::Dict(entries) => pending.extend(entries.values()),
            PickleValue::List(items) => pending.extend(items),
            PickleValue::Unsupported(kind) if kind == "torch.Tensor" => count += 1,
            _ => {}
        }
    }
    count
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

/// Returns the size of the largest tensor storage entry.
fn preflight_archive(bytes: &[u8], maximum_bytes: u64) -> Result<u64> {
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
    archive
        .validate_header_consistency()
        .map_err(|error| Error::Record(format!("Inconsistent PyTorch ZIP headers: {error}")))?;
    let mut names = HashSet::new();
    let mut root = None::<String>;
    let mut pickle = false;
    let mut expanded = 0u64;
    let mut largest_storage = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| Error::Record(format!("Invalid PyTorch ZIP member: {error}")))?;
        let name = entry
            .name()
            .map_err(|error| Error::Record(format!("Invalid PyTorch ZIP member name: {error}")))?;
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
            // torch.save never compresses storages; a compressed one would be inflated from its
            // start again on every tensor read.
            if entry.compression() != zip::CompressionMethod::Stored
                || entry.compressed_size() != entry.size()
            {
                return Err(Error::Invalid(format!(
                    "PyTorch tensor storage {name} must be stored uncompressed, as torch.save writes it"
                )));
            }
            largest_storage = largest_storage.max(entry.size());
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
    Ok(largest_storage)
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

    /// A one-dimensional float view into the storage saved as `data/0`.
    fn view(
        pickle: &mut Vec<u8>,
        name: &str,
        storage_elements: u32,
        offset: u32,
        elements: u32,
        stride: u32,
    ) {
        string(pickle, name);
        pickle.extend(b"ctorch._utils\n_rebuild_tensor_v2\n((");
        string(pickle, "storage");
        pickle.extend(b"ctorch\nFloatStorage\n");
        string(pickle, "0");
        string(pickle, "cpu");
        integer(pickle, storage_elements);
        pickle.extend([b't', b'Q']);
        integer(pickle, offset);
        pickle.push(b'(');
        integer(pickle, elements);
        pickle.extend([b't', b'(']);
        integer(pickle, stride);
        pickle.extend([b't', 0x89, b'}', b't', b'R']);
    }

    fn fixture(section: &str, elements: u32, extra_value: bool) -> Vec<u8> {
        let mut pickle = vec![0x80, 2, b'}', b'('];
        for field in section.split('.').filter(|field| !field.is_empty()) {
            string(&mut pickle, field);
            pickle.extend(b"}(");
        }
        view(&mut pickle, "linear.weight", 1, 0, elements, 0);
        if extra_value {
            string(&mut pickle, "unexpected");
            integer(&mut pickle, 3);
        }
        let depth = section.split('.').filter(|field| !field.is_empty()).count();
        pickle.extend(std::iter::repeat_n(b'u', depth + 1));
        pickle.push(b'.');
        archive(&pickle, &3.5f32.to_le_bytes(), None, &[])
    }

    fn shared_storage(views: u32, elements: u32) -> Vec<u8> {
        let mut pickle = vec![0x80, 2, b'}', b'('];
        for index in 0..views {
            view(&mut pickle, &format!("view{index}"), elements, index, 1, 1);
        }
        pickle.extend(b"u.");
        let storage = (0..elements)
            .flat_map(|value| (value as f32).to_le_bytes())
            .collect::<Vec<_>>();
        archive(&pickle, &storage, None, &[])
    }

    /// Entries are stored uncompressed, as torch.save writes them, unless listed in `deflated`.
    fn archive(
        pickle: &[u8],
        storage: &[u8],
        extra: Option<(&str, &[u8])>,
        deflated: &[&str],
    ) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in [
            ("weights/data.pkl", pickle),
            ("weights/data/0", storage),
            ("weights/version", b"3".as_slice()),
            ("weights/byteorder", b"little".as_slice()),
        ]
        .into_iter()
        .chain(extra)
        {
            let method = if deflated.contains(&name) {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            };
            let options = zip::write::SimpleFileOptions::default().compression_method(method);
            writer.start_file(name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn rejects_conflicting_storage_headers_before_import() {
        let mut bytes = fixture("model", 2, false);
        assert_eq!(preflight_archive(&bytes, 1024 * 1024).unwrap(), 4);
        let offset = zip::ZipArchive::new(Cursor::new(&bytes))
            .unwrap()
            .by_name("weights/data/0")
            .unwrap()
            .header_start() as usize;
        bytes[offset + 30] = b'x';
        let error = convert_pytorch_weights(&bytes, 1024 * 1024).unwrap_err();
        assert!(error.to_string().contains("filename mismatch"), "{error}");
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
        let bytes = archive(b"\x80\x02}.", &storage, None, &["weights/data/0"]);
        assert!(bytes.len() < 4096);
        assert!(
            convert_pytorch_weights(&bytes, 4096)
                .unwrap_err()
                .to_string()
                .contains("must be stored uncompressed")
        );
        let mut pickle = b"\x80\x02}.".to_vec();
        pickle.resize(4090, 0);
        let bytes = archive(&pickle, &[0; 4], None, &["weights/data.pkl"]);
        assert!(bytes.len() < 4096);
        assert!(
            convert_pytorch_weights(&bytes, 4096)
                .unwrap_err()
                .to_string()
                .contains("Expanded PyTorch archive")
        );
        let bytes = archive(
            b"\x80\x02}.",
            &[0; 4],
            Some(("weights/code.py", b"arbitrary payload")),
            &[],
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
    fn bounds_the_bytes_read_through_views_of_one_shared_storage() {
        let limit = 128 * 1024;
        let (bytes, _) = convert_pytorch_weights(&shared_storage(100, 16 * 1024), limit).unwrap();
        let tensors = safetensors::SafeTensors::deserialize(&bytes).unwrap();
        assert_eq!(tensors.len(), 100);
        assert_eq!(
            tensors.tensor("view99").unwrap().data(),
            99f32.to_le_bytes()
        );
        assert!(
            convert_pytorch_weights(&shared_storage(200, 16 * 1024), limit)
                .unwrap_err()
                .to_string()
                .contains("import read limit")
        );
    }

    #[test]
    fn rejects_forged_tensor_leaves_whose_name_a_real_tensor_supplies() {
        // {"model": {"linear.weight": forged}, "model.linear.weight": tensor}: the forged entry
        // passes the per-entry check, and the import would read the tensor outside the dict.
        for forged in [
            b"ctorch\nTensor\n".as_slice(),
            b"ctorch\nTensor\n)R".as_slice(),
        ] {
            let mut pickle = vec![0x80, 2, b'}', b'('];
            string(&mut pickle, "model");
            pickle.extend(b"}(");
            string(&mut pickle, "linear.weight");
            pickle.extend(forged);
            pickle.push(b'u');
            view(&mut pickle, "model.linear.weight", 1, 0, 2, 0);
            pickle.extend(b"u.");
            let bytes = archive(&pickle, &3.5f32.to_le_bytes(), None, &[]);
            let error = convert_pytorch_weights(&bytes, 1024 * 1024).unwrap_err();
            assert!(
                error.to_string().contains("that are not tensors"),
                "{error}"
            );
        }
    }

    #[test]
    fn legacy_checkpoints_get_a_resave_hint_and_safetensors_never_match() {
        let pickle = b"\x80\x02\x8a\x0a\x6c\xfc\x9c\x46\xf9\x20\x6a\xa8\x50\x19";
        let mut tar = vec![0u8; 1024];
        tar[..8].copy_from_slice(b"sys_info");
        tar[257..263].copy_from_slice(b"ustar\0");
        // A 640-byte SafeTensors header length also starts with 0x80 0x02.
        let header = format!(
            "{:<640}",
            r#"{"w":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#
        );
        let weights = [
            640u64.to_le_bytes().as_slice(),
            header.as_bytes(),
            1f32.to_le_bytes().as_slice(),
        ]
        .concat();
        safetensors::SafeTensors::deserialize(&weights).unwrap();
        let metadata = crate::PretrainedWeightsMetadata {
            source: "test fixture".into(),
            license: None,
            weights_license: None,
            revision: None,
            expected_sha256: None,
        };
        let import = |bytes: &[u8]| {
            let mut checkpoint = tempfile::NamedTempFile::new().unwrap();
            checkpoint.write_all(bytes).unwrap();
            crate::pretrained::read_import_weights(checkpoint.path(), &metadata, 4096)
        };
        for legacy in [pickle.as_slice(), tar.as_slice()] {
            assert!(is_legacy_checkpoint(legacy));
            assert!(
                import(legacy)
                    .unwrap_err()
                    .to_string()
                    .contains("Legacy PyTorch checkpoint format is not supported")
            );
        }
        assert!(!is_legacy_checkpoint(&weights));
        assert_eq!(import(&weights).unwrap().2, "safetensors_f32");
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
