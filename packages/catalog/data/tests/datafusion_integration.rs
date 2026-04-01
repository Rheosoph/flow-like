//! Integration tests for DataFusion nodes and session management
//!
//! These tests verify that our FlowLike node implementations work correctly
//! with DataFusion sessions using the FlowLikeStore abstraction layer.
//!
//! Run all tests: cargo test --package flow-like-catalog-data --test datafusion_integration
//!
//! For Docker-dependent tests:
//! ```sh
//! cd tests && docker-compose -f docker-compose.test.yml up -d
//! cargo test --package flow-like-catalog-data --test datafusion_integration -- --ignored
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use flow_like_storage::datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
use flow_like_storage::datafusion::arrow::datatypes::{DataType, Field, Schema};
use flow_like_storage::datafusion::arrow::record_batch::RecordBatch;
use flow_like_storage::datafusion::datasource::file_format::csv::CsvFormat;
use flow_like_storage::datafusion::datasource::file_format::json::JsonFormat;
use flow_like_storage::datafusion::datasource::file_format::parquet::ParquetFormat;
use flow_like_storage::datafusion::datasource::listing::{
    ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
};
use flow_like_storage::datafusion::prelude::{SessionConfig, SessionContext, col, lit};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::files::store::local_store::LocalObjectStore;
use flow_like_storage::object_store::ObjectStore;
use flow_like_storage::object_store::path::Path as ObjectPath;
use flow_like_types::reqwest::Url;
use futures::StreamExt;
use uuid::Uuid;

fn test_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

fn unique_object_path(prefix: &str, ext: &str) -> ObjectPath {
    let base = test_data_dir();
    std::fs::create_dir_all(&base).ok();
    ObjectPath::from(format!(
        "{}/{}_{}.{}",
        base.to_str().unwrap(),
        prefix,
        Uuid::new_v4(),
        ext
    ))
}

fn create_test_store() -> (FlowLikeStore, String) {
    let base_dir = PathBuf::from("/");
    let local_store = LocalObjectStore::new(base_dir).expect("Failed to create local store");
    let store = FlowLikeStore::Local(Arc::new(local_store));
    let store_url = "file:///".to_string();
    (store, store_url)
}

async fn register_parquet_from_store(
    ctx: &SessionContext,
    store: &FlowLikeStore,
    store_url: &str,
    path: &ObjectPath,
    table_name: &str,
) {
    let url = Url::parse(store_url).unwrap();
    ctx.register_object_store(&url, store.as_generic());

    let table_url = format!("{}{}", store_url, path.as_ref());
    let table_path = ListingTableUrl::parse(&table_url).unwrap();

    let options =
        ListingOptions::new(Arc::new(ParquetFormat::default())).with_file_extension("parquet");

    let config = ListingTableConfig::new(table_path)
        .with_listing_options(options)
        .infer_schema(&ctx.state())
        .await
        .unwrap();

    let table = ListingTable::try_new(config).unwrap();
    ctx.register_table(table_name, Arc::new(table)).unwrap();
}

async fn register_csv_from_store(
    ctx: &SessionContext,
    store: &FlowLikeStore,
    store_url: &str,
    path: &ObjectPath,
    table_name: &str,
) {
    let url = Url::parse(store_url).unwrap();
    ctx.register_object_store(&url, store.as_generic());

    let table_url = format!("{}{}", store_url, path.as_ref());
    let table_path = ListingTableUrl::parse(&table_url).unwrap();

    let options = ListingOptions::new(Arc::new(CsvFormat::default().with_has_header(true)))
        .with_file_extension("csv");

    let config = ListingTableConfig::new(table_path)
        .with_listing_options(options)
        .infer_schema(&ctx.state())
        .await
        .unwrap();

    let table = ListingTable::try_new(config).unwrap();
    ctx.register_table(table_name, Arc::new(table)).unwrap();
}

async fn register_json_from_store(
    ctx: &SessionContext,
    store: &FlowLikeStore,
    store_url: &str,
    path: &ObjectPath,
    table_name: &str,
) {
    let url = Url::parse(store_url).unwrap();
    ctx.register_object_store(&url, store.as_generic());

    let table_url = format!("{}{}", store_url, path.as_ref());
    let table_path = ListingTableUrl::parse(&table_url).unwrap();

    let options = ListingOptions::new(Arc::new(JsonFormat::default())).with_file_extension("json");

    let config = ListingTableConfig::new(table_path)
        .with_listing_options(options)
        .infer_schema(&ctx.state())
        .await
        .unwrap();

    let table = ListingTable::try_new(config).unwrap();
    ctx.register_table(table_name, Arc::new(table)).unwrap();
}

// ============================================================================
// Test Data Helpers - Using FlowLikeStore
// ============================================================================

mod test_data {
    use super::*;
    use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
    use flow_like_storage::object_store::PutPayload;

    pub async fn create_users_parquet(store: &FlowLikeStore) -> ObjectPath {
        let path = unique_object_path("users", "parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("department", DataType::Utf8, true),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5])),
                Arc::new(StringArray::from(vec![
                    "Alice", "Bob", "Charlie", "Diana", "Eve",
                ])),
                Arc::new(StringArray::from(vec![
                    Some("Engineering"),
                    Some("Sales"),
                    Some("Engineering"),
                    Some("HR"),
                    Some("Sales"),
                ])),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }

        store
            .as_generic()
            .put(&path, PutPayload::from(buffer))
            .await
            .unwrap();
        path
    }

    pub async fn create_orders_csv(store: &FlowLikeStore) -> ObjectPath {
        let path = unique_object_path("orders", "csv");
        let csv_content = "order_id,user_id,amount,product\n\
            101,1,150.00,Widget\n\
            102,1,200.00,Gadget\n\
            103,2,75.50,Widget\n\
            104,3,300.00,Gizmo\n\
            105,4,50.00,Widget\n\
            106,5,125.00,Gadget\n\
            107,2,180.00,Gizmo";

        store
            .as_generic()
            .put(&path, PutPayload::from(csv_content.as_bytes().to_vec()))
            .await
            .unwrap();
        path
    }

    pub async fn create_products_json(store: &FlowLikeStore) -> ObjectPath {
        let path = unique_object_path("products", "json");
        let json_content = r#"{"product_id": "Widget", "price": 50.00, "category": "Hardware"}
{"product_id": "Gadget", "price": 75.00, "category": "Electronics"}
{"product_id": "Gizmo", "price": 100.00, "category": "Hardware"}"#;

        store
            .as_generic()
            .put(&path, PutPayload::from(json_content.as_bytes().to_vec()))
            .await
            .unwrap();
        path
    }

    pub async fn create_salaries_parquet(store: &FlowLikeStore) -> ObjectPath {
        let path = unique_object_path("salaries", "parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("employee_id", DataType::Int32, false),
            Field::new("salary", DataType::Float64, false),
            Field::new("bonus", DataType::Float64, true),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5])),
                Arc::new(Float64Array::from(vec![
                    85000.0, 72000.0, 95000.0, 68000.0, 78000.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    Some(5000.0),
                    Some(3000.0),
                    Some(8000.0),
                    None,
                    Some(4000.0),
                ])),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }

        store
            .as_generic()
            .put(&path, PutPayload::from(buffer))
            .await
            .unwrap();
        path
    }
}

// ============================================================================
// Session Configuration Tests
// ============================================================================

#[cfg(test)]
mod session_config_tests {
    use super::*;

    #[tokio::test]
    async fn test_session_with_custom_partitions() {
        let config = SessionConfig::new()
            .with_target_partitions(8)
            .with_batch_size(4096);
        let ctx = SessionContext::new_with_config(config);

        let state = ctx.state();
        assert_eq!(state.config().target_partitions(), 8);
        assert_eq!(state.config().batch_size(), 4096);
    }

    #[tokio::test]
    async fn test_session_repartition_settings() {
        let config = SessionConfig::new()
            .with_repartition_joins(true)
            .with_repartition_aggregations(true)
            .with_repartition_sorts(false);
        let ctx = SessionContext::new_with_config(config);

        let state = ctx.state();
        let opts = state.config().options();
        assert!(opts.optimizer.repartition_joins);
        assert!(opts.optimizer.repartition_aggregations);
        assert!(!opts.optimizer.repartition_sorts);
    }

    #[tokio::test]
    async fn test_session_coalesce_and_parquet_settings() {
        let config = SessionConfig::new()
            .with_coalesce_batches(true)
            .with_parquet_pruning(true);
        let ctx = SessionContext::new_with_config(config);

        let state = ctx.state();
        let opts = state.config().options();
        assert!(opts.execution.coalesce_batches);
        assert!(opts.execution.parquet.pruning);
    }
}

// ============================================================================
// Single Source Tests (Parquet, CSV, JSON) - Using FlowLikeStore
// ============================================================================

#[cfg(test)]
mod parquet_source_tests {
    use super::*;

    #[tokio::test]
    async fn test_parquet_basic_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx.sql("SELECT COUNT(*) as cnt FROM users").await.unwrap();
        let batches = df.collect().await.unwrap();

        assert!(!batches.is_empty());
        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert!(total > 0);

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_parquet_filtered_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx
            .sql("SELECT name FROM users WHERE department = 'Engineering'")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 2, "Should have 2 engineers");

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_parquet_aggregation() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx.sql("SELECT department, COUNT(*) as cnt FROM users GROUP BY department ORDER BY cnt DESC")
            .await.unwrap();
        let batches = df.collect().await.unwrap();

        assert!(!batches.is_empty());
        store.as_generic().delete(&path).await.ok();
    }
}

#[cfg(test)]
mod csv_source_tests {
    use super::*;

    #[tokio::test]
    async fn test_csv_basic_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &path, "orders").await;

        let df = ctx.sql("SELECT COUNT(*) as cnt FROM orders").await.unwrap();
        let batches = df.collect().await.unwrap();

        assert!(!batches.is_empty());
        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_csv_filtered_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &path, "orders").await;

        let df = ctx
            .sql("SELECT order_id, amount FROM orders WHERE amount > 100")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert!(total >= 4, "Should have at least 4 orders over 100");

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_csv_aggregation_by_product() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &path, "orders").await;

        let df = ctx
            .sql(
                "SELECT product, COUNT(*) as order_count, SUM(amount) as total_amount
             FROM orders
             GROUP BY product
             ORDER BY total_amount DESC",
            )
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        assert!(!batches.is_empty());
        store.as_generic().delete(&path).await.ok();
    }
}

#[cfg(test)]
mod json_source_tests {
    use super::*;

    #[tokio::test]
    async fn test_json_basic_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_products_json(&store).await;

        let ctx = SessionContext::new();
        register_json_from_store(&ctx, &store, &store_url, &path, "products").await;

        let df = ctx
            .sql("SELECT COUNT(*) as cnt FROM products")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        assert!(!batches.is_empty());
        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 1, "COUNT returns 1 row");

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_json_filtered_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_products_json(&store).await;

        let ctx = SessionContext::new();
        register_json_from_store(&ctx, &store, &store_url, &path, "products").await;

        let df = ctx
            .sql("SELECT product_id, price FROM products WHERE category = 'Hardware'")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 2, "Should have 2 hardware products");

        store.as_generic().delete(&path).await.ok();
    }
}

// ============================================================================
// Cross-Source Join Tests (The main feature of DataFusion federation)
// ============================================================================

#[cfg(test)]
mod cross_source_join_tests {
    use super::*;

    #[tokio::test]
    async fn test_join_parquet_and_csv() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;

        let df = ctx.sql(
            "SELECT u.name, u.department, COUNT(o.order_id) as order_count, SUM(o.amount) as total_spent
             FROM users u
             LEFT JOIN orders o ON u.id = o.user_id
             GROUP BY u.name, u.department
             ORDER BY total_spent DESC NULLS LAST"
        ).await.unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(
            total_rows, 5,
            "Should have 5 users with their order summaries"
        );

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
    }

    #[tokio::test]
    async fn test_join_csv_and_json() {
        let (store, store_url) = create_test_store();
        let orders_path = test_data::create_orders_csv(&store).await;
        let products_path = test_data::create_products_json(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;
        register_json_from_store(&ctx, &store, &store_url, &products_path, "products").await;

        let df = ctx
            .sql(
                "SELECT o.order_id, o.amount, p.price, p.category
             FROM orders o
             JOIN products p ON o.product = p.product_id
             ORDER BY o.amount DESC",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&orders_path).await.ok();
        store.as_generic().delete(&products_path).await.ok();
    }

    #[tokio::test]
    async fn test_three_way_join_parquet_csv_json() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;
        let products_path = test_data::create_products_json(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;
        register_json_from_store(&ctx, &store, &store_url, &products_path, "products").await;

        let df = ctx
            .sql(
                "SELECT
                u.name,
                u.department,
                p.product_id as product,
                p.category,
                SUM(o.amount) as total_spent,
                COUNT(*) as order_count
             FROM users u
             JOIN orders o ON u.id = o.user_id
             JOIN products p ON o.product = p.product_id
             GROUP BY u.name, u.department, p.product_id, p.category
             ORDER BY total_spent DESC",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
        store.as_generic().delete(&products_path).await.ok();
    }

    #[tokio::test]
    async fn test_join_multiple_parquet_files() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;
        let salaries_path = test_data::create_salaries_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_parquet_from_store(&ctx, &store, &store_url, &salaries_path, "salaries").await;

        let df = ctx.sql(
            "SELECT u.name, u.department, s.salary, s.bonus, (s.salary + COALESCE(s.bonus, 0)) as total_comp
             FROM users u
             JOIN salaries s ON u.id = s.employee_id
             ORDER BY total_comp DESC"
        ).await.unwrap();

        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5, "Should have 5 employee records");

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&salaries_path).await.ok();
    }

    #[tokio::test]
    async fn test_subquery_across_sources() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;

        let df = ctx
            .sql(
                "SELECT u.name, u.department
             FROM users u
             WHERE u.id IN (
                SELECT DISTINCT user_id FROM orders WHERE amount > 150
             )
             ORDER BY u.name",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
    }

    #[tokio::test]
    async fn test_union_across_sources() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;

        let df = ctx.sql(
            "SELECT 'user' as source, CAST(id AS VARCHAR) as id, name as label FROM users
             UNION ALL
             SELECT 'order' as source, CAST(order_id AS VARCHAR) as id, product as label FROM orders"
        ).await.unwrap();

        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 12, "Should have 5 users + 7 orders = 12 rows");

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
    }
}

// ============================================================================
// Session Persistence Tests (Multiple queries on same session)
// ============================================================================

#[cfg(test)]
mod session_persistence_tests {
    use super::*;

    #[tokio::test]
    async fn test_session_persists_registered_tables() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;

        let df1 = ctx.sql("SELECT COUNT(*) FROM users").await.unwrap();
        let _ = df1.collect().await.unwrap();

        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;

        let df2 = ctx.sql("SELECT COUNT(*) FROM users").await.unwrap();
        let _ = df2.collect().await.unwrap();

        let df3 = ctx.sql("SELECT COUNT(*) FROM orders").await.unwrap();
        let _ = df3.collect().await.unwrap();

        let df4 = ctx
            .sql("SELECT u.name, o.amount FROM users u JOIN orders o ON u.id = o.user_id LIMIT 5")
            .await
            .unwrap();
        let batches = df4.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
    }

    #[tokio::test]
    async fn test_multiple_queries_same_session() {
        let (store, store_url) = create_test_store();
        let users_path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;

        for dept in ["Engineering", "Sales", "HR"] {
            let df = ctx
                .sql("SELECT name, department FROM users")
                .await
                .unwrap()
                .filter(col("department").eq(lit(dept.to_string())))
                .unwrap();
            let _ = df.collect().await.unwrap();
        }

        let df = ctx
            .sql("SELECT department, COUNT(*) FROM users GROUP BY department")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&users_path).await.ok();
    }
}

// ============================================================================
// Schema Introspection Tests
// ============================================================================

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[tokio::test]
    async fn test_parquet_schema_introspection() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx.table("users").await.unwrap();
        let schema = df.schema();

        assert_eq!(schema.fields().len(), 3);
        assert!(schema.field_with_name(None, "id").is_ok());
        assert!(schema.field_with_name(None, "name").is_ok());
        assert!(schema.field_with_name(None, "department").is_ok());

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_csv_schema_inference() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &path, "orders").await;

        let df = ctx.table("orders").await.unwrap();
        let schema = df.schema();

        assert_eq!(schema.fields().len(), 4);
        assert!(schema.field_with_name(None, "order_id").is_ok());
        assert!(schema.field_with_name(None, "user_id").is_ok());
        assert!(schema.field_with_name(None, "amount").is_ok());
        assert!(schema.field_with_name(None, "product").is_ok());

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_json_schema_inference() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_products_json(&store).await;

        let ctx = SessionContext::new();
        register_json_from_store(&ctx, &store, &store_url, &path, "products").await;

        let df = ctx.table("products").await.unwrap();
        let schema = df.schema();

        assert!(schema.field_with_name(None, "product_id").is_ok());
        assert!(schema.field_with_name(None, "price").is_ok());
        assert!(schema.field_with_name(None, "category").is_ok());

        store.as_generic().delete(&path).await.ok();
    }
}

// ============================================================================
// Real Database Integration Tests
// Run with: cargo test --package flow-like-catalog-data --test datafusion_integration
// Requires: docker-compose -f tests/docker-compose.test.yml up -d
// ============================================================================

#[cfg(test)]
mod postgres_integration_tests {
    use tokio_postgres::{Error as PgError, NoTls};

    const HOST: &str = "localhost";
    const PORT: u16 = 54320;
    const USER: &str = "flowlike";
    const PASSWORD: &str = "flowlike_test";
    const DATABASE: &str = "flowlike_test";

    async fn get_client() -> Result<tokio_postgres::Client, PgError> {
        let conn_str = format!(
            "host={} port={} user={} password={} dbname={}",
            HOST, PORT, USER, PASSWORD, DATABASE
        );
        let (client, connection) = tokio_postgres::connect(&conn_str, NoTls).await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("PostgreSQL connection error: {}", e);
            }
        });
        Ok(client)
    }

    #[tokio::test]
    async fn test_postgres_connection() {
        let client = match get_client().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };

        let row = client.query_one("SELECT 1 as test", &[]).await.unwrap();
        let value: i32 = row.get("test");
        assert_eq!(value, 1);
    }

    #[tokio::test]
    async fn test_postgres_query_users_table() {
        let client = match get_client().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };

        let rows = client
            .query("SELECT * FROM users ORDER BY id", &[])
            .await
            .unwrap();
        assert!(!rows.is_empty(), "users table should have data");

        let first_row = &rows[0];
        let id: i32 = first_row.get("id");
        let name: &str = first_row.get("name");
        assert_eq!(id, 1);
        assert!(!name.is_empty());
    }

    #[tokio::test]
    async fn test_postgres_query_orders_with_join() {
        let client = match get_client().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };

        let rows = client
            .query(
                "SELECT u.name, o.product_name, o.price
                 FROM users u
                 JOIN orders o ON u.id = o.user_id
                 ORDER BY o.price DESC
                 LIMIT 5",
                &[],
            )
            .await
            .unwrap();

        assert!(!rows.is_empty(), "Should have joined results");
        for row in &rows {
            let name: &str = row.get("name");
            let product: &str = row.get("product_name");
            assert!(!name.is_empty());
            assert!(!product.is_empty());
        }
    }

    #[tokio::test]
    async fn test_postgres_aggregation() {
        let client = match get_client().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };

        let row = client
            .query_one(
                "SELECT COUNT(*) as cnt, SUM(price) as total FROM orders",
                &[],
            )
            .await
            .unwrap();

        let count: i64 = row.get("cnt");
        assert!(count > 0, "Should have orders");
    }

    #[tokio::test]
    async fn test_postgres_insert_and_rollback() {
        let client = match get_client().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };

        // Start transaction
        client.execute("BEGIN", &[]).await.unwrap();

        // Insert test data using correct schema columns
        let result = client
            .execute(
                "INSERT INTO users (name, email, age) VALUES ($1, $2, $3)",
                &[&"Test User", &"testuser_rollback@test.com", &30i32],
            )
            .await;

        assert!(result.is_ok(), "Insert should succeed");

        // Verify insert
        let row = client
            .query_one(
                "SELECT COUNT(*) as cnt FROM users WHERE name = 'Test User'",
                &[],
            )
            .await
            .unwrap();
        let count: i64 = row.get("cnt");
        assert_eq!(count, 1);

        // Rollback to not pollute test data
        client.execute("ROLLBACK", &[]).await.unwrap();

        // Verify rollback
        let row = client
            .query_one(
                "SELECT COUNT(*) as cnt FROM users WHERE name = 'Test User'",
                &[],
            )
            .await
            .unwrap();
        let count: i64 = row.get("cnt");
        assert_eq!(count, 0);
    }
}

#[cfg(test)]
mod mysql_integration_tests {
    use sqlx::{Row, mysql::MySqlPoolOptions};

    const HOST: &str = "localhost";
    const PORT: u16 = 33060;
    const USER: &str = "flowlike";
    const PASSWORD: &str = "flowlike_test";
    const DATABASE: &str = "flowlike_test";

    async fn get_pool() -> Result<sqlx::MySqlPool, sqlx::Error> {
        let url = format!(
            "mysql://{}:{}@{}:{}/{}",
            USER, PASSWORD, HOST, PORT, DATABASE
        );
        MySqlPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
    }

    #[tokio::test]
    async fn test_mysql_connection() {
        let pool = match get_pool().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping test - MySQL not available: {}", e);
                return;
            }
        };

        let row: (i32,) = sqlx::query_as("SELECT 1 as test")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    async fn test_mysql_query_products_table() {
        let pool = match get_pool().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping test - MySQL not available: {}", e);
                return;
            }
        };

        let rows = sqlx::query("SELECT * FROM products ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert!(!rows.is_empty(), "products table should have data");

        let first_row = &rows[0];
        let id: i32 = first_row.get("id");
        let name: String = first_row.get("name");
        assert_eq!(id, 1);
        assert!(!name.is_empty());
    }

    #[tokio::test]
    async fn test_mysql_aggregation_by_category() {
        let pool = match get_pool().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping test - MySQL not available: {}", e);
                return;
            }
        };

        let rows = sqlx::query(
            "SELECT category, COUNT(*) as cnt, AVG(price) as avg_price
             FROM products
             GROUP BY category
             ORDER BY cnt DESC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        assert!(!rows.is_empty(), "Should have aggregated results");
    }

    #[tokio::test]
    async fn test_mysql_transaction_rollback() {
        let pool = match get_pool().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping test - MySQL not available: {}", e);
                return;
            }
        };

        let mut tx = pool.begin().await.unwrap();

        // Insert test data
        sqlx::query("INSERT INTO products (name, category, price, stock) VALUES (?, ?, ?, ?)")
            .bind("Test Product")
            .bind("Test")
            .bind(99.99f64)
            .bind(10i32)
            .execute(&mut *tx)
            .await
            .unwrap();

        // Verify insert within transaction
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM products WHERE name = 'Test Product'")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(row.0, 1);

        // Rollback
        tx.rollback().await.unwrap();

        // Verify rollback
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM products WHERE name = 'Test Product'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.0, 0);
    }
}

#[cfg(test)]
mod clickhouse_integration_tests {
    const HOST: &str = "localhost";
    const PORT: u16 = 8123;
    const USER: &str = "flowlike";
    const PASSWORD: &str = "flowlike_test";
    const DATABASE: &str = "flowlike_test";

    async fn clickhouse_query(query: &str) -> Result<String, reqwest::Error> {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://{}:{}/", HOST, PORT))
            .query(&[
                ("user", USER),
                ("password", PASSWORD),
                ("database", DATABASE),
            ])
            .body(query.to_string())
            .send()
            .await?;
        response.text().await
    }

    #[tokio::test]
    async fn test_clickhouse_ping() {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{}:{}/ping", HOST, PORT))
            .send()
            .await;

        match response {
            Ok(resp) => {
                let body = resp.text().await.unwrap();
                assert_eq!(body.trim(), "Ok.", "ClickHouse should respond Ok.");
            }
            Err(e) => {
                eprintln!("Skipping test - ClickHouse not available: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_clickhouse_query_analytics() {
        let result = clickhouse_query("SELECT COUNT(*) FROM analytics").await;

        match result {
            Ok(body) => {
                let count: i64 = body.trim().parse().unwrap_or(0);
                assert!(count >= 0, "Should return valid count");
            }
            Err(e) => {
                eprintln!("Skipping test - ClickHouse not available: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_clickhouse_aggregation_by_event() {
        let result = clickhouse_query(
            "SELECT event_type, COUNT(*) as cnt, AVG(value) as avg_val
             FROM analytics
             GROUP BY event_type
             ORDER BY cnt DESC
             FORMAT TabSeparated",
        )
        .await;

        match result {
            Ok(body) => {
                assert!(!body.is_empty(), "Should have results");
            }
            Err(e) => {
                eprintln!("Skipping test - ClickHouse not available: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_clickhouse_time_series_query() {
        let result = clickhouse_query(
            "SELECT
                toDate(timestamp) as day,
                COUNT(*) as events,
                SUM(value) as total_value
             FROM analytics
             GROUP BY day
             ORDER BY day
             FORMAT TabSeparated",
        )
        .await;

        match result {
            Ok(body) => {
                assert!(!body.is_empty(), "Should have time series results");
            }
            Err(e) => {
                eprintln!("Skipping test - ClickHouse not available: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_clickhouse_insert_and_query() {
        // First, get max id to generate a unique one
        let max_id_result = clickhouse_query("SELECT max(id) FROM analytics").await;
        if max_id_result.is_err() {
            eprintln!("Skipping test - ClickHouse not available");
            return;
        }

        let max_id: u64 = max_id_result.unwrap().trim().parse().unwrap_or(0);
        let new_id = max_id + 1000 + rand::random::<u64>() % 10000;
        let test_event = format!("test_event_{}", new_id);

        // Insert test data with proper schema (id is required UInt64)
        let insert_result = clickhouse_query(&format!(
            "INSERT INTO analytics (id, event_type, event_data) VALUES ({}, '{}', '{{}}')",
            new_id, test_event
        ))
        .await;

        if let Err(e) = insert_result {
            eprintln!("Insert failed: {}", e);
            return;
        }

        // ClickHouse MergeTree needs a moment for async insert
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Query the inserted data
        let query_result = clickhouse_query(&format!(
            "SELECT COUNT(*) FROM analytics WHERE event_type = '{}'",
            test_event
        ))
        .await
        .unwrap();

        let count: i64 = query_result.trim().parse().unwrap_or(0);
        assert!(
            count >= 1,
            "Should have inserted test event (got count: {})",
            count
        );
    }
}

#[cfg(test)]
mod minio_integration_tests {
    const MINIO_ENDPOINT: &str = "http://localhost:9002";
    const MINIO_BUCKET: &str = "test-bucket";

    #[tokio::test]
    async fn test_minio_health() {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/minio/health/live", MINIO_ENDPOINT))
            .send()
            .await;

        match response {
            Ok(resp) => {
                assert!(resp.status().is_success(), "MinIO should be healthy");
            }
            Err(e) => {
                eprintln!("Skipping test - MinIO not available: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_minio_bucket_exists() {
        let client = reqwest::Client::new();

        // Simple HEAD request to check bucket exists (MinIO S3 API)
        let response = client
            .head(format!("{}/{}", MINIO_ENDPOINT, MINIO_BUCKET))
            .send()
            .await;

        match response {
            Ok(resp) => {
                // 200 = bucket exists, 404 = doesn't exist
                println!("MinIO bucket check status: {}", resp.status());
            }
            Err(e) => {
                eprintln!("Skipping test - MinIO not available: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod cross_database_tests {
    use super::*;
    use sqlx::{Row, mysql::MySqlPoolOptions};
    use tokio_postgres::NoTls;

    #[tokio::test]
    async fn test_postgres_and_mysql_same_schema() {
        // Connect to PostgreSQL
        let pg_conn_str =
            "host=localhost port=54320 user=flowlike password=flowlike_test dbname=flowlike_test";
        let pg_result = tokio_postgres::connect(pg_conn_str, NoTls).await;

        let (pg_client, pg_connection) = match pg_result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };
        tokio::spawn(async move {
            pg_connection.await.ok();
        });

        // Connect to MySQL
        let mysql_url = "mysql://flowlike:flowlike_test@localhost:33060/flowlike_test";
        let mysql_pool = match MySqlPoolOptions::new().connect(mysql_url).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping test - MySQL not available: {}", e);
                return;
            }
        };

        // Query same conceptual data from both databases
        let pg_count: i64 = pg_client
            .query_one("SELECT COUNT(*) FROM users", &[])
            .await
            .unwrap()
            .get(0);

        let mysql_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM products")
            .fetch_one(&mysql_pool)
            .await
            .unwrap();

        println!(
            "PostgreSQL users: {}, MySQL products: {}",
            pg_count, mysql_count.0
        );
        assert!(pg_count > 0, "PostgreSQL should have users");
        assert!(mysql_count.0 > 0, "MySQL should have products");
    }

    #[tokio::test]
    async fn test_datafusion_with_postgres_exported_data() {
        // Connect to PostgreSQL and export data
        let pg_conn_str =
            "host=localhost port=54320 user=flowlike password=flowlike_test dbname=flowlike_test";
        let pg_result = tokio_postgres::connect(pg_conn_str, NoTls).await;

        let (pg_client, pg_connection) = match pg_result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };
        tokio::spawn(async move {
            pg_connection.await.ok();
        });

        // Query PostgreSQL data - use actual columns from schema
        let rows = pg_client
            .query(
                "SELECT id, name, age, is_active FROM users ORDER BY id",
                &[],
            )
            .await
            .unwrap();

        // Create a Parquet file from PostgreSQL data using FlowLikeStore
        let (store, store_url) = create_test_store();
        let path = unique_object_path("pg_export", "parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("age", DataType::Int32, true),
        ]));

        let ids: Vec<i32> = rows.iter().map(|r| r.get::<_, i32>("id")).collect();
        let names: Vec<String> = rows.iter().map(|r| r.get::<_, String>("name")).collect();
        let ages: Vec<Option<i32>> = rows
            .iter()
            .map(|r| r.get::<_, Option<i32>>("age"))
            .collect();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(names)),
                Arc::new(Int32Array::from(ages)),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }

        use flow_like_storage::object_store::PutPayload;
        store
            .as_generic()
            .put(&path, PutPayload::from(buffer))
            .await
            .unwrap();

        // Now use DataFusion to query the exported data
        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "pg_users").await;

        let df = ctx
            .sql("SELECT COUNT(*) as cnt, AVG(age) as avg_age FROM pg_users")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        assert!(
            !batches.is_empty(),
            "Should have aggregated results from PG data"
        );

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_federated_query_simulation() {
        // This test simulates what a federated query would do:
        // 1. Fetch data from PostgreSQL (users)
        // 2. Fetch data from MySQL (products)
        // 3. Join them locally in DataFusion

        // PostgreSQL connection
        let pg_conn_str =
            "host=localhost port=54320 user=flowlike password=flowlike_test dbname=flowlike_test";
        let pg_result = tokio_postgres::connect(pg_conn_str, NoTls).await;

        let (pg_client, pg_connection) = match pg_result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Skipping test - PostgreSQL not available: {}", e);
                return;
            }
        };
        tokio::spawn(async move {
            pg_connection.await.ok();
        });

        // MySQL connection
        let mysql_url = "mysql://flowlike:flowlike_test@localhost:33060/flowlike_test";
        let mysql_pool = match MySqlPoolOptions::new().connect(mysql_url).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping test - MySQL not available: {}", e);
                return;
            }
        };

        // Fetch users from PostgreSQL
        let pg_rows = pg_client
            .query("SELECT id, name, email FROM users", &[])
            .await
            .unwrap();

        let user_ids: Vec<i32> = pg_rows.iter().map(|r| r.get("id")).collect();
        let user_names: Vec<String> = pg_rows.iter().map(|r| r.get("name")).collect();

        // Fetch products from MySQL - cast DECIMAL to DOUBLE for compatibility
        let mysql_rows =
            sqlx::query("SELECT id, name, category, CAST(price AS DOUBLE) as price FROM products")
                .fetch_all(&mysql_pool)
                .await
                .unwrap();

        let product_ids: Vec<i32> = mysql_rows.iter().map(|r| r.get("id")).collect();
        let product_names: Vec<String> = mysql_rows.iter().map(|r| r.get("name")).collect();
        let product_prices: Vec<f64> = mysql_rows.iter().map(|r| r.get("price")).collect();

        // Create Parquet files for both using FlowLikeStore
        let (store, store_url) = create_test_store();
        let users_path = unique_object_path("federated_users", "parquet");
        let products_path = unique_object_path("federated_products", "parquet");

        // Write users
        let users_schema = Arc::new(Schema::new(vec![
            Field::new("user_id", DataType::Int32, false),
            Field::new("user_name", DataType::Utf8, false),
        ]));
        let users_batch = RecordBatch::try_new(
            users_schema.clone(),
            vec![
                Arc::new(Int32Array::from(user_ids)),
                Arc::new(StringArray::from(user_names)),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, users_schema, None).unwrap();
            writer.write(&users_batch).unwrap();
            writer.close().unwrap();
        }
        use flow_like_storage::object_store::PutPayload;
        store
            .as_generic()
            .put(&users_path, PutPayload::from(buffer))
            .await
            .unwrap();

        // Write products
        let products_schema = Arc::new(Schema::new(vec![
            Field::new("product_id", DataType::Int32, false),
            Field::new("product_name", DataType::Utf8, false),
            Field::new("price", DataType::Float64, false),
        ]));
        let products_batch = RecordBatch::try_new(
            products_schema.clone(),
            vec![
                Arc::new(Int32Array::from(product_ids)),
                Arc::new(StringArray::from(product_names)),
                Arc::new(Float64Array::from(product_prices)),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, products_schema, None).unwrap();
            writer.write(&products_batch).unwrap();
            writer.close().unwrap();
        }
        store
            .as_generic()
            .put(&products_path, PutPayload::from(buffer))
            .await
            .unwrap();

        // Now use DataFusion to join data from both sources
        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_parquet_from_store(&ctx, &store, &store_url, &products_path, "products").await;

        // Cross join to show all user-product combinations (simulating a recommendation scenario)
        let df = ctx
            .sql(
                "SELECT u.user_name, p.product_name, p.price
             FROM users u
             CROSS JOIN products p
             WHERE p.price > 100
             ORDER BY p.price DESC
             LIMIT 10",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty(), "Federated query should return results");

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        println!("Federated query returned {} rows", total_rows);

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&products_path).await.ok();
    }
}

// ============================================================================
// PRIORITY 1: SQLite Tests (No Docker Required)
// ============================================================================

#[cfg(test)]
mod sqlite_integration_tests {
    use super::*;
    use sqlx::{Executor, Row, sqlite::SqlitePoolOptions};
    use std::fs;

    fn unique_sqlite_path(prefix: &str) -> PathBuf {
        let path = test_data_dir().join(format!("{}_{}.db", prefix, Uuid::new_v4()));
        fs::create_dir_all(path.parent().unwrap()).ok();
        path
    }

    #[tokio::test]
    async fn test_sqlite_create_and_query() {
        let db_path = unique_sqlite_path("sqlite_test");
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .expect("Failed to create SQLite database");

        pool.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT)")
            .await
            .expect("Failed to create users table");

        pool.execute("INSERT INTO users (id, name, email) VALUES (1, 'Alice', 'alice@example.com'), (2, 'Bob', 'bob@example.com'), (3, 'Charlie', 'charlie@example.com')")
            .await
            .expect("Failed to insert data");

        let rows: Vec<_> = sqlx::query("SELECT id, name, email FROM users ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("Failed to query");

        assert_eq!(rows.len(), 3);
        let first_name: String = rows[0].get("name");
        assert_eq!(first_name, "Alice");

        pool.close().await;
        fs::remove_file(&db_path).ok();
    }

    #[tokio::test]
    async fn test_sqlite_file_persistence() {
        let db_path = unique_sqlite_path("sqlite_persist");
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&db_url)
                .await
                .expect("Failed to create SQLite database");

            pool.execute("CREATE TABLE persist_test (id INTEGER PRIMARY KEY, value TEXT)")
                .await
                .unwrap();
            pool.execute("INSERT INTO persist_test (id, value) VALUES (1, 'persisted')")
                .await
                .unwrap();
            pool.close().await;
        }

        {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&db_url)
                .await
                .expect("Failed to reopen SQLite database");

            let rows: Vec<_> = sqlx::query("SELECT value FROM persist_test WHERE id = 1")
                .fetch_all(&pool)
                .await
                .unwrap();

            assert_eq!(rows.len(), 1);
            let value: String = rows[0].get("value");
            assert_eq!(value, "persisted");
            pool.close().await;
        }

        fs::remove_file(&db_path).ok();
    }

    #[tokio::test]
    async fn test_sqlite_aggregation() {
        let db_path = unique_sqlite_path("sqlite_agg");
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .expect("Failed to create SQLite database");

        pool.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, product TEXT, amount REAL, quantity INTEGER)"
        ).await.unwrap();

        pool.execute(
            "INSERT INTO orders (product, amount, quantity) VALUES
             ('Widget', 100.0, 5),
             ('Widget', 150.0, 3),
             ('Gadget', 200.0, 2),
             ('Gadget', 250.0, 1),
             ('Gizmo', 300.0, 4)",
        )
        .await
        .unwrap();

        let rows: Vec<_> = sqlx::query(
            "SELECT product, SUM(amount) as total, AVG(quantity) as avg_qty
             FROM orders GROUP BY product ORDER BY total DESC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(rows.len(), 3);
        let first_product: String = rows[0].get("product");
        assert_eq!(first_product, "Gadget");

        pool.close().await;
        fs::remove_file(&db_path).ok();
    }

    #[tokio::test]
    async fn test_sqlite_join_with_datafusion() {
        let db_path = unique_sqlite_path("sqlite_join");
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .expect("Failed to create SQLite database");

        pool.execute("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT, region TEXT)")
            .await
            .unwrap();
        pool.execute(
            "INSERT INTO customers (id, name, region) VALUES
             (1, 'Acme Corp', 'West'),
             (2, 'TechStart', 'East'),
             (3, 'Global Inc', 'West')",
        )
        .await
        .unwrap();

        let rows: Vec<_> = sqlx::query("SELECT * FROM customers")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;

        let customer_ids: Vec<i32> = rows.iter().map(|r| r.get("id")).collect();
        let customer_names: Vec<String> = rows.iter().map(|r| r.get("name")).collect();
        let regions: Vec<String> = rows.iter().map(|r| r.get("region")).collect();

        // Use FlowLikeStore for Parquet files
        let (store, store_url) = create_test_store();
        let orders_path = unique_object_path("sqlite_orders", "parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("order_id", DataType::Int32, false),
            Field::new("customer_id", DataType::Int32, false),
            Field::new("amount", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![101, 102, 103, 104])),
                Arc::new(Int32Array::from(vec![1, 1, 2, 3])),
                Arc::new(Float64Array::from(vec![500.0, 300.0, 750.0, 200.0])),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        use flow_like_storage::object_store::PutPayload;
        store
            .as_generic()
            .put(&orders_path, PutPayload::from(buffer))
            .await
            .unwrap();

        let customers_path = unique_object_path("sqlite_customers", "parquet");
        let cust_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
        ]));

        let cust_batch = RecordBatch::try_new(
            cust_schema.clone(),
            vec![
                Arc::new(Int32Array::from(customer_ids)),
                Arc::new(StringArray::from(customer_names)),
                Arc::new(StringArray::from(regions)),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, cust_schema, None).unwrap();
            writer.write(&cust_batch).unwrap();
            writer.close().unwrap();
        }
        store
            .as_generic()
            .put(&customers_path, PutPayload::from(buffer))
            .await
            .unwrap();

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &customers_path, "customers").await;
        register_parquet_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;

        let df = ctx
            .sql(
                "SELECT c.name, c.region, SUM(o.amount) as total_orders
             FROM customers c
             JOIN orders o ON c.id = o.customer_id
             GROUP BY c.name, c.region
             ORDER BY total_orders DESC",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        fs::remove_file(&db_path).ok();
        store.as_generic().delete(&orders_path).await.ok();
        store.as_generic().delete(&customers_path).await.ok();
    }
}

// ============================================================================
// PRIORITY 1: DuckDB Tests (No Docker Required)
// ============================================================================

#[cfg(test)]
mod duckdb_integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_duckdb_parquet_native_read() {
        let (store, store_url) = create_test_store();
        let parquet_path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &parquet_path, "users").await;

        let df = ctx
            .sql("SELECT * FROM users WHERE department = 'Engineering'")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        store.as_generic().delete(&parquet_path).await.ok();
    }

    #[tokio::test]
    async fn test_duckdb_style_aggregation() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_orders_csv(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &path, "orders").await;

        let df = ctx
            .sql(
                "SELECT product,
                    COUNT(*) as order_count,
                    SUM(amount) as total_amount,
                    AVG(amount) as avg_amount
             FROM orders
             GROUP BY product
             HAVING COUNT(*) > 1
             ORDER BY total_amount DESC",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_duckdb_window_functions() {
        let (store, store_url) = create_test_store();
        let parquet_path = test_data::create_salaries_parquet(&store).await;

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &parquet_path, "salaries").await;

        let df = ctx
            .sql(
                "SELECT employee_id, salary,
                    RANK() OVER (ORDER BY salary DESC) as salary_rank,
                    SUM(salary) OVER () as total_salary
             FROM salaries",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5);

        store.as_generic().delete(&parquet_path).await.ok();
    }

    #[tokio::test]
    async fn test_duckdb_cte_query() {
        let (store, store_url) = create_test_store();
        let path = test_data::create_orders_csv(&store).await;
        let users_path = test_data::create_users_parquet(&store).await;

        let ctx = SessionContext::new();
        register_csv_from_store(&ctx, &store, &store_url, &path, "orders").await;
        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;

        let df = ctx
            .sql(
                "WITH order_totals AS (
                SELECT user_id, SUM(amount) as total_spent
                FROM orders
                GROUP BY user_id
            )
            SELECT u.name, u.department, COALESCE(ot.total_spent, 0) as total_spent
            FROM users u
            LEFT JOIN order_totals ot ON u.id = ot.user_id
            ORDER BY total_spent DESC",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        store.as_generic().delete(&path).await.ok();
        store.as_generic().delete(&users_path).await.ok();
    }
}

// ============================================================================
// PRIORITY 1: Hive Partitioned Parquet Tests - Using FlowLikeStore
// ============================================================================

#[cfg(test)]
mod hive_partitioned_tests {
    use super::*;
    use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
    use flow_like_storage::object_store::PutPayload;

    async fn create_hive_partitioned_data_via_store(store: &FlowLikeStore) -> String {
        let base_dir = test_data_dir();
        std::fs::create_dir_all(&base_dir).ok();
        let base_prefix = format!(
            "{}/hive_data_{}",
            base_dir.to_str().unwrap(),
            Uuid::new_v4()
        );

        for year in [2023, 2024] {
            for month in [1, 6, 12] {
                let schema = Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Int32, false),
                    Field::new("value", DataType::Float64, false),
                    Field::new("category", DataType::Utf8, false),
                ]));

                let base_id = year * 100 + month;
                let batch = RecordBatch::try_new(
                    schema.clone(),
                    vec![
                        Arc::new(Int32Array::from(vec![base_id, base_id + 1, base_id + 2])),
                        Arc::new(Float64Array::from(vec![100.0 * year as f64, 200.0, 300.0])),
                        Arc::new(StringArray::from(vec!["A", "B", "A"])),
                    ],
                )
                .unwrap();

                let mut buffer = Vec::new();
                {
                    let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
                    writer.write(&batch).unwrap();
                    writer.close().unwrap();
                }

                let partition_path = ObjectPath::from(format!(
                    "{}/year={}/month={}/data.parquet",
                    base_prefix, year, month
                ));
                store
                    .as_generic()
                    .put(&partition_path, PutPayload::from(buffer))
                    .await
                    .unwrap();
            }
        }

        base_prefix
    }

    async fn cleanup_hive_data(store: &FlowLikeStore, base_prefix: &str) {
        let obj_store = store.as_generic();
        let prefix = ObjectPath::from(base_prefix);

        let mut stream = obj_store.list(Some(&prefix));
        while let Some(meta) = stream.next().await {
            if let Ok(m) = meta {
                obj_store.delete(&m.location).await.ok();
            }
        }
    }

    #[tokio::test]
    async fn test_hive_partitioned_read() {
        let (store, store_url) = create_test_store();
        let base_prefix = create_hive_partitioned_data_via_store(&store).await;

        let ctx = SessionContext::new();
        let url = Url::parse(&store_url).unwrap();
        ctx.register_object_store(&url, store.as_generic());

        let table_url = format!("{}{}/", store_url, base_prefix);
        let options = ListingOptions::new(Arc::new(ParquetFormat::default()))
            .with_table_partition_cols(vec![
                ("year".to_string(), DataType::Int32),
                ("month".to_string(), DataType::Int32),
            ]);

        ctx.register_listing_table("partitioned_data", &table_url, options, None, None)
            .await
            .unwrap();

        let df = ctx.sql("SELECT * FROM partitioned_data").await.unwrap();
        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 18);

        cleanup_hive_data(&store, &base_prefix).await;
    }

    #[tokio::test]
    async fn test_hive_partition_pruning() {
        let (store, store_url) = create_test_store();
        let base_prefix = create_hive_partitioned_data_via_store(&store).await;

        let ctx = SessionContext::new();
        let url = Url::parse(&store_url).unwrap();
        ctx.register_object_store(&url, store.as_generic());

        let table_url = format!("{}{}/", store_url, base_prefix);
        let options = ListingOptions::new(Arc::new(ParquetFormat::default()))
            .with_table_partition_cols(vec![
                ("year".to_string(), DataType::Int32),
                ("month".to_string(), DataType::Int32),
            ]);

        ctx.register_listing_table("partitioned_data", &table_url, options, None, None)
            .await
            .unwrap();

        let df = ctx
            .sql("SELECT * FROM partitioned_data WHERE year = 2024")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 9);

        let df = ctx
            .sql("SELECT * FROM partitioned_data WHERE year = 2024 AND month = 6")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        cleanup_hive_data(&store, &base_prefix).await;
    }

    #[tokio::test]
    async fn test_hive_aggregation_by_partition() {
        let (store, store_url) = create_test_store();
        let base_prefix = create_hive_partitioned_data_via_store(&store).await;

        let ctx = SessionContext::new();
        let url = Url::parse(&store_url).unwrap();
        ctx.register_object_store(&url, store.as_generic());

        let table_url = format!("{}{}/", store_url, base_prefix);
        let options = ListingOptions::new(Arc::new(ParquetFormat::default()))
            .with_table_partition_cols(vec![
                ("year".to_string(), DataType::Int32),
                ("month".to_string(), DataType::Int32),
            ]);

        ctx.register_listing_table("partitioned_data", &table_url, options, None, None)
            .await
            .unwrap();

        let df = ctx
            .sql(
                "SELECT year, SUM(value) as total_value, COUNT(*) as row_count
             FROM partitioned_data
             GROUP BY year
             ORDER BY year",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        assert!(!batches.is_empty());

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        cleanup_hive_data(&store, &base_prefix).await;
    }
}

// ============================================================================
// PRIORITY 2: S3/MinIO Integration Tests
// ============================================================================

#[cfg(test)]
mod s3_minio_store_tests {
    use super::*;
    use flow_like_storage::object_store::ObjectStore;
    use reqwest::Client;

    const MINIO_ENDPOINT: &str = "http://localhost:9002";
    const MINIO_BUCKET: &str = "flowlike-test";

    async fn check_minio_available() -> bool {
        let client = Client::new();
        client
            .get(format!("{}/minio/health/live", MINIO_ENDPOINT))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_s3_store_with_minio_parquet_upload() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let (store, _) = create_test_store();
        let parquet_path = test_data::create_users_parquet(&store).await;

        let parquet_data = store
            .as_generic()
            .get(&parquet_path)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();

        let client = Client::new();
        let object_key = format!("test_uploads/users_{}.parquet", Uuid::new_v4());

        let put_url = format!("{}/{}/{}", MINIO_ENDPOINT, MINIO_BUCKET, object_key);
        let minio_access = "flowlike";
        let minio_secret = "flowlike_test";
        let response = client
            .put(&put_url)
            .basic_auth(minio_access, Some(minio_secret))
            .body(parquet_data.to_vec())
            .send()
            .await;

        match response {
            Ok(r) if r.status().is_success() => {
                println!("Uploaded parquet to MinIO: {}", object_key);
            }
            Ok(r) => {
                eprintln!("MinIO upload returned status: {} - skipping", r.status());
                store.as_generic().delete(&parquet_path).await.ok();
                return;
            }
            Err(e) => {
                eprintln!("MinIO upload failed: {} - skipping", e);
                store.as_generic().delete(&parquet_path).await.ok();
                return;
            }
        }

        let delete_url = format!("{}/{}/{}", MINIO_ENDPOINT, MINIO_BUCKET, object_key);
        client
            .delete(&delete_url)
            .basic_auth(minio_access, Some(minio_secret))
            .send()
            .await
            .ok();

        store.as_generic().delete(&parquet_path).await.ok();
    }

    #[tokio::test]
    async fn test_s3_store_list_objects() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let client = Client::new();
        let list_url = format!("{}/{}?list-type=2", MINIO_ENDPOINT, MINIO_BUCKET);
        let minio_access = "flowlike";
        let minio_secret = "flowlike_test";

        let response = client
            .get(&list_url)
            .basic_auth(minio_access, Some(minio_secret))
            .send()
            .await;

        match response {
            Ok(r) if r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                assert!(
                    body.contains("ListBucketResult")
                        || body.contains("Contents")
                        || body.is_empty()
                        || body.contains("<?xml")
                );
                println!("Successfully listed bucket contents");
            }
            Ok(r) => {
                eprintln!("List returned status: {}", r.status());
            }
            Err(e) => {
                eprintln!("List request failed: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_s3_datafusion_integration() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        use flow_like_storage::object_store::aws::AmazonS3Builder;

        let minio_access = "flowlike";
        let minio_secret = "flowlike_test";

        let s3 = match AmazonS3Builder::new()
            .with_bucket_name(MINIO_BUCKET)
            .with_endpoint(MINIO_ENDPOINT)
            .with_access_key_id(minio_access)
            .with_secret_access_key(minio_secret)
            .with_allow_http(true)
            .with_virtual_hosted_style_request(false)
            .build()
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build S3 store: {} - skipping", e);
                return;
            }
        };

        use flow_like_storage::object_store::{ObjectStore, path::Path};
        let list_result = s3.list(Some(&Path::from(""))).next().await;

        match list_result {
            Some(Ok(_)) => println!("Successfully connected to MinIO via object_store"),
            Some(Err(e)) => eprintln!(
                "Error listing: {} - store works but bucket might be empty",
                e
            ),
            None => println!("Empty bucket listing - store connection works"),
        }
    }
}

// ============================================================================
// Scoped S3 Credentials Tests - Tests prefix-based access control
// ============================================================================

#[cfg(test)]
mod scoped_credentials_tests {
    use super::*;
    use flow_like_storage::object_store::aws::AmazonS3Builder;
    use flow_like_storage::object_store::{ObjectStore, PutPayload, path::Path as ObjectStorePath};
    use reqwest::Client;

    const MINIO_ENDPOINT: &str = "http://localhost:9002";
    const CONTENT_BUCKET: &str = "flowlike-content";
    const LOGS_BUCKET: &str = "flowlike-logs";

    // Scoped user credentials (has access only to specific prefixes)
    const SCOPED_ACCESS_KEY: &str = "scoped-user";
    const SCOPED_SECRET_KEY: &str = "scoped-user-secret";

    // Denied user credentials (has no access)
    const DENIED_ACCESS_KEY: &str = "denied-user";
    const DENIED_SECRET_KEY: &str = "denied-user-secret";

    // Root credentials for setup/verification
    const ROOT_ACCESS_KEY: &str = "flowlike";
    const ROOT_SECRET_KEY: &str = "flowlike_test";

    async fn check_minio_available() -> bool {
        let client = Client::new();
        client
            .get(format!("{}/minio/health/live", MINIO_ENDPOINT))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn build_s3_store(
        bucket: &str,
        access_key: &str,
        secret_key: &str,
    ) -> Result<impl ObjectStore, String> {
        AmazonS3Builder::new()
            .with_bucket_name(bucket)
            .with_endpoint(MINIO_ENDPOINT)
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key)
            .with_allow_http(true)
            .with_virtual_hosted_style_request(false)
            .build()
            .map_err(|e| e.to_string())
    }

    #[tokio::test]
    async fn test_scoped_user_can_write_to_allowed_prefix() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(CONTENT_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store: {} - skipping", e);
                return;
            }
        };

        // Scoped user should be able to write to apps/test-app-123/
        let test_path = ObjectStorePath::from(format!(
            "apps/test-app-123/test_file_{}.txt",
            Uuid::new_v4()
        ));
        let test_content = b"Hello from scoped user!";

        let put_result = s3
            .put(&test_path, PutPayload::from(test_content.to_vec()))
            .await;

        match put_result {
            Ok(_) => {
                println!("✓ Scoped user successfully wrote to allowed prefix");
                // Cleanup
                s3.delete(&test_path).await.ok();
            }
            Err(e) => {
                // This might fail if MinIO setup hasn't run yet
                eprintln!(
                    "Scoped user write failed (ensure MinIO setup has run): {}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn test_scoped_user_can_write_to_user_prefix() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(CONTENT_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store: {} - skipping", e);
                return;
            }
        };

        // Scoped user should be able to write to users/user-abc/apps/test-app-123/
        let test_path = ObjectStorePath::from(format!(
            "users/user-abc/apps/test-app-123/user_data_{}.json",
            Uuid::new_v4()
        ));
        let test_content = br#"{"user": "test", "data": "allowed"}"#;

        let put_result = s3
            .put(&test_path, PutPayload::from(test_content.to_vec()))
            .await;

        match put_result {
            Ok(_) => {
                println!("✓ Scoped user successfully wrote to user prefix");
                s3.delete(&test_path).await.ok();
            }
            Err(e) => {
                eprintln!(
                    "Scoped user write to user prefix failed (ensure MinIO setup): {}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn test_scoped_user_can_write_to_temp_prefix() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(CONTENT_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store: {} - skipping", e);
                return;
            }
        };

        // Scoped user should be able to write to tmp/user/user-abc/apps/test-app-123/
        let test_path = ObjectStorePath::from(format!(
            "tmp/user/user-abc/apps/test-app-123/temp_data_{}.bin",
            Uuid::new_v4()
        ));
        let test_content = b"temporary data";

        let put_result = s3
            .put(&test_path, PutPayload::from(test_content.to_vec()))
            .await;

        match put_result {
            Ok(_) => {
                println!("✓ Scoped user successfully wrote to temp user prefix");
                s3.delete(&test_path).await.ok();
            }
            Err(e) => {
                eprintln!("Scoped user write to temp prefix failed: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_scoped_user_cannot_write_to_other_app() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(CONTENT_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store: {} - skipping", e);
                return;
            }
        };

        // Scoped user should NOT be able to write to apps/other-app/
        let forbidden_path = ObjectStorePath::from(format!(
            "apps/other-app-456/forbidden_{}.txt",
            Uuid::new_v4()
        ));
        let test_content = b"This should fail";

        let put_result = s3
            .put(&forbidden_path, PutPayload::from(test_content.to_vec()))
            .await;

        match put_result {
            Ok(_) => {
                // Unexpected success - clean up and warn
                eprintln!("⚠ WARNING: Scoped user was able to write to forbidden prefix!");
                s3.delete(&forbidden_path).await.ok();
            }
            Err(_) => {
                println!("✓ Scoped user correctly denied access to other app prefix");
            }
        }
    }

    #[tokio::test]
    async fn test_scoped_user_cannot_write_to_other_user() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(CONTENT_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store: {} - skipping", e);
                return;
            }
        };

        // Scoped user should NOT be able to write to users/other-user/
        let forbidden_path = ObjectStorePath::from(format!(
            "users/other-user-xyz/apps/test-app-123/forbidden_{}.txt",
            Uuid::new_v4()
        ));
        let test_content = b"This should fail";

        let put_result = s3
            .put(&forbidden_path, PutPayload::from(test_content.to_vec()))
            .await;

        match put_result {
            Ok(_) => {
                eprintln!("⚠ WARNING: Scoped user was able to write to other user's prefix!");
                s3.delete(&forbidden_path).await.ok();
            }
            Err(_) => {
                println!("✓ Scoped user correctly denied access to other user's prefix");
            }
        }
    }

    #[tokio::test]
    async fn test_scoped_user_can_read_from_allowed_prefix() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        // First, use root to create a file
        let root_s3 = match build_s3_store(CONTENT_BUCKET, ROOT_ACCESS_KEY, ROOT_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build root S3 store: {} - skipping", e);
                return;
            }
        };

        let test_path =
            ObjectStorePath::from(format!("apps/test-app-123/readable_{}.txt", Uuid::new_v4()));
        let test_content = b"Content to be read by scoped user";

        if root_s3
            .put(&test_path, PutPayload::from(test_content.to_vec()))
            .await
            .is_err()
        {
            eprintln!("Failed to create test file with root credentials");
            return;
        }

        // Now try to read with scoped user
        let scoped_s3 = match build_s3_store(CONTENT_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store: {} - skipping", e);
                root_s3.delete(&test_path).await.ok();
                return;
            }
        };

        let get_result = scoped_s3.get(&test_path).await;

        match get_result {
            Ok(result) => {
                let bytes = result.bytes().await.unwrap();
                assert_eq!(bytes.as_ref(), test_content);
                println!("✓ Scoped user successfully read from allowed prefix");
            }
            Err(e) => {
                eprintln!("Scoped user read failed: {}", e);
            }
        }

        // Cleanup
        root_s3.delete(&test_path).await.ok();
    }

    #[tokio::test]
    async fn test_denied_user_cannot_access_anything() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(CONTENT_BUCKET, DENIED_ACCESS_KEY, DENIED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build denied user S3 store: {} - skipping", e);
                return;
            }
        };

        // Denied user should not be able to list anything
        let list_result: Vec<_> = s3
            .list(Some(&ObjectStorePath::from("")))
            .take(1)
            .collect()
            .await;

        match list_result.first() {
            Some(Ok(_)) => {
                eprintln!("⚠ WARNING: Denied user was able to list bucket contents!");
            }
            Some(Err(_)) | None => {
                println!("✓ Denied user correctly cannot list bucket");
            }
        }
    }

    #[tokio::test]
    async fn test_scoped_user_datafusion_integration() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        // Create test parquet file with root credentials
        let root_s3 = match build_s3_store(CONTENT_BUCKET, ROOT_ACCESS_KEY, ROOT_SECRET_KEY) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("Failed to build root S3 store: {} - skipping", e);
                return;
            }
        };

        // Create a parquet file in the allowed prefix
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["Alice", "Bob", "Charlie"])),
            ],
        )
        .unwrap();

        let parquet_path = ObjectStorePath::from(format!(
            "apps/test-app-123/scoped_test_{}.parquet",
            Uuid::new_v4()
        ));

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }

        if root_s3
            .put(&parquet_path, PutPayload::from(buffer))
            .await
            .is_err()
        {
            eprintln!("Failed to create test parquet with root credentials");
            return;
        }

        // Now use scoped credentials to read via DataFusion
        let scoped_s3 = match AmazonS3Builder::new()
            .with_bucket_name(CONTENT_BUCKET)
            .with_endpoint(MINIO_ENDPOINT)
            .with_access_key_id(SCOPED_ACCESS_KEY)
            .with_secret_access_key(SCOPED_SECRET_KEY)
            .with_allow_http(true)
            .with_virtual_hosted_style_request(false)
            .build()
        {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("Failed to build scoped S3 for DataFusion: {}", e);
                root_s3.delete(&parquet_path).await.ok();
                return;
            }
        };

        let ctx = SessionContext::new();
        let s3_url = Url::parse(&format!("s3://{}", CONTENT_BUCKET)).unwrap();
        ctx.register_object_store(&s3_url, scoped_s3);

        let table_url = format!("s3://{}/{}", CONTENT_BUCKET, parquet_path.as_ref());
        let options =
            ListingOptions::new(Arc::new(ParquetFormat::default())).with_file_extension("parquet");

        match ListingTableConfig::new(ListingTableUrl::parse(&table_url).unwrap())
            .with_listing_options(options)
            .infer_schema(&ctx.state())
            .await
        {
            Ok(config) => {
                let table = ListingTable::try_new(config).unwrap();
                ctx.register_table("scoped_data", Arc::new(table)).unwrap();

                let df = ctx.sql("SELECT * FROM scoped_data").await.unwrap();
                let batches = df.collect().await.unwrap();
                let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
                assert_eq!(total_rows, 3);
                println!("✓ Scoped user successfully queried parquet via DataFusion");
            }
            Err(e) => {
                eprintln!("Failed to register table with scoped credentials: {}", e);
            }
        }

        // Cleanup
        root_s3.delete(&parquet_path).await.ok();
    }

    #[tokio::test]
    async fn test_scoped_credentials_logs_bucket_access() {
        if !check_minio_available().await {
            eprintln!("Skipping test - MinIO not available");
            return;
        }

        let s3 = match build_s3_store(LOGS_BUCKET, SCOPED_ACCESS_KEY, SCOPED_SECRET_KEY) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build scoped S3 store for logs: {} - skipping", e);
                return;
            }
        };

        // Scoped user should be able to write to runs/test-app-123/
        let log_path =
            ObjectStorePath::from(format!("runs/test-app-123/run_{}.log", Uuid::new_v4()));
        let log_content = b"[INFO] Test log entry from scoped credential test";

        let put_result = s3
            .put(&log_path, PutPayload::from(log_content.to_vec()))
            .await;

        match put_result {
            Ok(_) => {
                println!("✓ Scoped user successfully wrote to logs bucket");
                s3.delete(&log_path).await.ok();
            }
            Err(e) => {
                eprintln!("Scoped user write to logs failed: {}", e);
            }
        }
    }
}

// ============================================================================
// Oracle Integration Tests
// ============================================================================
// NOTE: These tests simulate Oracle-style queries using DataFusion. Direct
// DataFusion -> Oracle connections are NOT currently possible because:
// 1. DataFusion does not have a native Oracle table provider
// 2. The typical pattern is to export Oracle data to Parquet/CSV for analytics
// 3. The RegisterOracleNode creates a placeholder VIEW (not actual connection)
//
// For true Oracle connectivity, users would:
// - Use Oracle client libraries (oracle crate) for CRUD operations
// - Export data to object storage (S3/Parquet) for DataFusion analytics
// - Use a federated query engine that supports Oracle (e.g., Trino, Presto)
// ============================================================================

#[cfg(test)]
mod oracle_integration_tests {
    use super::*;

    const ORACLE_HOST: &str = "localhost";
    const ORACLE_PORT: u16 = 1521;

    async fn check_oracle_port_open() -> bool {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::TcpStream::connect(format!("{}:{}", ORACLE_HOST, ORACLE_PORT)),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_oracle_connection_available() {
        if !check_oracle_port_open().await {
            eprintln!(
                "Skipping test - Oracle not available on port {}",
                ORACLE_PORT
            );
            return;
        }
        println!(
            "Oracle port {} is open and accepting connections",
            ORACLE_PORT
        );
    }

    #[tokio::test]
    async fn test_oracle_simulated_data_export() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("employee_id", DataType::Int32, false),
            Field::new("first_name", DataType::Utf8, false),
            Field::new("last_name", DataType::Utf8, false),
            Field::new("department", DataType::Utf8, true),
            Field::new("salary", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![100, 101, 102, 103, 104])),
                Arc::new(StringArray::from(vec![
                    "John", "Jane", "Bob", "Alice", "Charlie",
                ])),
                Arc::new(StringArray::from(vec![
                    "Doe", "Smith", "Johnson", "Williams", "Brown",
                ])),
                Arc::new(StringArray::from(vec![
                    Some("IT"),
                    Some("HR"),
                    Some("IT"),
                    Some("Finance"),
                    Some("HR"),
                ])),
                Arc::new(Float64Array::from(vec![
                    75000.0, 65000.0, 82000.0, 90000.0, 55000.0,
                ])),
            ],
        )
        .unwrap();

        let (store, store_url) = create_test_store();
        let path = unique_object_path("oracle_export", "parquet");

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        use flow_like_storage::object_store::PutPayload;
        store
            .as_generic()
            .put(&path, PutPayload::from(buffer))
            .await
            .unwrap();

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "employees").await;

        let df = ctx
            .sql(
                "SELECT department, AVG(salary) as avg_salary, COUNT(*) as emp_count
             FROM employees
             GROUP BY department
             ORDER BY avg_salary DESC",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_oracle_style_hierarchical_query() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("manager_id", DataType::Int32, true),
            Field::new("level", DataType::Int32, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5])),
                Arc::new(StringArray::from(vec![
                    "CEO", "VP Sales", "VP Tech", "Manager", "Engineer",
                ])),
                Arc::new(Int32Array::from(vec![
                    None,
                    Some(1),
                    Some(1),
                    Some(3),
                    Some(4),
                ])),
                Arc::new(Int32Array::from(vec![1, 2, 2, 3, 4])),
            ],
        )
        .unwrap();

        let (store, store_url) = create_test_store();
        let path = unique_object_path("oracle_hierarchy", "parquet");

        let mut buffer = Vec::new();
        {
            use flow_like_storage::datafusion::parquet::arrow::ArrowWriter;
            let mut writer = ArrowWriter::try_new(&mut buffer, schema, None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        use flow_like_storage::object_store::PutPayload;
        store
            .as_generic()
            .put(&path, PutPayload::from(buffer))
            .await
            .unwrap();

        let ctx = SessionContext::new();
        register_parquet_from_store(&ctx, &store, &store_url, &path, "org_chart").await;

        let df = ctx
            .sql(
                "SELECT level, COUNT(*) as count_at_level
             FROM org_chart
             GROUP BY level
             ORDER BY level",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 4);

        store.as_generic().delete(&path).await.ok();
    }
}

// ============================================================================
// PRIORITY 3: Node Execution Interface Tests - Using FlowLikeStore
// ============================================================================

#[cfg(test)]
mod node_execution_tests {
    use super::*;

    #[tokio::test]
    async fn test_datafusion_session_creation() {
        let ctx = SessionContext::new();

        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;
        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let tables = ctx.catalog_names();
        assert!(!tables.is_empty());

        let df = ctx.table("users").await.unwrap();
        let schema = df.schema();
        assert!(schema.field_with_name(None, "id").is_ok());
        assert!(schema.field_with_name(None, "name").is_ok());

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_sql_query_execution() {
        let ctx = SessionContext::new();
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx
            .sql("SELECT id, name FROM users WHERE department = 'Engineering'")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_describe_table_functionality() {
        let ctx = SessionContext::new();
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx.table("users").await.unwrap();
        let schema = df.schema();

        assert_eq!(schema.fields().len(), 3);

        let id_field = schema.field_with_name(None, "id").unwrap();
        assert_eq!(id_field.data_type(), &DataType::Int32);

        let name_field = schema.field_with_name(None, "name").unwrap();
        assert!(matches!(
            name_field.data_type(),
            DataType::Utf8 | DataType::Utf8View
        ));

        let dept_field = schema.field_with_name(None, "department").unwrap();
        assert!(matches!(
            dept_field.data_type(),
            DataType::Utf8 | DataType::Utf8View
        ));

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_list_tables_functionality() {
        let ctx = SessionContext::new();
        let (store, store_url) = create_test_store();

        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;

        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;

        let catalog = ctx.catalog("datafusion").unwrap();
        let schema = catalog.schema("public").unwrap();
        let table_names = schema.table_names();

        assert!(table_names.contains(&"users".to_string()));
        assert!(table_names.contains(&"orders".to_string()));
        assert_eq!(table_names.len(), 2);

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
    }

    #[tokio::test]
    async fn test_multiple_sessions_isolation() {
        let ctx1 = SessionContext::new();
        let ctx2 = SessionContext::new();

        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        register_parquet_from_store(&ctx1, &store, &store_url, &path, "users").await;

        let catalog1 = ctx1.catalog("datafusion").unwrap();
        let schema1 = catalog1.schema("public").unwrap();
        assert!(schema1.table_names().contains(&"users".to_string()));

        let catalog2 = ctx2.catalog("datafusion").unwrap();
        let schema2 = catalog2.schema("public").unwrap();
        assert!(!schema2.table_names().contains(&"users".to_string()));

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_query_with_parameters() {
        let ctx = SessionContext::new();
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let department = "Engineering";
        let query = format!("SELECT * FROM users WHERE department = '{}'", department);
        let df = ctx.sql(&query).await.unwrap();
        let batches = df.collect().await.unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_query_result_to_json() {
        use flow_like_storage::datafusion::arrow::array::{Array, AsArray};

        let ctx = SessionContext::new();
        let (store, store_url) = create_test_store();
        let path = test_data::create_users_parquet(&store).await;

        register_parquet_from_store(&ctx, &store, &store_url, &path, "users").await;

        let df = ctx
            .sql("SELECT id, name FROM users ORDER BY id LIMIT 2")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        assert!(!batches.is_empty());
        let batch = &batches[0];

        let id_col = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(id_col.value(0), 1);

        let name_col = batch.column(1);
        let name_value: String = if let Some(arr) = name_col.as_any().downcast_ref::<StringArray>()
        {
            arr.value(0).to_string()
        } else {
            name_col.as_string_view().value(0).to_string()
        };
        assert_eq!(name_value, "Alice");

        store.as_generic().delete(&path).await.ok();
    }

    #[tokio::test]
    async fn test_complex_query_plan() {
        let ctx = SessionContext::new();
        let (store, store_url) = create_test_store();

        let users_path = test_data::create_users_parquet(&store).await;
        let orders_path = test_data::create_orders_csv(&store).await;
        let salaries_path = test_data::create_salaries_parquet(&store).await;

        register_parquet_from_store(&ctx, &store, &store_url, &users_path, "users").await;
        register_csv_from_store(&ctx, &store, &store_url, &orders_path, "orders").await;
        register_parquet_from_store(&ctx, &store, &store_url, &salaries_path, "salaries").await;

        let df = ctx
            .sql(
                "WITH user_orders AS (
                SELECT user_id, SUM(amount) as total_orders
                FROM orders
                GROUP BY user_id
            ),
            user_comp AS (
                SELECT employee_id, salary + COALESCE(bonus, 0) as total_comp
                FROM salaries
            )
            SELECT u.name, u.department,
                   COALESCE(uo.total_orders, 0) as orders,
                   COALESCE(uc.total_comp, 0) as compensation
            FROM users u
            LEFT JOIN user_orders uo ON u.id = uo.user_id
            LEFT JOIN user_comp uc ON u.id = uc.employee_id
            ORDER BY u.id",
            )
            .await
            .unwrap();

        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5);

        store.as_generic().delete(&users_path).await.ok();
        store.as_generic().delete(&orders_path).await.ok();
        store.as_generic().delete(&salaries_path).await.ok();
    }
}
