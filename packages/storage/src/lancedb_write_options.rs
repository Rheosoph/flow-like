use lance::dataset::WriteParams;
use lance_file::version::LanceFileVersion;

/// Build default LanceDB write options for new datasets and overwrites.
pub fn default_write_options() -> lancedb::table::WriteOptions {
    lancedb::table::WriteOptions {
        lance_write_params: Some(WriteParams {
            data_storage_version: Some(LanceFileVersion::V2_2),
            ..Default::default()
        }),
    }
}