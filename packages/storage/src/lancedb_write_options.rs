use lance::dataset::{WriteMode, WriteParams};

/// Default LanceDB write options: append, and leave the file format to Lance.
///
/// New tables get Lance's stable format (V2.2). Appends keep the table's own
/// format: an explicit version that differs from it makes Lance 13 mark the table
/// as mixed-version, which older readers cannot open.
pub fn default_write_options() -> lancedb::table::WriteOptions {
    lancedb::table::WriteOptions {
        lance_write_params: Some(WriteParams {
            mode: WriteMode::Append,
            ..Default::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Int32Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use lance::dataset::InsertBuilder;
    use lance_file::version::{ConcreteFileVersion, LanceFileVersion};
    use lance_table::feature_flags::FLAG_MIXED_DATA_FILE_VERSIONS;
    use std::sync::Arc;

    fn batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));
        RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1]))]).unwrap()
    }

    #[tokio::test]
    async fn appends_keep_an_older_tables_file_format() {
        let created = InsertBuilder::new("memory://")
            .with_params(&WriteParams {
                data_storage_version: Some(LanceFileVersion::V2_1),
                ..Default::default()
            })
            .execute(vec![batch()])
            .await
            .unwrap();
        let params = default_write_options().lance_write_params.unwrap();

        let appended = InsertBuilder::new(Arc::new(created))
            .with_params(&params)
            .execute(vec![batch()])
            .await
            .unwrap();

        let manifest = appended.manifest();
        assert_eq!(
            manifest.reader_feature_flags & FLAG_MIXED_DATA_FILE_VERSIONS,
            0
        );
        for fragment in manifest.fragments.iter() {
            assert_eq!(
                fragment.files[0].file_version().unwrap(),
                ConcreteFileVersion::V2_1
            );
        }
    }

    #[tokio::test]
    async fn new_tables_use_the_stable_format() {
        let params = default_write_options().lance_write_params.unwrap();
        let created = InsertBuilder::new("memory://")
            .with_params(&WriteParams {
                mode: WriteMode::Create,
                ..params
            })
            .execute(vec![batch()])
            .await
            .unwrap();
        assert_eq!(
            created.manifest().fragments[0].files[0]
                .file_version()
                .unwrap(),
            ConcreteFileVersion::V2_2
        );
    }
}
