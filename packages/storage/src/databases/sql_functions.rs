use std::sync::Arc;

use datafusion::catalog::{SchemaProvider, TableProvider};
use datafusion::common::{DataFusionError, Result as DataFusionResult, TableReference};
use datafusion::prelude::SessionContext;
use futures::FutureExt;
use lancedb::index::scalar::FullTextSearchQuery;
use lancedb::table::datafusion::udtf::fts::{FtsTableFunction, TableResolver};

/// Register Lance's token, JSONB and mounted-table FTS functions with Flow's spatial SQL.
/// Spatial registration runs last to retain our geometry metadata and relation fixes.
pub fn register_sql_functions(context: &SessionContext) {
    lance_datafusion::udf::register_functions(context);
    crate::geometry::register_geo_functions(context);
    let session = context.state_weak_ref();
    context.register_udtf(
        "fts",
        Arc::new(FtsTableFunction::new(Arc::new(MountedLanceResolver {
            resolve_location: Box::new(move |name| {
                let session = session.upgrade().ok_or_else(|| {
                    DataFusionError::Plan("fts(): its SQL session is no longer available".into())
                })?;
                let (catalogs, reference) = {
                    let state = session.read();
                    let options = state.config_options();
                    let reference = TableReference::parse_str_normalized(
                        name,
                        !options.sql_parser.enable_ident_normalization,
                    )
                    .resolve(
                        &options.catalog.default_catalog,
                        &options.catalog.default_schema,
                    );
                    (state.catalog_list().clone(), reference)
                };
                let schema = catalogs
                    .catalog(&reference.catalog)
                    .and_then(|catalog| catalog.schema(&reference.schema))
                    .ok_or_else(|| {
                        DataFusionError::Plan(format!("fts(): table '{name}' is not mounted"))
                    })?;
                Ok((schema, reference.table.to_string()))
            }),
        }))),
    );
}

type ResolveTableLocation =
    dyn Fn(&str) -> DataFusionResult<(Arc<dyn SchemaProvider>, String)> + Send + Sync;

struct MountedLanceResolver {
    resolve_location: Box<ResolveTableLocation>,
}

impl std::fmt::Debug for MountedLanceResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MountedLanceResolver")
            .finish_non_exhaustive()
    }
}

impl TableResolver for MountedLanceResolver {
    fn resolve_table(
        &self,
        name: &str,
        query: Option<FullTextSearchQuery>,
    ) -> DataFusionResult<Arc<dyn TableProvider>> {
        let (schema, table) = (self.resolve_location)(name)?;
        let missing = || DataFusionError::Plan(format!("fts(): table '{name}' is not mounted"));
        // The upstream table-function API is synchronous. In-memory catalogs
        // resolve mounted providers immediately; never block on remote discovery.
        let provider = schema
            .table(&table)
            .now_or_never()
            .ok_or_else(|| DataFusionError::Plan(format!(
                "fts(): table '{name}' uses asynchronous discovery; mount its Lance provider in the session first"
            )))??
            .ok_or_else(missing)?;
        let query = query
            .ok_or_else(|| DataFusionError::Plan("fts() requires a full-text query".into()))?;
        super::vector::lancedb::full_text_table_provider(&provider, query).ok_or_else(|| {
            DataFusionError::Plan(format!(
                "fts(): table '{name}' must be a mounted Lance table"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use arrow_array::{Int64Array, LargeBinaryArray, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use datafusion::execution::FunctionRegistry;
    use flow_like_types::json::json;
    use flow_like_types::{Result, Value};

    use super::*;
    use crate::databases::df_provider::lance_table_provider;
    use crate::databases::vector::lancedb::{
        LanceDBVectorStore, LocalWriteReceipt, LogicalTableMutation, LogicalTableMutationAdapter,
        connect_lance,
    };

    async fn rows(context: &SessionContext, query: &str) -> Result<Vec<Value>> {
        Ok(context
            .sql(query)
            .await?
            .collect()
            .await?
            .iter()
            .map(crate::arrow_utils::record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat())
    }

    #[tokio::test]
    async fn registers_lance_scalar_functions_and_evaluates_tokens() -> Result<()> {
        let context = SessionContext::new();
        assert!(context.udf("contains_tokens").is_err());
        assert!(context.udf("json_get_string").is_err());
        register_sql_functions(&context);
        for name in [
            "contains_tokens",
            "json_extract",
            "json_extract_with_type",
            "json_exists",
            "json_get",
            "json_get_string",
            "json_get_int",
            "json_get_float",
            "json_get_bool",
            "json_array_contains",
            "json_array_length",
        ] {
            assert!(context.udf(name).is_ok(), "missing Lance function {name}");
        }
        assert_eq!(
            rows(
                &context,
                "SELECT contains_tokens(text, 'cat fish') AS matches \
                 FROM (VALUES ('a cat and fish'), ('cat only'), (CAST(NULL AS VARCHAR))) AS docs(text) \
                 ORDER BY text NULLS LAST",
            )
            .await?,
            vec![json!({"matches": true}), json!({"matches": false}), json!({"matches": null})]
        );
        Ok(())
    }

    #[tokio::test]
    async fn json_functions_query_jsonb_and_preserve_nulls() -> Result<()> {
        let context = SessionContext::new();
        register_sql_functions(&context);
        let encoded = jsonb::parse_value(
            br#"{"category":"news","count":42,"ratio":1.5,"enabled":true,"tags":["featured","local"],"nested":{"name":"inner"}}"#,
        )?
        .to_vec();
        context.register_batch(
            "documents",
            RecordBatch::try_new(
                Arc::new(Schema::new(vec![Field::new(
                    "payload",
                    DataType::LargeBinary,
                    true,
                )])),
                vec![Arc::new(LargeBinaryArray::from(vec![
                    Some(encoded.as_slice()),
                    None,
                ]))],
            )?,
        )?;
        let actual = rows(
            &context,
            "SELECT json_get_string(payload, 'category') AS category, \
             json_get_int(payload, 'count') AS count, \
             json_get_float(payload, 'ratio') AS ratio, \
             json_get_bool(payload, 'enabled') AS enabled, \
             json_exists(payload, '$.missing') AS missing, \
             json_extract(payload, '$.count') AS extracted, \
             json_get_string(json_get(payload, 'nested'), 'name') AS nested, \
             json_array_contains(payload, '$.tags', 'featured') AS featured, \
             json_array_length(payload, '$.tags') AS tags, \
             get_field(json_extract_with_type(payload, '$.count'), 'type_tag') AS type_tag \
             FROM documents ORDER BY category NULLS LAST",
        )
        .await?;
        assert_eq!(
            actual[0],
            json!({
                "category": "news", "count": 42, "ratio": 1.5, "enabled": true,
                "missing": false, "extracted": "42", "nested": "inner",
                "featured": true, "tags": 2, "type_tag": 2
            })
        );
        assert_eq!(actual.len(), 2);
        for (name, value) in actual[1].as_object().expect("row object") {
            if name == "type_tag" {
                assert_eq!(value, &json!(0));
            } else {
                assert!(
                    value.is_null(),
                    "{name} should preserve the typed null: {value}"
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn combined_registration_retains_spatial_fixes_and_geometry_metadata() -> Result<()> {
        let context = SessionContext::new();
        register_sql_functions(&context);
        register_sql_functions(&context);
        let batch = crate::arrow_utils::value_to_record_batch_with_fields(
            vec![
                json!({"id": 1, "geom": {"type": "Point", "coordinates": [2.0, 2.0]}}),
                json!({"id": 2, "geom": {"type": "Point", "coordinates": [9.0, 9.0]}}),
            ],
            Some(vec![
                Arc::new(Field::new("id", DataType::Int64, false)),
                Arc::new(crate::geometry::geometry_field("geom", true)),
            ]),
        )?;
        context.register_batch("shapes", batch)?;
        for predicate in [
            "ST_Contains(flow_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'), geom)",
            "ST_Within(geom, flow_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'))",
        ] {
            assert_eq!(
                rows(
                    &context,
                    &format!("SELECT id FROM shapes WHERE {predicate}")
                )
                .await?,
                vec![json!({"id": 1})]
            );
        }
        let actual = rows(
            &context,
            "SELECT ST_AsGeoJSON(geom) AS geojson, \
             ST_GeomFromGeoJSON(ST_AsGeoJSON(geom)) AS restored, ST_AsText(geom) AS wkt \
             FROM shapes WHERE id = 1",
        )
        .await?;
        let point = json!({"type": "Point", "coordinates": [2.0, 2.0]});
        assert_eq!(actual[0]["restored"], point);
        assert_eq!(
            flow_like_types::json::from_str::<Value>(actual[0]["geojson"].as_str().unwrap())?,
            point
        );
        assert_eq!(
            flow_like_geometry::from_wkt(actual[0]["wkt"].as_str().unwrap())?,
            point
        );
        Ok(())
    }

    struct FtsFixture {
        directory: std::path::PathBuf,
        connection: lancedb::Connection,
        table: lancedb::Table,
    }

    impl Drop for FtsFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    impl FtsFixture {
        async fn new() -> Result<Self> {
            let directory = std::env::temp_dir().join(format!(
                "flow-sql-functions-{}",
                flow_like_types::create_id()
            ));
            std::fs::create_dir_all(&directory)?;
            let connection = connect_lance(directory.to_str().unwrap()).execute().await?;
            let batch = RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new("text", DataType::Utf8, false),
                ])),
                vec![
                    Arc::new(Int64Array::from(vec![1, 4, 3])),
                    Arc::new(StringArray::from(vec!["cat fish", "dog", "cat sleeps"])),
                ],
            )?;
            let table = connection
                .create_table("documents", vec![batch])
                .execute()
                .await?;
            table
                .create_index(&["text"], lancedb::index::Index::FTS(Default::default()))
                .execute()
                .await?;
            Ok(Self {
                directory,
                connection,
                table,
            })
        }
    }

    fn fts_query(table: &str) -> String {
        format!(
            r#"SELECT id FROM fts('{table}', '{{"match":{{"column":"text","terms":"cat"}}}}') ORDER BY id"#
        )
    }

    #[tokio::test]
    async fn fts_resolves_current_mounted_tables_and_rejects_writes() -> Result<()> {
        let fixture = FtsFixture::new().await?;
        let context = SessionContext::new_with_state(
            crate::databases::df_provider::with_lance_order_pushdown(SessionContext::new().state()),
        );
        register_sql_functions(&context);
        context.register_table(
            "documents",
            lance_table_provider(fixture.table.clone(), true).await?,
        )?;
        assert_eq!(
            rows(&context, &fts_query("datafusion.public.documents")).await?,
            vec![json!({"id": 1}), json!({"id": 3})]
        );
        let source = r#"fts('documents', '{"match":{"column":"text","terms":"cat"}}')"#;
        assert_eq!(
            rows(&context, &format!("SELECT COUNT(*) AS count FROM {source}")).await?,
            vec![json!({"count": 2})]
        );
        assert_eq!(
            rows(
                &context,
                &format!("SELECT id FROM {source} ORDER BY id DESC LIMIT 1")
            )
            .await?,
            vec![json!({"id": 3})]
        );
        let ranked = rows(
            &context,
            &format!("SELECT id, _score FROM {source} ORDER BY _score DESC, id LIMIT 1"),
        )
        .await?;
        assert_eq!(ranked.len(), 1);
        assert!(matches!(ranked[0]["id"].as_i64(), Some(1 | 3)));
        assert!(
            ranked[0]["_score"]
                .as_f64()
                .is_some_and(|score| score > 0.0)
        );
        assert!(
            rows(&context, &fts_query("unmounted"))
                .await
                .unwrap_err()
                .to_string()
                .contains("not mounted")
        );

        let fts = super::super::vector::lancedb::full_text_table_provider(
            &context.table_provider("documents").await?,
            FullTextSearchQuery::new_query(lance_index::scalar::inverted::query::FtsQuery::Match(
                lance_index::scalar::inverted::query::MatchQuery::new("cat".into())
                    .with_column(Some("text".into())),
            )),
        )
        .expect("known Lance provider");
        context.register_table("hits", fts)?;
        for query in [
            "INSERT INTO hits (id, text) VALUES (9, 'cat')",
            "UPDATE hits SET text = 'changed' WHERE id = 1",
            "DELETE FROM hits WHERE id = 1",
        ] {
            let failed = match context.sql(query).await {
                Err(_) => true,
                Ok(dataframe) => dataframe.collect().await.is_err(),
            };
            assert!(failed, "FTS provider accepted mutation: {query}");
        }
        assert_eq!(fixture.table.count_rows(None).await?, 3);

        context.deregister_table("documents")?;
        assert!(
            rows(&context, &fts_query("documents"))
                .await
                .unwrap_err()
                .to_string()
                .contains("not mounted")
        );
        context.register_batch(
            "documents",
            RecordBatch::try_new(
                Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
                vec![Arc::new(Int64Array::from(vec![9]))],
            )?,
        )?;
        assert!(
            rows(&context, &fts_query("documents"))
                .await
                .unwrap_err()
                .to_string()
                .contains("mounted Lance table")
        );
        Ok(())
    }

    struct ReadGate {
        table: lancedb::Table,
        denied: AtomicBool,
    }

    #[flow_like_types::async_trait]
    impl LogicalTableMutationAdapter for ReadGate {
        async fn apply(&self, _: LogicalTableMutation) -> Result<LocalWriteReceipt> {
            Err(flow_like_types::anyhow!("test adapter is read-only"))
        }

        async fn read_table(&self) -> Result<Option<lancedb::Table>> {
            if self.denied.load(Ordering::Acquire) {
                return Err(flow_like_types::anyhow!("authorization denied"));
            }
            Ok(Some(self.table.clone()))
        }
    }

    #[tokio::test]
    async fn fts_keeps_pinned_versions_and_rechecks_managed_authorization() -> Result<()> {
        let fixture = FtsFixture::new().await?;
        let context = SessionContext::new();
        register_sql_functions(&context);
        let pinned = fixture.connection.open_table("documents").execute().await?;
        pinned.checkout(fixture.table.version().await?).await?;
        context.register_table("snapshot", lance_table_provider(pinned, false).await?)?;
        let gate = Arc::new(ReadGate {
            table: fixture.table.clone(),
            denied: AtomicBool::new(false),
        });
        let managed = LanceDBVectorStore::from_connection_for_overlay(
            fixture.connection.clone(),
            "documents".into(),
            Default::default(),
        )?
        .with_mutation_adapter(gate.clone());
        context.register_table("managed", managed.to_datafusion().await?)?;
        assert_eq!(rows(&context, &fts_query("managed")).await?.len(), 2);
        fixture.table.delete("id = 1").await?;
        assert_eq!(
            rows(&context, &fts_query("snapshot")).await?,
            vec![json!({"id": 1}), json!({"id": 3})]
        );
        assert_eq!(
            rows(&context, &fts_query("managed")).await?,
            vec![json!({"id": 3})]
        );
        gate.denied.store(true, Ordering::Release);
        for query in [
            fts_query("managed"),
            r#"SELECT COUNT(*) FROM fts('managed', '{"match":{"column":"text","terms":"cat"}}')"#
                .into(),
        ] {
            assert!(
                rows(&context, &query)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("authorization denied")
            );
        }
        Ok(())
    }

    #[derive(Debug)]
    struct DeferredSchema;

    #[flow_like_types::async_trait]
    impl datafusion::catalog::SchemaProvider for DeferredSchema {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn table_names(&self) -> Vec<String> {
            vec!["documents".into()]
        }

        fn table_exist(&self, name: &str) -> bool {
            name == "documents"
        }

        async fn table(&self, _: &str) -> DataFusionResult<Option<Arc<dyn TableProvider>>> {
            futures::future::pending().await
        }
    }

    #[tokio::test]
    async fn fts_rejects_pending_catalog_discovery_without_blocking() -> Result<()> {
        let context = SessionContext::new();
        register_sql_functions(&context);
        context
            .catalog("datafusion")
            .unwrap()
            .register_schema("deferred", Arc::new(DeferredSchema))?;
        let error = rows(&context, &fts_query("deferred.documents"))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("asynchronous discovery"),
            "{error}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn fts_tracks_session_name_resolution_and_catalog_replacement() -> Result<()> {
        use datafusion::catalog::{MemoryCatalogProviderList, MemorySchemaProvider};

        let fixture = FtsFixture::new().await?;
        let context = SessionContext::new();
        register_sql_functions(&context);
        let provider = lance_table_provider(fixture.table.clone(), true).await?;
        context.register_table("documents", provider.clone())?;
        context.register_table(TableReference::bare("MixedDocs"), provider.clone())?;
        assert_eq!(rows(&context, &fts_query("\"MixedDocs\"")).await?.len(), 2);
        assert!(rows(&context, &fts_query("MixedDocs")).await.is_err());
        context
            .sql("SET datafusion.sql_parser.enable_ident_normalization = false")
            .await?
            .collect()
            .await?;
        assert_eq!(rows(&context, &fts_query("MixedDocs")).await?.len(), 2);

        context
            .catalog("datafusion")
            .unwrap()
            .register_schema("alternate", Arc::new(MemorySchemaProvider::new()))?;
        context
            .sql("SET datafusion.catalog.default_schema = 'alternate'")
            .await?
            .collect()
            .await?;
        assert!(
            rows(&context, &fts_query("documents"))
                .await
                .unwrap_err()
                .to_string()
                .contains("not mounted")
        );
        assert_eq!(
            rows(&context, &fts_query("public.documents")).await?.len(),
            2
        );
        context.register_table("documents", provider)?;
        assert_eq!(rows(&context, &fts_query("documents")).await?.len(), 2);
        context.register_catalog_list(Arc::new(MemoryCatalogProviderList::new()));
        assert!(
            rows(&context, &fts_query("documents"))
                .await
                .unwrap_err()
                .to_string()
                .contains("not mounted")
        );
        Ok(())
    }
}
