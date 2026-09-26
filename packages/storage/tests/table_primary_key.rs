//! Table key marker (Lance unenforced primary key): concurrent upserts of the same new
//! ID must commit one row, and the key follows the rules in todo/table-primary-key.md.

use std::{path::PathBuf, sync::Arc};

use flow_like_storage::arrow_schema::DataType;
use flow_like_storage::databases::vector::{
    VectorStore,
    lancedb::LanceDBVectorStore,
    schema::{DatabaseSchemaField, PrimaryKeyRejected, database_fields_to_arrow_schema},
};
use flow_like_storage::lancedb::table::ColumnAlteration;
use flow_like_types::{Error, Result, Value, create_id, json::json};
use tokio::sync::{Barrier, Mutex};

const TABLE: &str = "items";
const WRITERS: usize = 8;
const RACED_IDS: [&str; 3] = ["a", "b", "c"];

struct TempDb(PathBuf);

impl TempDb {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("flow-like-table-key-{}", create_id())))
    }

    async fn open(&self) -> Result<LanceDBVectorStore> {
        LanceDBVectorStore::new(self.0.clone(), TABLE.into()).await
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn seed_row() -> Value {
    json!({ "id": "seed", "value": 0 })
}

fn string_ids() -> Vec<Value> {
    RACED_IDS.iter().map(|id| json!(id)).collect()
}

fn raced_rows(ids: &[Value], writer: usize) -> Vec<Value> {
    ids.iter()
        .map(|id| json!({ "id": id, "value": writer as i64 }))
        .collect()
}

fn id_filter(id: &Value) -> String {
    match id {
        Value::String(id) => format!("id = '{id}'"),
        id => format!("id = {id}"),
    }
}

fn schema_field(name: &str, data_type: &str, nullable: bool) -> DatabaseSchemaField {
    DatabaseSchemaField {
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        vector_size: None,
        primary_key: false,
    }
}

fn key_field(name: &str, data_type: &str) -> DatabaseSchemaField {
    DatabaseSchemaField {
        primary_key: true,
        ..schema_field(name, data_type, false)
    }
}

fn rejection(error: Error) -> String {
    assert!(
        error.downcast_ref::<PrimaryKeyRejected>().is_some(),
        "expected a key rule rejection, got: {error:#}"
    );
    error.to_string()
}

/// One race at a time: parallel races share the CPU, stretch Lance's retry backoff and
/// could hit its retry timeout without any product defect.
static RACE: Mutex<()> = Mutex::const_new(());

/// Every writer opens its own handle before the barrier, so all of them commit
/// against the same stale base.
async fn race_upserts(db: &TempDb, ids: &[Value]) -> Result<()> {
    let _race = RACE.lock().await;
    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut writers = Vec::with_capacity(WRITERS);
    for writer in 0..WRITERS {
        let mut store = db.open().await?;
        let barrier = barrier.clone();
        let rows = raced_rows(ids, writer);
        writers.push(tokio::spawn(async move {
            barrier.wait().await;
            store.upsert(rows, "id".into()).await
        }));
    }
    for (writer, handle) in writers.into_iter().enumerate() {
        handle
            .await?
            .map_err(|error| flow_like_types::anyhow!("writer {writer} failed: {error:#}"))?;
    }
    Ok(())
}

async fn assert_one_row_per_raced_id(db: &TempDb, ids: &[Value]) -> Result<()> {
    let store = db.open().await?;
    for id in ids {
        let rows = store.count(Some(id_filter(id))).await?;
        assert_eq!(
            rows, 1,
            "id {id} has {rows} rows after {WRITERS} racing upserts"
        );
    }
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));
    Ok(())
}

/// JSON strings infer as LargeUtf8, which Lance cannot BTree-index, so the indexed
/// races declare a Utf8 `id` up front.
async fn seed_with_btree_on_id(db: &TempDb, keyed_by_upsert: bool) -> Result<()> {
    let mut seed = db.open().await?;
    let schema = database_fields_to_arrow_schema(&[
        schema_field("id", "string", false),
        schema_field("value", "int64", true),
    ])?;
    seed.create_empty_table(schema, false).await?;
    if keyed_by_upsert {
        seed.upsert(vec![seed_row()], "id".into()).await?;
        assert_eq!(seed.primary_key().await?.as_deref(), Some("id"));
    } else {
        seed.insert(vec![seed_row()]).await?;
        assert_eq!(seed.primary_key().await?, None);
    }
    seed.index("id", Some("BTREE")).await?;
    let indices = seed.list_indices().await?;
    assert!(
        indices
            .iter()
            .any(|index| index.index_type == "BTREE" && index.columns == ["id"]),
        "the BTree index on the key must exist before the race: {indices:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_upserts_of_new_ids_keep_one_row_each() -> Result<()> {
    let db = TempDb::new();
    let mut seed = db.open().await?;
    seed.upsert(vec![seed_row()], "id".into()).await?;
    assert_eq!(seed.primary_key().await?.as_deref(), Some("id"));

    race_upserts(&db, &string_ids()).await?;
    assert_one_row_per_raced_id(&db, &string_ids()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_upserts_mark_an_unkeyed_table_once() -> Result<()> {
    let db = TempDb::new();
    let mut seed = db.open().await?;
    seed.insert(vec![seed_row()]).await?;
    assert_eq!(seed.primary_key().await?, None);

    race_upserts(&db, &string_ids()).await?;
    assert_one_row_per_raced_id(&db, &string_ids()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_upserts_bypass_a_btree_index_on_the_key() -> Result<()> {
    let db = TempDb::new();
    seed_with_btree_on_id(&db, true).await?;

    race_upserts(&db, &string_ids()).await?;
    assert_one_row_per_raced_id(&db, &string_ids()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_upserts_mark_an_indexed_unkeyed_table() -> Result<()> {
    let db = TempDb::new();
    seed_with_btree_on_id(&db, false).await?;

    race_upserts(&db, &string_ids()).await?;
    assert_one_row_per_raced_id(&db, &string_ids()).await
}

/// Non-negative JSON numbers infer as UInt64. lancedb's own key setter refuses that type,
/// but Lance's conflict filter hashes it, so the store writes the marker itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_upserts_of_new_numeric_ids_keep_one_row_each() -> Result<()> {
    let db = TempDb::new();
    let mut seed = db.open().await?;
    seed.upsert(vec![json!({ "id": 0, "value": 0 })], "id".into())
        .await?;
    assert_eq!(seed.primary_key().await?.as_deref(), Some("id"));

    let ids = [json!(1), json!(2), json!(3)];
    race_upserts(&db, &ids).await?;
    assert_one_row_per_raced_id(&db, &ids).await
}

#[tokio::test]
async fn upserts_keyed_on_another_column_still_succeed() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .upsert(
            vec![json!({ "id": "a", "name": "alpha", "value": 1 })],
            "id".into(),
        )
        .await?;
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));

    store
        .upsert(
            vec![
                json!({ "id": "a", "name": "alpha", "value": 2 }),
                json!({ "id": "b", "name": "beta", "value": 3 }),
            ],
            "name".into(),
        )
        .await?;

    assert_eq!(store.count(None).await?, 2);
    assert_eq!(
        store
            .count(Some("name = 'alpha' AND value = 2".into()))
            .await?,
        1
    );
    assert_eq!(store.count(Some("name = 'beta'".into())).await?, 1);
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));
    Ok(())
}

/// Floats are not a key type Lance's conflict filter hashes: the table stays unkeyed and
/// upserts behave as before.
#[tokio::test]
async fn upserts_on_ineligible_id_columns_stay_unkeyed() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .upsert(vec![json!({ "id": 1.5, "value": 1 })], "id".into())
        .await?;
    store
        .upsert(
            vec![
                json!({ "id": 1.5, "value": 2 }),
                json!({ "id": 2.5, "value": 3 }),
            ],
            "id".into(),
        )
        .await?;

    assert_eq!(store.primary_key().await?, None);
    assert_eq!(store.count(None).await?, 2);
    assert_eq!(store.count(Some("id = 1.5 AND value = 2".into())).await?, 1);
    Ok(())
}

/// Lance's conflict filter skips FixedSizeBinary, so such a key would look protected
/// without being checked.
#[tokio::test]
async fn set_primary_key_rejects_types_the_conflict_filter_skips() -> Result<()> {
    use flow_like_storage::arrow_schema::{Field, Schema};

    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .create_empty_table(
            Schema::new(vec![
                Field::new("hash", DataType::FixedSizeBinary(16), false),
                Field::new("value", DataType::Int64, true),
            ]),
            false,
        )
        .await?;

    let message = rejection(store.set_primary_key("hash").await.unwrap_err());
    assert!(message.contains("not supported"), "{message}");
    assert_eq!(store.primary_key().await?, None);
    Ok(())
}

#[tokio::test]
async fn set_primary_key_is_idempotent_and_immutable() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .insert(vec![
            json!({ "id": "a", "name": "alpha", "tag": "x" }),
            json!({ "id": "b", "name": "beta", "tag": null }),
        ])
        .await?;
    assert_eq!(store.primary_key().await?, None);

    let error = rejection(store.set_primary_key("tag").await.unwrap_err());
    assert!(
        error.contains("'tag'") && error.contains(TABLE) && error.contains("nullable"),
        "{error}"
    );
    let error = rejection(store.set_primary_key("missing").await.unwrap_err());
    assert!(error.contains("'missing'"), "{error}");

    store.set_primary_key("id").await?;
    store.set_primary_key("id").await?;
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));

    let error = rejection(store.set_primary_key("name").await.unwrap_err());
    assert!(
        error.contains("'id'") && error.contains("'name'"),
        "{error}"
    );
    assert_eq!(db.open().await?.primary_key().await?.as_deref(), Some("id"));
    Ok(())
}

#[tokio::test]
async fn set_primary_key_accepts_a_key_another_handle_set() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .insert(vec![json!({ "id": "a" }), json!({ "id": "b" })])
        .await?;
    let stale = db.open().await?;

    store.set_primary_key("id").await?;
    stale.set_primary_key("id").await?;
    assert_eq!(stale.primary_key().await?.as_deref(), Some("id"));
    Ok(())
}

#[tokio::test]
async fn set_primary_key_rejects_duplicate_values() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .insert(vec![
            json!({ "id": "a" }),
            json!({ "id": "a" }),
            json!({ "id": "a" }),
            json!({ "id": "b" }),
        ])
        .await?;

    let error = rejection(store.set_primary_key("id").await.unwrap_err());
    assert!(error.contains("2 duplicate"), "{error}");
    assert_eq!(store.primary_key().await?, None);
    Ok(())
}

async fn keyed_table(db: &TempDb) -> Result<LanceDBVectorStore> {
    let mut store = db.open().await?;
    store
        .upsert(
            vec![json!({ "id": "a", "name": "alpha", "value": 1 })],
            "id".into(),
        )
        .await?;
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));
    Ok(store)
}

/// Lance itself accepts all three changes: a nullable key then fails every later
/// upsert and insert, and dropping it silently removes the key.
#[tokio::test]
async fn the_key_column_cannot_become_optional_or_be_dropped() -> Result<()> {
    let db = TempDb::new();
    let mut store = keyed_table(&db).await?;

    let error = rejection(store.make_column_nullable("id", true).await.unwrap_err());
    assert!(
        error.contains("'id'") && error.contains(TABLE) && error.contains("required"),
        "{error}"
    );

    let error = rejection(
        store
            .alter_column(&[ColumnAlteration::new("id".into()).set_nullable(true)])
            .await
            .unwrap_err(),
    );
    assert!(error.contains("'id'") && error.contains(TABLE), "{error}");

    let error = rejection(store.drop_columns(&["value", "id"]).await.unwrap_err());
    assert!(
        error.contains("'id'") && error.contains(TABLE) && error.contains("removed"),
        "{error}"
    );

    let error = rejection(
        store
            .alter_column(&[ColumnAlteration::new("id".into()).cast_to(DataType::Utf8)])
            .await
            .unwrap_err(),
    );
    assert!(error.contains("keep its type"), "{error}");

    rejection(store.make_column_nullable("`id`", true).await.unwrap_err());
    rejection(store.drop_columns(&["`id`"]).await.unwrap_err());

    store.make_column_nullable("name", true).await?;
    store
        .alter_column(&[ColumnAlteration::new("id".into()).rename("item_id".into())])
        .await?;
    assert_eq!(store.primary_key().await?.as_deref(), Some("item_id"));
    store
        .upsert(
            vec![json!({ "item_id": "a", "name": null, "value": 2 })],
            "item_id".into(),
        )
        .await?;
    assert_eq!(store.count(None).await?, 1);
    Ok(())
}

#[tokio::test]
async fn created_tables_carry_their_declared_key() -> Result<()> {
    let db = TempDb::new();
    let schema = database_fields_to_arrow_schema(&[
        key_field("ticket_id", "string"),
        schema_field("title", "string", true),
    ])?;
    let mut store = db.open().await?;
    assert!(store.create_empty_table(schema.clone(), false).await?);
    assert!(!store.create_empty_table(schema, true).await?);
    assert_eq!(store.primary_key().await?.as_deref(), Some("ticket_id"));

    for title in ["first", "second"] {
        store
            .upsert(
                vec![json!({ "ticket_id": "t-1", "title": title })],
                "ticket_id".into(),
            )
            .await?;
    }
    assert_eq!(store.count(None).await?, 1);
    assert_eq!(store.count(Some("title = 'second'".into())).await?, 1);

    let error =
        database_fields_to_arrow_schema(&[key_field("a", "string"), key_field("b", "int64")])
            .unwrap_err()
            .to_string();
    assert!(error.contains("'a'") && error.contains("'b'"), "{error}");
    Ok(())
}

/// An upsert keys the table after it was created, so re-running the same idempotent
/// create must still succeed; declaring a different key must not.
#[tokio::test]
async fn create_if_not_exists_accepts_a_table_its_upserts_keyed() -> Result<()> {
    let db = TempDb::new();
    let declared = database_fields_to_arrow_schema(&[
        schema_field("id", "string", false),
        schema_field("sku", "string", false),
    ])?;
    let mut store = db.open().await?;
    assert!(store.create_empty_table(declared.clone(), true).await?);
    store
        .upsert(vec![json!({ "id": "a", "sku": "s-1" })], "id".into())
        .await?;
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));

    assert!(!db.open().await?.create_empty_table(declared, true).await?);
    let other_key = database_fields_to_arrow_schema(&[
        schema_field("id", "string", false),
        key_field("sku", "string"),
    ])?;
    assert!(
        db.open()
            .await?
            .create_empty_table(other_key, true)
            .await
            .is_err()
    );
    Ok(())
}

/// Schema changes commit a whole schema; one built on a handle opened before the key
/// was set would silently drop it, and its guard would not see the key.
#[tokio::test]
async fn schema_changes_from_a_stale_handle_keep_a_newer_key() -> Result<()> {
    let db = TempDb::new();
    db.open()
        .await?
        .insert(vec![json!({ "id": "a", "note": "x", "value": 1 })])
        .await?;
    let stale = db.open().await?;
    let mut keyer = db.open().await?;
    keyer
        .upsert(
            vec![json!({ "id": "b", "note": "y", "value": 2 })],
            "id".into(),
        )
        .await?;
    assert_eq!(keyer.primary_key().await?.as_deref(), Some("id"));

    stale.make_column_nullable("note", true).await?;
    assert_eq!(db.open().await?.primary_key().await?.as_deref(), Some("id"));
    rejection(stale.drop_columns(&["id"]).await.unwrap_err());
    rejection(stale.make_column_nullable("id", true).await.unwrap_err());
    Ok(())
}

/// A column already holding duplicates is evidently not a unique ID; keying it would
/// be irreversible, so upserts leave the table unkeyed.
#[tokio::test]
async fn upserts_do_not_key_a_column_holding_duplicates() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .insert(vec![
            json!({ "id": "a", "value": 1 }),
            json!({ "id": "a", "value": 2 }),
        ])
        .await?;
    store
        .upsert(vec![json!({ "id": "b", "value": 3 })], "id".into())
        .await?;
    assert_eq!(store.primary_key().await?, None);
    assert_eq!(store.count(Some("id = 'b'".into())).await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_batch_that_fails_to_convert_leaves_the_table_unkeyed() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store.insert(vec![json!({ "id": "a", "value": 1 })]).await?;
    assert!(
        store
            .upsert(
                vec![json!({ "id": "b", "value": { "not": "a number" } })],
                "id".into()
            )
            .await
            .is_err()
    );
    assert_eq!(store.primary_key().await?, None);
    Ok(())
}

/// Lance can make a column required once it holds no NULLs; the same store must then
/// key it instead of remembering it as ineligible.
#[tokio::test]
async fn an_id_column_made_required_later_is_keyed() -> Result<()> {
    let db = TempDb::new();
    let mut store = db.open().await?;
    store
        .create_empty_table(
            database_fields_to_arrow_schema(&[
                schema_field("id", "string", true),
                schema_field("value", "int64", true),
            ])?,
            false,
        )
        .await?;
    store
        .upsert(vec![json!({ "id": "a", "value": 1 })], "id".into())
        .await?;
    assert_eq!(store.primary_key().await?, None);

    store.make_column_nullable("id", false).await?;
    store
        .upsert(vec![json!({ "id": "b", "value": 2 })], "id".into())
        .await?;
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));
    Ok(())
}

/// Keyed upserts retry when an insert commits in between; they must still land.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn keyed_upserts_succeed_while_inserts_commit_concurrently() -> Result<()> {
    let db = TempDb::new();
    let mut seed = db.open().await?;
    seed.upsert(vec![seed_row()], "id".into()).await?;
    assert_eq!(seed.primary_key().await?.as_deref(), Some("id"));

    let race = RACE.lock().await;
    let half = WRITERS / 2;
    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut writers = Vec::with_capacity(WRITERS);
    for writer in 0..WRITERS {
        let mut store = db.open().await?;
        let barrier = barrier.clone();
        writers.push(tokio::spawn(async move {
            barrier.wait().await;
            if writer < half {
                store
                    .upsert(raced_rows(&string_ids(), writer), "id".into())
                    .await
            } else {
                store
                    .insert(vec![json!({ "id": format!("log-{writer}"), "value": 0 })])
                    .await
            }
        }));
    }
    for (writer, handle) in writers.into_iter().enumerate() {
        handle
            .await?
            .map_err(|error| flow_like_types::anyhow!("writer {writer} failed: {error:#}"))?;
    }
    drop(race);

    assert_one_row_per_raced_id(&db, &string_ids()).await?;
    assert_eq!(
        db.open()
            .await?
            .count(Some("id LIKE 'log-%'".into()))
            .await?,
        half
    );
    Ok(())
}

/// An overwrite commits the query's schema, which carries no key marker.
#[tokio::test]
async fn sql_insert_overwrite_is_refused_on_a_keyed_table() -> Result<()> {
    use flow_like_storage::datafusion::prelude::SessionContext;

    let db = TempDb::new();
    let store = keyed_table(&db).await?;
    let ctx = SessionContext::new();
    ctx.register_table(TABLE, store.to_datafusion().await?)?;

    let error = match ctx
        .sql("INSERT OVERWRITE items VALUES ('z', 'zeta', 9)")
        .await
    {
        Ok(frame) => frame.collect().await.unwrap_err().to_string(),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("INSERT OVERWRITE"), "{error}");
    assert_eq!(store.primary_key().await?.as_deref(), Some("id"));
    assert_eq!(store.count(None).await?, 1);
    Ok(())
}

/// The UI finds the key by reading field metadata from the JSON schema that
/// `db_schema` (desktop) and `GET /db/{table}/schema` (API) return.
#[tokio::test]
async fn the_schema_json_marks_only_the_key_column() -> Result<()> {
    let db = TempDb::new();
    let store = keyed_table(&db).await?;
    let schema = flow_like_types::json::to_value(store.schema().await?)?;
    let fields = schema["fields"]
        .as_array()
        .ok_or_else(|| flow_like_types::anyhow!("schema JSON has no fields: {schema}"))?;
    let marked: Vec<&str> = fields
        .iter()
        .filter(|field| {
            field["metadata"]
                .get("lance-schema:unenforced-primary-key:position")
                .is_some()
        })
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(marked, ["id"], "{schema}");
    Ok(())
}
