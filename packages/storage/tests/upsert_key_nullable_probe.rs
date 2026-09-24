use std::sync::Arc;

use arrow_array::{RecordBatchIterator, RecordBatch, StringArray};
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
use flow_like_types::json::json;
use futures::TryStreamExt;
use lancedb::{
    query::{ExecutableQuery, QueryBase},
    table::FieldMetadataUpdate,
};
use tokio::sync::Barrier;

const PK: &str = "lance-schema:unenforced-primary-key";

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn probe_key_on_json_inferred_table() {
    let dir = std::env::temp_dir().join(format!("pk-null-{}", flow_like_types::create_id()));
    let mut store = LanceDBVectorStore::new(dir.clone(), "t".into()).await.unwrap();
    store
        .upsert(vec![json!({"id": "seed", "value": 0})], "id".into())
        .await
        .unwrap();

    let uri = dir.to_string_lossy().to_string();
    let conn = lancedb::connect(&uri).execute().await.unwrap();
    let table = conn.open_table("t").execute().await.unwrap();
    let schema = table.schema().await.unwrap();
    println!(
        "inferred id nullable={}",
        schema.field_with_name("id").unwrap().is_nullable()
    );

    let mut update = FieldMetadataUpdate::new("id");
    update.metadata.insert(PK.to_string(), Some("true".into()));
    let install = table.update_field_metadata(&[update]).await;
    println!("install on inferred column -> {:?}", install.as_ref().map(|_| ()).map_err(|e| e.to_string()));

    let table = conn.open_table("t").execute().await.unwrap();
    let schema = table.schema().await.unwrap();
    println!("id metadata after install={:?}", schema.field_with_name("id").unwrap().metadata());

    let barrier = Arc::new(Barrier::new(4));
    let mut tasks = Vec::new();
    for writer in 0..4 {
        let uri = uri.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            let mut store = LanceDBVectorStore::new(uri.clone().into(), "t".into()).await.unwrap();
            barrier.wait().await;
            store
                .upsert(vec![json!({"id": "a", "value": writer})], "id".into())
                .await
                .map_err(|e| e.to_string())
        }));
    }
    for task in tasks {
        println!("store upsert -> {:?}", task.await.unwrap());
    }

    let batches: Vec<RecordBatch> = table
        .query()
        .select(lancedb::query::Select::columns(&["id"]))
        .execute()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let ids: Vec<String> = batches
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .iter()
                .flatten()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    println!("rows={ids:?}");
    let _ = RecordBatchIterator::new(Vec::<Result<RecordBatch, arrow_schema::ArrowError>>::new(), schema);
    let _ = std::fs::remove_dir_all(&dir);
}
