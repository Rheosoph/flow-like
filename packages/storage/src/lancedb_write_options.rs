use lance::dataset::{WriteMode, WriteParams};
use lance_file::version::LanceFileVersion;

/// Build default LanceDB write options for new datasets and overwrites.
///
/// Note: `mode` must be set to `Append` (not the `WriteParams::default()` of
/// `Create`) because lancedb 0.27 passes user-supplied `lance_write_params`
/// straight through to lance without overriding the mode field. Setting
/// `Create` causes "Dataset already exists" errors on every `table.add()`.
pub fn default_write_options() -> lancedb::table::WriteOptions {
    lancedb::table::WriteOptions {
        lance_write_params: Some(WriteParams {
            data_storage_version: Some(LanceFileVersion::V2_2),
            mode: WriteMode::Append,
            ..Default::default()
        }),
    }
}
