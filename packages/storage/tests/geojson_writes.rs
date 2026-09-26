//! GeoJSON and structured values written through LanceDBVectorStore: declared geometry columns
//! take Features and text, text columns take objects as JSON text, and rejected input surfaces
//! as `TableInputRejected`.

use std::{collections::HashMap, path::PathBuf};

use flow_like_storage::{
    databases::vector::{
        VectorStore,
        lancedb::{LanceDBVectorStore, record_batches_to_vec},
        schema::{DatabaseSchemaField, TableInputRejected, database_fields_to_arrow_schema},
    },
    geometry::is_geometry_field,
};
use flow_like_types::{Error, Result, Value, create_id, json::json};

struct TempDb(PathBuf);

impl TempDb {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("flow-like-geojson-writes-{}", create_id())))
    }

    async fn open(&self, table: &str) -> Result<LanceDBVectorStore> {
        LanceDBVectorStore::new(self.0.clone(), table.into()).await
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn field(name: &str, data_type: &str) -> DatabaseSchemaField {
    DatabaseSchemaField {
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        vector_size: None,
        primary_key: false,
    }
}

fn key(name: &str) -> DatabaseSchemaField {
    DatabaseSchemaField {
        nullable: false,
        primary_key: true,
        ..field(name, "string")
    }
}

fn square(offset: f64) -> Value {
    json!({"type": "Polygon", "coordinates": [[
        [offset, offset], [offset + 1.0, offset], [offset + 1.0, offset + 1.0],
        [offset, offset + 1.0], [offset, offset]
    ]]})
}

fn with_hole() -> Value {
    json!({"type": "Polygon", "coordinates": [
        [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]],
        [[1.0, 1.0], [1.0, 2.0], [2.0, 2.0], [2.0, 1.0], [1.0, 1.0]]
    ]})
}

fn feature(id: Option<&str>, geometry: Value) -> Value {
    let mut feature = json!({
        "type": "Feature",
        "geometry": geometry,
        "properties": {"siteName": "Example", "tags": {"addr:city": "Springfield"}}
    });
    if let Some(id) = id {
        feature["id"] = json!(id);
    }
    feature
}

async fn sites(db: &TempDb) -> Result<LanceDBVectorStore> {
    let mut sites = db.open("sites").await?;
    sites
        .create_empty_table(
            database_fields_to_arrow_schema(&[
                key("feature_id"),
                field("label", "string"),
                field("tags", "string"),
                field("count", "int64"),
                field("flag", "boolean"),
                field("seen_at", "timestamp:ms:UTC"),
                field("payload", "binary"),
                field("geometry", "geometry"),
            ])?,
            true,
        )
        .await?;
    Ok(sites)
}

async fn rows(db: &LanceDBVectorStore, sql: &str) -> Result<Vec<Value>> {
    record_batches_to_vec(Some(db.sql("sites", sql).await?.collect().await?))
}

async fn geometry_of(db: &LanceDBVectorStore, id: &str) -> Result<Value> {
    let rows = rows(
        db,
        &format!("SELECT geometry FROM sites WHERE feature_id = '{id}'"),
    )
    .await?;
    Ok(rows[0]["geometry"].clone())
}

fn rejection(error: Error) -> String {
    assert!(
        error
            .chain()
            .any(|cause| cause.downcast_ref::<TableInputRejected>().is_some()),
        "expected a table input rejection, got: {error:#}"
    );
    error.to_string()
}

#[tokio::test]
async fn declared_geometry_columns_take_features_and_geojson_or_wkt_text() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;

    sites
        .insert(vec![
            json!({"feature_id": "feature", "geometry": feature(Some("f-1"), with_hole())}),
            json!({"feature_id": "geojson_text", "geometry": square(10.0).to_string()}),
            json!({"feature_id": "wkt_text", "geometry": "POLYGON ((20 20, 21 20, 21 21, 20 21, 20 20))"}),
            json!({"feature_id": "unplaced", "geometry": feature(None, Value::Null)}),
        ])
        .await?;

    assert_eq!(geometry_of(&sites, "feature").await?, with_hole());
    assert_eq!(geometry_of(&sites, "geojson_text").await?, square(10.0));
    assert_eq!(geometry_of(&sites, "wkt_text").await?, square(20.0));
    assert_eq!(geometry_of(&sites, "unplaced").await?, Value::Null);

    let inside = rows(
        &sites,
        "SELECT feature_id FROM sites WHERE ST_Intersects(geometry, flow_geomfromtext('POINT(3 3)'))",
    )
    .await?;
    assert_eq!(inside, vec![json!({"feature_id": "feature"})]);
    Ok(())
}

#[tokio::test]
async fn declared_geometry_columns_reject_feature_collections() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    let error = sites
        .insert(vec![json!({
            "feature_id": "collection",
            "geometry": {"type": "FeatureCollection", "features": [feature(None, square(0.0))]}
        })])
        .await
        .expect_err("a FeatureCollection is many rows, not one geometry");
    let message = rejection(error);
    assert!(
        message.contains("Geometry column 'geometry', row 0"),
        "{message}"
    );
    assert!(message.contains("one row per feature"), "{message}");
    assert_eq!(sites.count(None).await?, 0);
    Ok(())
}

#[tokio::test]
async fn geometry_updates_take_features() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    sites
        .insert(vec![json!({"feature_id": "a", "geometry": square(0.0)})])
        .await?;

    sites
        .update(
            "feature_id = 'a'",
            HashMap::from([("geometry".to_string(), feature(Some("a"), square(5.0)))]),
        )
        .await?;
    assert_eq!(geometry_of(&sites, "a").await?, square(5.0));

    sites
        .update(
            "feature_id = 'a'",
            HashMap::from([("geometry".to_string(), json!("POINT (1 2)"))]),
        )
        .await?;
    assert_eq!(
        geometry_of(&sites, "a").await?,
        json!({"type": "Point", "coordinates": [1.0, 2.0]})
    );

    let error = sites
        .update(
            "feature_id = 'a'",
            HashMap::from([(
                "geometry".to_string(),
                json!({"type": "Point", "coordinates": [1.0, 2.0, 3.0]}),
            )]),
        )
        .await
        .expect_err("Z coordinates are not stored");
    assert!(rejection(error).contains("Z/M"));
    Ok(())
}

#[tokio::test]
async fn text_columns_store_objects_and_arrays_as_json_text() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    sites
        .insert(vec![json!({
            "feature_id": "a",
            "label": ["id-1", "id-2"],
            "tags": {"addr:city": "Springfield", "building": "yes"}
        })])
        .await?;
    sites
        .upsert(
            vec![json!({"feature_id": "b", "tags": {"levels": [1, 2]}, "label": 7})],
            "feature_id".into(),
        )
        .await?;

    assert_eq!(
        rows(
            &sites,
            "SELECT feature_id, label, tags FROM sites ORDER BY feature_id"
        )
        .await?,
        vec![
            json!({"feature_id": "a", "label": r#"["id-1","id-2"]"#, "tags": r#"{"addr:city":"Springfield","building":"yes"}"#}),
            json!({"feature_id": "b", "label": "7", "tags": r#"{"levels":[1,2]}"#}),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn first_writes_reject_invalid_geometry_naming_column_and_row() -> Result<()> {
    let db = TempDb::new();
    let mut shapes = db.open("shapes").await?;

    let error = shapes
        .insert(vec![
            json!({"id": "a", "outline": square(0.0)}),
            json!({"id": "b", "outline": {"type": "Polygon", "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [5.0, 5.0]]]}}),
        ])
        .await
        .expect_err("an unclosed ring is not a geometry");
    let message = rejection(error);
    assert!(message.contains("'outline'"), "{message}");
    assert!(message.contains("row 1"), "{message}");
    assert!(message.contains("closed"), "{message}");
    assert!(!shapes.table_exists().await?);

    shapes
        .insert(vec![
            json!({"id": "a", "outline": square(0.0), "raw": feature(Some("a"), square(0.0))}),
            json!({"id": "b", "outline": null, "raw": feature(Some("b"), square(1.0))}),
        ])
        .await?;
    let schema = shapes.schema().await?;
    assert!(is_geometry_field(schema.field_with_name("outline")?));
    assert!(!is_geometry_field(schema.field_with_name("raw")?));
    Ok(())
}

#[tokio::test]
async fn plain_binary_updates_reject_objects_and_point_to_geometry_columns() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    sites
        .insert(vec![json!({"feature_id": "a", "payload": [1, 2]})])
        .await?;

    for value in [square(0.0), json!([13.4, 52.5])] {
        let error = sites
            .update(
                "feature_id = 'a'",
                HashMap::from([("payload".to_string(), value)]),
            )
            .await
            .expect_err("a plain binary column holds bytes");
        let message = rejection(error);
        assert!(message.contains("'payload'"), "{message}");
        assert!(message.contains("\"type\": \"geometry\""), "{message}");
    }
    assert_eq!(
        rows(&sites, "SELECT payload FROM sites").await?,
        vec![json!({"payload": [1, 2]})]
    );
    Ok(())
}

#[tokio::test]
async fn rejected_input_is_typed_through_the_error_chain() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;

    let error = sites
        .insert(vec![json!({"feature_id": "a", "count": "seven"})])
        .await
        .expect_err("text is not an int64");
    rejection(error);

    let error = sites
        .update(
            "feature_id = 'a'",
            HashMap::from([
                ("colour".to_string(), json!("red")),
                ("label".to_string(), json!("x")),
            ]),
        )
        .await
        .expect_err("the table has no colour column");
    let message = rejection(error);
    assert_eq!(
        message,
        "Table 'sites' has no column 'colour'; its columns are 'feature_id', 'label', 'tags', 'count', 'flag', 'seen_at', 'payload', 'geometry'"
    );
    Ok(())
}

#[tokio::test]
async fn updates_reject_values_the_column_type_cannot_hold() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    sites
        .insert(vec![json!({"feature_id": "a", "count": 3, "flag": true})])
        .await?;

    for (column, value) in [
        ("count", json!("seven")),
        ("count", json!({"a": 1})),
        ("flag", json!("maybe")),
        ("seen_at", json!("yesterday")),
    ] {
        let received = value.to_string();
        let error = sites
            .update(
                "feature_id = 'a'",
                HashMap::from([(column.to_string(), value)]),
            )
            .await
            .expect_err("the column type cannot hold the value");
        let message = rejection(error);
        let named = format!("Column '{column}' of table 'sites'");
        assert!(
            message.contains(&named) && message.contains(&received),
            "{message}"
        );
    }

    sites
        .update(
            "feature_id = 'a'",
            HashMap::from([
                ("count".to_string(), json!("7")),
                ("flag".to_string(), json!(false)),
                ("seen_at".to_string(), json!("2026-01-02T03:04:05Z")),
            ]),
        )
        .await?;
    assert_eq!(
        rows(&sites, "SELECT count, flag FROM sites").await?,
        vec![json!({"count": 7, "flag": false})]
    );
    Ok(())
}

#[tokio::test]
async fn sql_converts_between_geometry_and_geojson_or_wkt_text() -> Result<()> {
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    sites
        .insert(vec![
            json!({"feature_id": "a", "geometry": with_hole()}),
            json!({"feature_id": "b", "geometry": null}),
        ])
        .await?;

    let converted = rows(
        &sites,
        "SELECT feature_id, ST_AsGeoJSON(geometry) AS json, ST_AsText(geometry) AS wkt, \
         ST_GeomFromGeoJSON(ST_AsGeoJSON(geometry)) AS round_trip \
         FROM sites ORDER BY feature_id",
    )
    .await?;
    let json_text = converted[0]["json"].as_str().expect("GeoJSON text");
    assert_eq!(
        flow_like_types::json::from_str::<Value>(json_text)?,
        with_hole()
    );
    let wkt = converted[0]["wkt"].as_str().expect("WKT text");
    assert!(wkt.starts_with("POLYGON"), "{wkt}");
    assert_eq!(flow_like_geometry::from_wkt(wkt)?, with_hole());
    assert_eq!(converted[0]["round_trip"], with_hole());
    assert_eq!(
        converted[1],
        json!({"feature_id": "b", "json": null, "wkt": null, "round_trip": null})
    );
    let text = sites
        .sql(
            "sites",
            "SELECT ST_AsText(geometry) AS wkt, ST_AsGeoJSON(geometry) AS json FROM sites",
        )
        .await?;
    let schema = text.schema();
    assert!(
        !schema.fields().iter().any(|field| is_geometry_field(field)),
        "{schema:?}"
    );

    let matched = rows(
        &sites,
        "SELECT feature_id FROM sites WHERE ST_Contains(geometry, \
         ST_GeomFromGeoJSON('{\"type\":\"Point\",\"coordinates\":[3,3]}'))",
    )
    .await?;
    assert_eq!(matched, vec![json!({"feature_id": "a"})]);
    Ok(())
}

#[cfg(feature = "graph")]
#[tokio::test]
async fn workbench_columns_describe_st_astext_as_plain_text() -> Result<()> {
    use flow_like_storage::{
        databases::workbench::{WorkbenchSurface, execute_readonly_sql},
        geometry::EXTENSION_NAME,
    };
    let db = TempDb::new();
    let mut sites = sites(&db).await?;
    sites
        .insert(vec![json!({"feature_id": "a", "geometry": with_hole()})])
        .await?;

    let connection = flow_like_storage::lancedb::connect(&db.0.to_string_lossy())
        .execute()
        .await?;
    let result = execute_readonly_sql(
        &connection,
        WorkbenchSurface::Native,
        Vec::new(),
        "SELECT geometry, ST_AsText(geometry) AS wkt FROM sites",
        &Value::Null,
        Some(10),
    )
    .await?;
    let column = |name: &str| {
        result
            .columns
            .iter()
            .find(|column| column.name == name)
            .unwrap_or_else(|| panic!("no column {name} in {:?}", result.columns))
    };
    assert!(column("geometry").metadata.contains_key(EXTENSION_NAME));
    assert!(
        !column("wkt").metadata.contains_key(EXTENSION_NAME),
        "{:?}",
        column("wkt")
    );
    let wkt = result.rows[0]["wkt"].as_str().expect("WKT text");
    assert_eq!(flow_like_geometry::from_wkt(wkt)?, with_hole());
    Ok(())
}
