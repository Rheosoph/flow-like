//! Geometry fields, GeoArrow conversion, and application SQL registration.

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{DataType, Field};
use flow_like_types::{Result, Value, anyhow};
use geoarrow_array::GeoArrowArray;
use std::{collections::HashMap, sync::Arc};

mod geojson;
mod nested;

pub const EXTENSION_NAME: &str = "ARROW:extension:name";
pub const EXTENSION_METADATA: &str = "ARROW:extension:metadata";
pub const WGS84_METADATA: &str = r#"{"crs":"EPSG:4326","crs_type":"authority_code"}"#;

pub fn is_geometry_field(field: &Field) -> bool {
    field
        .metadata()
        .get(EXTENSION_NAME)
        .is_some_and(|name| name.starts_with("geoarrow."))
}

pub fn contains_geometry_field(field: &Field) -> bool {
    fn children(data_type: &DataType) -> bool {
        match data_type {
            DataType::Struct(fields) => fields.iter().any(|field| contains_geometry_field(field)),
            DataType::List(field)
            | DataType::LargeList(field)
            | DataType::FixedSizeList(field, _)
            | DataType::ListView(field)
            | DataType::LargeListView(field)
            | DataType::Map(field, _)
            | DataType::RunEndEncoded(_, field) => contains_geometry_field(field),
            DataType::Union(fields, _) => fields
                .iter()
                .any(|(_, field)| contains_geometry_field(field)),
            DataType::Dictionary(_, value) => children(value),
            _ => false,
        }
    }
    is_geometry_field(field) || children(field.data_type())
}

/// V1 table writes support scalar geometry columns. Nested geometry reads remain available.
pub fn validate_geometry_field(field: &Field) -> Result<()> {
    if !is_geometry_field(field) && contains_geometry_field(field) {
        return Err(anyhow!(
            "Column '{}' contains nested geometry; declare a scalar geometry column for validated writes",
            field.name()
        ));
    }
    if is_geometry_field(field) {
        validate_crs(field)?;
    }
    Ok(())
}

const GEOMETRY_INPUT_FORMS: &str = "a GeoJSON geometry object (Point, LineString, Polygon, MultiPoint, MultiLineString, MultiPolygon or GeometryCollection), a GeoJSON Feature, GeoJSON text, WKT text in longitude latitude order, or null";

/// The geometry a declared geometry column stores for `value`: a bare GeoJSON
/// geometry, or null. Features contribute their geometry; GeoJSON and WKT text
/// are parsed; `bbox` and foreign members are dropped.
pub fn geometry_input(value: &Value) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::String(text) => geometry_text(text),
        Value::Object(object) => match object.get("type").and_then(Value::as_str) {
            Some("Feature") => match object.get("geometry") {
                Some(Value::Null) => Ok(Value::Null),
                Some(geometry) => bare_geometry(geometry)
                    .map_err(|error| anyhow!("GeoJSON Feature geometry: {error}")),
                None => Err(anyhow!(
                    "received a GeoJSON Feature without a geometry member; expected {GEOMETRY_INPUT_FORMS}"
                )),
            },
            Some("FeatureCollection") => Err(anyhow!(
                "received a GeoJSON FeatureCollection; write one row per feature, each carrying that feature's geometry"
            )),
            _ => bare_geometry(value),
        },
        other => Err(anyhow!(
            "received {}; expected {GEOMETRY_INPUT_FORMS}",
            describe_value(other)
        )),
    }
}

/// [`geometry_input`] encoded as the WKB a geometry column stores.
pub fn geometry_input_wkb(value: &Value) -> Result<Option<Vec<u8>>> {
    match geometry_input(value)? {
        Value::Null => Ok(None),
        geometry => flow_like_geometry::to_wkb(&geometry).map(Some),
    }
}

fn geometry_text(text: &str) -> Result<Value> {
    let text = text.trim();
    if text.is_empty() {
        return Err(anyhow!(
            "received an empty string; use null for an absent geometry, otherwise {GEOMETRY_INPUT_FORMS}"
        ));
    }
    if text.starts_with('{') {
        let parsed: Value = serde_json::from_str(text)
            .map_err(|error| anyhow!("received text that is not valid GeoJSON: {error}"))?;
        return geometry_input(&parsed);
    }
    flow_like_geometry::from_wkt(text).map_err(|error| {
        anyhow!("received text that is neither GeoJSON nor valid WKT ({error}); expected {GEOMETRY_INPUT_FORMS}")
    })
}

fn bare_geometry(value: &Value) -> Result<Value> {
    if !names_geometry_kind(value) {
        return Err(anyhow!(
            "received {}; expected {GEOMETRY_INPUT_FORMS}",
            describe_value(value)
        ));
    }
    let geometry = without_foreign_members(value);
    flow_like_types::geometry::validate_geometry(&geometry, None)?;
    Ok(geometry)
}

/// Keeps `crs` so the validator still refuses an alternate CRS declaration.
fn without_foreign_members(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    Value::Object(
        object
            .iter()
            .filter_map(|(key, member)| match key.as_str() {
                "type" | "coordinates" | "crs" => Some((key.clone(), member.clone())),
                "geometries" => Some((
                    key.clone(),
                    member
                        .as_array()
                        .map(|children| {
                            Value::Array(children.iter().map(without_foreign_members).collect())
                        })
                        .unwrap_or_else(|| member.clone()),
                )),
                _ => None,
            })
            .collect(),
    )
}

fn describe_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "a boolean".into(),
        Value::Number(_) => "a number".into(),
        Value::String(_) => "a string".into(),
        Value::Array(_) => "an array".into(),
        Value::Object(object) => match object.get("type").and_then(Value::as_str) {
            Some(kind) => format!("an object of type '{kind}'"),
            None => "an object without a GeoJSON type".into(),
        },
    }
}

/// An object whose `type` names a geometry kind is meant as a geometry: a
/// query parameter shaped like this binds as one, and must validate as one.
pub fn names_geometry_kind(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| {
            kind.parse::<flow_like_types::geometry::GeometryKind>()
                .is_ok()
        })
}

pub fn geometry_field(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Binary, nullable).with_metadata(HashMap::from([
        (EXTENSION_NAME.into(), "geoarrow.wkb".into()),
        (EXTENSION_METADATA.into(), WGS84_METADATA.into()),
    ]))
}

/// Preserves semantic column types alongside JSON rows.
pub fn property_metadata(
    schema: &arrow_schema::Schema,
) -> HashMap<String, HashMap<String, String>> {
    schema
        .fields()
        .iter()
        .filter(|field| !field.metadata().is_empty())
        .map(|field| (field.name().clone(), field.metadata().clone()))
        .collect()
}

pub fn validate_crs(field: &Field) -> Result<()> {
    let metadata: Value = serde_json::from_str(
        field
            .metadata()
            .get(EXTENSION_METADATA)
            .map(String::as_str)
            .unwrap_or("{}"),
    )?;
    let crs = metadata.get("crs");
    let known = crs.and_then(Value::as_str).is_some_and(|crs| {
        matches!(
            crs,
            "EPSG:4326" | "OGC:CRS84" | "urn:ogc:def:crs:OGC::CRS84"
        )
    }) || crs.is_some_and(|crs| {
        let id = &crs["id"];
        id["authority"].as_str() == Some("EPSG")
            && (id["code"].as_u64() == Some(4326) || id["code"].as_str() == Some("4326"))
    });
    if !known {
        return Err(anyhow!(
            "Geometry column '{}' has unknown or unsupported CRS. Use flow_geomfromtext(WKT) to explicitly import WGS84 longitude/latitude coordinates",
            field.name()
        ));
    }
    if metadata
        .get("edges")
        .is_some_and(|edges| !edges.is_null() && edges.as_str() != Some("planar"))
    {
        return Err(anyhow!(
            "Geometry column '{}' uses unsupported edge semantics",
            field.name()
        ));
    }
    Ok(())
}

/// Decode every supported GeoArrow layout through its WKB representation.
pub fn decode_column(array: &dyn Array, field: &Field) -> Result<Vec<Value>> {
    validate_crs(field)?;
    let geo = geoarrow_array::array::from_arrow_array(array, field)?;
    let wkb = geoarrow_array::cast::to_wkb::<i32>(geo.as_ref())?.to_array_ref();
    let binary = wkb
        .as_any()
        .downcast_ref::<BinaryArray>()
        .ok_or_else(|| anyhow!("GeoArrow WKB conversion did not return Binary"))?;
    binary
        .iter()
        .map(|bytes| {
            bytes
                .map(flow_like_geometry::from_wkb)
                .transpose()
                .map(|value| value.unwrap_or(Value::Null))
        })
        .collect()
}

pub fn decode_value(array: &dyn Array, field: &Field, row: usize) -> Result<Value> {
    if row >= array.len() {
        return Err(anyhow!("Geometry row index out of bounds"));
    }
    Ok(decode_column(array.slice(row, 1).as_ref(), field)?.remove(0))
}

/// Validate geometry columns supplied as Arrow, before any database mutation.
pub fn validate_batch(batch: &RecordBatch) -> Result<()> {
    for (field, array) in batch.schema().fields().iter().zip(batch.columns()) {
        validate_geometry_field(field)?;
        if is_geometry_field(field) {
            decode_column(array.as_ref(), field)?;
        }
    }
    Ok(())
}

/// Convert direct Arrow geometry input to the declared WKB schema.
pub fn normalize_batch(
    batch: &RecordBatch,
    target: &arrow_schema::SchemaRef,
) -> Result<RecordBatch> {
    if batch.num_columns() != target.fields().len() {
        return Err(anyhow!(
            "Arrow insert column count differs from the declared table schema"
        ));
    }
    let mut arrays = Vec::with_capacity(target.fields().len());
    for field in target.fields() {
        validate_geometry_field(field)?;
        let index = batch.schema().index_of(field.name())?;
        let source = batch.schema().field(index).clone();
        let array = batch.column(index);
        if is_geometry_field(field) {
            if !is_geometry_field(&source) {
                return Err(anyhow!(
                    "Column '{}' requires declared GeoArrow input with WGS84 metadata",
                    field.name()
                ));
            }
            validate_crs(field)?;
            let values = decode_column(array.as_ref(), &source)?;
            if field.data_type() != &DataType::Binary {
                if source.data_type() != field.data_type()
                    || source.metadata().get(EXTENSION_NAME) != field.metadata().get(EXTENSION_NAME)
                {
                    return Err(anyhow!(
                        "Native GeoArrow targets require matching geometry kinds and Arrow layouts"
                    ));
                }
                arrays.push(array.clone());
                continue;
            }
            let bytes = values
                .iter()
                .map(|value| {
                    if value.is_null() {
                        Ok(None)
                    } else {
                        flow_like_geometry::to_wkb(value).map(Some)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            arrays.push(Arc::new(BinaryArray::from_iter(
                bytes.iter().map(|bytes| bytes.as_deref()),
            )) as ArrayRef);
        } else {
            if is_geometry_field(&source) {
                return Err(anyhow!(
                    "Geometry column '{}' requires an explicitly declared geometry target",
                    field.name()
                ));
            }
            arrays.push(array.clone());
        }
    }
    let result = RecordBatch::try_new(target.clone(), arrays)?;
    validate_batch(&result)?;
    Ok(result)
}

/// Register spatial SQL and an explicit WGS84 import operation.
pub fn register_geo_functions(context: &datafusion::prelude::SessionContext) {
    geodatafusion::register(context);
    register_ordered_relations(context);
    nested::register_extension_preserving_nesting(context);
    geojson::register_text_functions(context);
    context.register_udf(datafusion::logical_expr::ScalarUDF::from(
        Wgs84FromText::default(),
    ));
}

/// A WKB geometry result, scalar when the function was called with a scalar.
fn wkb_result(
    scalar: bool,
    values: Vec<Option<Vec<u8>>>,
) -> datafusion::logical_expr::ColumnarValue {
    use datafusion::{common::ScalarValue, logical_expr::ColumnarValue};
    if scalar {
        ColumnarValue::Scalar(ScalarValue::Binary(values.into_iter().next().flatten()))
    } else {
        ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            values.iter().map(|value| value.as_deref()),
        )))
    }
}

const CONVERSE_RELATIONS: [(&str, &str); 4] = [
    ("st_contains", "st_within"),
    ("st_within", "st_contains"),
    ("st_covers", "st_coveredby"),
    ("st_coveredby", "st_covers"),
];

fn register_ordered_relations(context: &datafusion::prelude::SessionContext) {
    use datafusion::execution::FunctionRegistry;
    let originals: HashMap<&str, Arc<datafusion::logical_expr::ScalarUDF>> = CONVERSE_RELATIONS
        .iter()
        .filter_map(|(name, _)| Some((*name, context.udf(name).ok()?)))
        .collect();
    for (name, converse) in CONVERSE_RELATIONS {
        if let (Some(relation), Some(converse)) = (originals.get(name), originals.get(converse)) {
            context.register_udf(datafusion::logical_expr::ScalarUDF::from(OrderedRelation {
                relation: relation.clone(),
                converse: converse.clone(),
            }));
        }
    }
}

/// geodatafusion 0.4 evaluates a constant first argument against a column as
/// `column.relate(constant)` without transposing the matrix, so asymmetric
/// relations answer their converse. Swapping the arguments into the converse
/// relation routes the call through its correct `(column, constant)` path.
#[derive(Debug, PartialEq, Eq, Hash)]
struct OrderedRelation {
    relation: Arc<datafusion::logical_expr::ScalarUDF>,
    converse: Arc<datafusion::logical_expr::ScalarUDF>,
}

impl datafusion::logical_expr::ScalarUDFImpl for OrderedRelation {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn name(&self) -> &str {
        self.relation.name()
    }
    fn signature(&self) -> &datafusion::logical_expr::Signature {
        self.relation.signature()
    }
    fn return_type(&self, arg_types: &[DataType]) -> datafusion::error::Result<DataType> {
        self.relation.return_type(arg_types)
    }
    fn invoke_with_args(
        &self,
        mut args: datafusion::logical_expr::ScalarFunctionArgs,
    ) -> datafusion::error::Result<datafusion::logical_expr::ColumnarValue> {
        use datafusion::logical_expr::ColumnarValue;
        if let [ColumnarValue::Scalar(_), ColumnarValue::Array(_)] = args.args.as_slice() {
            args.args.swap(0, 1);
            args.arg_fields.swap(0, 1);
            return self.converse.invoke_with_args(args);
        }
        self.relation.invoke_with_args(args)
    }
    fn documentation(&self) -> Option<&datafusion::logical_expr::Documentation> {
        self.relation.documentation()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct Wgs84FromText {
    signature: datafusion::logical_expr::Signature,
}
impl Default for Wgs84FromText {
    fn default() -> Self {
        use datafusion::logical_expr::TypeSignature;
        Self {
            signature: datafusion::logical_expr::Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Utf8]),
                    TypeSignature::Exact(vec![DataType::Binary]),
                ],
                datafusion::logical_expr::Volatility::Immutable,
            ),
        }
    }
}
impl datafusion::logical_expr::ScalarUDFImpl for Wgs84FromText {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn name(&self) -> &str {
        "flow_geomfromtext"
    }
    fn signature(&self) -> &datafusion::logical_expr::Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(DataType::Binary)
    }
    fn return_field_from_args(
        &self,
        _: datafusion::logical_expr::ReturnFieldArgs,
    ) -> datafusion::error::Result<Arc<Field>> {
        Ok(Arc::new(geometry_field("flow_geomfromtext", true)))
    }
    fn invoke_with_args(
        &self,
        args: datafusion::logical_expr::ScalarFunctionArgs,
    ) -> datafusion::error::Result<datafusion::logical_expr::ColumnarValue> {
        use datafusion::{error::DataFusionError, logical_expr::ColumnarValue};
        // A bound Geometry parameter already is a WGS 84 geometry.
        if args.args[0].data_type() == DataType::Binary {
            let field = &args.arg_fields[0];
            if !is_geometry_field(field) {
                return Err(DataFusionError::Execution(
                    "flow_geomfromtext takes WKT text or a WGS84 geometry".into(),
                ));
            }
            validate_crs(field).map_err(|e| DataFusionError::Execution(e.to_string()))?;
            return Ok(args.args.into_iter().next().expect("one argument"));
        }
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let input = arrays[0]
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .ok_or_else(|| {
                DataFusionError::Execution("flow_geomfromtext requires WKT text".into())
            })?;
        let values = input
            .iter()
            .map(|text| {
                text.map(|text| {
                    flow_like_geometry::from_wkt(text)
                        .and_then(|value| flow_like_geometry::to_wkb(&value))
                })
                .transpose()
            })
            .collect::<Result<Vec<_>>>()
            .map_err(|e| DataFusionError::Execution(e.to_string()))?;
        Ok(wkb_result(
            matches!(args.args[0], ColumnarValue::Scalar(_)),
            values,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn binary_is_not_implicitly_geometry() {
        assert!(!is_geometry_field(&Field::new(
            "bytes",
            DataType::Binary,
            true
        )));
        assert!(validate_crs(&Field::new("unknown", DataType::Binary, true)).is_err());
    }
    fn square() -> Value {
        json!({"type": "Polygon", "coordinates": [[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]]]})
    }

    #[test]
    fn geometry_input_accepts_geometries_features_and_text() -> Result<()> {
        let point = json!({"type": "Point", "coordinates": [13.405, 52.52]});
        let annotated = json!({
            "type": "GeometryCollection",
            "bbox": [0.0, 0.0, 4.0, 4.0],
            "name": "site",
            "geometries": [{"type": "Point", "coordinates": [13.405, 52.52], "id": 7}]
        });
        for (input, expected) in [
            (point.clone(), point.clone()),
            (
                json!({"type": "Feature", "id": 3, "geometry": square(), "properties": {"a": 1}}),
                square(),
            ),
            (Value::String(square().to_string()), square()),
            (
                Value::String(json!({"type": "Feature", "geometry": point}).to_string()),
                point.clone(),
            ),
            (json!(" POINT (13.405 52.52) "), point.clone()),
            (json!("POLYGON ((0 0, 4 0, 4 4, 0 4, 0 0))"), square()),
            (
                annotated,
                json!({"type": "GeometryCollection", "geometries": [point]}),
            ),
            (Value::Null, Value::Null),
            (
                json!({"type": "Feature", "geometry": null, "properties": {}}),
                Value::Null,
            ),
        ] {
            assert_eq!(geometry_input(&input)?, expected, "{input}");
        }
        assert_eq!(
            geometry_input_wkb(&json!("POINT (1 2)"))?,
            Some(flow_like_geometry::to_wkb(
                &json!({"type": "Point", "coordinates": [1.0, 2.0]})
            )?)
        );
        assert_eq!(geometry_input_wkb(&Value::Null)?, None);
        Ok(())
    }

    #[test]
    fn geometry_input_rejections_name_the_input_and_the_accepted_forms() {
        let rejected = |value: Value| geometry_input(&value).unwrap_err().to_string();

        let collection = rejected(json!({"type": "FeatureCollection", "features": []}));
        assert!(collection.contains("one row per feature"), "{collection}");

        for (value, received) in [
            (json!(42), "a number"),
            (json!([13.4, 52.5]), "an array"),
            (json!({"type": "Circle", "radius": 3}), "type 'Circle'"),
            (json!({"coordinates": [1, 2]}), "without a GeoJSON type"),
        ] {
            let message = rejected(value);
            assert!(message.contains(received), "{message}");
            assert!(message.contains("WKT text"), "{message}");
        }

        let z = rejected(json!({"type": "Point", "coordinates": [1.0, 2.0, 3.0]}));
        assert!(z.contains("Z/M"), "{z}");
        let wkt_z = rejected(json!("POINT Z (1 2 3)"));
        assert!(wkt_z.contains("two-dimensional"), "{wkt_z}");
        let feature = rejected(json!({"type": "Feature", "properties": {}}));
        assert!(feature.contains("without a geometry member"), "{feature}");
        let nested = rejected(json!({"type": "Feature", "geometry": {"type": "Feature"}}));
        assert!(nested.contains("type 'Feature'"), "{nested}");
        let text = rejected(json!("not a geometry"));
        assert!(text.contains("neither GeoJSON nor valid WKT"), "{text}");
        let broken = rejected(json!("{\"type\": \"Point\""));
        assert!(broken.contains("not valid GeoJSON"), "{broken}");
        assert!(rejected(json!("  ")).contains("use null"));
        let crs = rejected(json!({"type": "Point", "coordinates": [1.0, 2.0], "crs": {}}));
        assert!(crs.contains("CRS"), "{crs}");
    }

    fn shapes_context() -> Result<datafusion::prelude::SessionContext> {
        let ctx = datafusion::prelude::SessionContext::new();
        register_geo_functions(&ctx);
        let batch = crate::arrow_utils::value_to_record_batch_with_fields(
            vec![
                json!({"id": 1, "geom": square(), "text": square().to_string()}),
                json!({"id": 2, "geom": null, "text": null}),
            ],
            Some(vec![
                Arc::new(Field::new("id", DataType::Int64, false)),
                Arc::new(geometry_field("geom", true)),
                Arc::new(Field::new("text", DataType::LargeUtf8, true)),
            ]),
        )?;
        ctx.register_batch("shapes", batch)?;
        Ok(ctx)
    }

    async fn sql_rows(ctx: &datafusion::prelude::SessionContext, sql: &str) -> Result<Vec<Value>> {
        let batches = ctx.sql(sql).await?.collect().await?;
        Ok(batches
            .iter()
            .map(crate::arrow_utils::record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat())
    }

    #[tokio::test]
    async fn geojson_sql_functions_round_trip_and_st_astext_reads_as_wkt() -> Result<()> {
        let ctx = shapes_context()?;
        let converted = sql_rows(
            &ctx,
            "SELECT ST_AsGeoJSON(geom) AS json, ST_GeomFromGeoJSON(text) AS parsed, \
             ST_AsGeoJSON(ST_GeomFromGeoJSON(ST_AsGeoJSON(geom))) AS round_trip, \
             ST_AsText(geom) AS wkt FROM shapes ORDER BY id",
        )
        .await?;
        let json_text = converted[0]["json"].as_str().expect("GeoJSON text");
        assert_eq!(serde_json::from_str::<Value>(json_text)?, square());
        assert_eq!(converted[0]["parsed"], square());
        assert_eq!(converted[0]["round_trip"], converted[0]["json"]);
        let wkt = converted[0]["wkt"].as_str().expect("WKT text");
        assert_eq!(flow_like_geometry::from_wkt(wkt)?, square());
        for column in ["json", "parsed", "round_trip", "wkt"] {
            assert_eq!(converted[1][column], Value::Null, "{column}");
        }

        let text = ctx
            .sql("SELECT ST_AsText(geom) AS wkt, ST_AsGeoJSON(geom) AS json FROM shapes")
            .await?;
        for field in text.schema().fields() {
            assert_eq!(field.data_type(), &DataType::Utf8, "{field:?}");
            assert!(!field.metadata().contains_key(EXTENSION_NAME), "{field:?}");
        }
        Ok(())
    }

    #[test]
    fn stored_wkt_geometry_columns_read_as_geojson() -> Result<()> {
        let field = Field::new("outline", DataType::Utf8, true).with_metadata(HashMap::from([
            (EXTENSION_NAME.into(), "geoarrow.wkt".into()),
            (EXTENSION_METADATA.into(), WGS84_METADATA.into()),
        ]));
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![field])),
            vec![Arc::new(arrow_array::StringArray::from(vec![
                Some("POLYGON ((0 0, 4 0, 4 4, 0 4, 0 0))"),
                None,
            ]))],
        )?;
        assert_eq!(
            crate::arrow_utils::record_batch_to_value(&batch)?,
            vec![json!({"outline": square()}), json!({"outline": null})]
        );
        Ok(())
    }

    #[tokio::test]
    async fn geojson_sql_functions_take_scalars_and_reject_non_geometry() -> Result<()> {
        let ctx = shapes_context()?;
        let scalars = sql_rows(
            &ctx,
            "SELECT ST_AsGeoJSON(flow_geomfromtext('POINT(1 2)')) AS json, \
             ST_GeomFromGeoJSON('{\"type\":\"Feature\",\"geometry\":{\"type\":\"Point\",\"coordinates\":[1,2]}}') AS feature, \
             ST_GeomFromGeoJSON('POINT (1 2)') AS wkt, \
             ST_Area(ST_GeomFromGeoJSON(text)) AS area FROM shapes WHERE id = 1",
        )
        .await?;
        let point = json!({"type": "Point", "coordinates": [1.0, 2.0]});
        assert_eq!(
            serde_json::from_str::<Value>(scalars[0]["json"].as_str().unwrap_or_default())?,
            point
        );
        assert_eq!(scalars[0]["feature"], point);
        assert_eq!(scalars[0]["wkt"], point);
        assert_eq!(scalars[0]["area"], json!(16.0));

        let error = ctx
            .sql("SELECT ST_GeomFromGeoJSON('{\"type\":\"FeatureCollection\",\"features\":[]}') AS g")
            .await;
        let error = match error {
            Ok(frame) => frame.collect().await.map(|_| ()).unwrap_err().to_string(),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("one row per feature"), "{error}");
        let error = ctx
            .sql("SELECT ST_AsGeoJSON(text) FROM shapes")
            .await
            .map(|_| ())
            .unwrap_err()
            .to_string();
        assert!(error.contains("ST_GeomFromGeoJSON"), "{error}");
        Ok(())
    }

    #[test]
    fn geometry_metadata_accepts_explicit_planar_edges() {
        let field = |edges: &str| {
            geometry_field("geom", true).with_metadata(HashMap::from([
                (EXTENSION_NAME.into(), "geoarrow.wkb".into()),
                (
                    EXTENSION_METADATA.into(),
                    format!(r#"{{"crs":"EPSG:4326","edges":"{edges}"}}"#),
                ),
            ]))
        };
        assert!(validate_crs(&field("planar")).is_ok());
        assert!(validate_crs(&field("spherical")).is_err());
    }

    #[test]
    fn geometry_nested_writes_are_rejected_and_null_parents_remain_null() -> Result<()> {
        let child = Arc::new(geometry_field("geom", true));
        let children = arrow_schema::Fields::from(vec![child.clone()]);
        let nested = arrow_array::StructArray::new(
            children.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(&[1_u8, 2][..])]))],
            Some(arrow::buffer::NullBuffer::from(vec![false])),
        );
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![Field::new(
                "nested",
                DataType::Struct(children),
                true,
            )])),
            vec![Arc::new(nested)],
        )?;
        assert!(validate_batch(&batch).is_err());
        assert_eq!(
            crate::arrow_utils::record_batch_to_value(&batch)?,
            vec![json!({"nested":null})]
        );
        let fields = batch.schema().fields().to_vec();
        assert!(
            crate::arrow_utils::value_to_record_batch_with_fields(
                vec![json!({"nested":null})],
                Some(fields)
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn geometry_native_writes_cannot_relabel_the_same_physical_layout() -> Result<()> {
        use arrow_array::{Float64Array, ListArray, StructArray};
        let xy = arrow_schema::Fields::from(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]);
        let points = StructArray::new(
            xy.clone(),
            vec![
                Arc::new(Float64Array::from(vec![1., 3.])),
                Arc::new(Float64Array::from(vec![2., 4.])),
            ],
            None,
        );
        let vertices = Arc::new(Field::new("vertices", DataType::Struct(xy), false));
        let line = ListArray::new(
            vertices.clone(),
            arrow::buffer::OffsetBuffer::new(vec![0, 2].into()),
            Arc::new(points),
            None,
        );
        let field = |kind: &str| {
            Field::new("geom", DataType::List(vertices.clone()), true).with_metadata(HashMap::from(
                [
                    (EXTENSION_NAME.into(), kind.into()),
                    (EXTENSION_METADATA.into(), WGS84_METADATA.into()),
                ],
            ))
        };
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![field(
                "geoarrow.linestring",
            )])),
            vec![Arc::new(line)],
        )?;
        validate_batch(&batch)?;
        let target = Arc::new(arrow_schema::Schema::new(vec![field(
            "geoarrow.multipoint",
        )]));
        let error = normalize_batch(&batch, &target).unwrap_err().to_string();
        assert!(error.contains("matching geometry kinds"), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn geometry_arrow_inserts_use_authoritative_metadata() -> Result<()> {
        let ctx = datafusion::prelude::SessionContext::new();
        register_geo_functions(&ctx);
        let batches = ctx
            .sql("SELECT ST_Centroid(flow_geomfromtext('POINT(13 52)')) AS geom")
            .await?
            .collect()
            .await?;
        let target = Arc::new(arrow_schema::Schema::new(vec![geometry_field(
            "geom", true,
        )]));
        let normalized = normalize_batch(&batches[0], &target)?;
        assert_eq!(normalized.schema(), target);
        assert_eq!(
            decode_column(normalized.column(0).as_ref(), normalized.schema().field(0))?[0],
            json!({"type":"Point","coordinates":[13.,52.]})
        );
        let untagged = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![Field::new(
                "geom",
                DataType::Binary,
                true,
            )])),
            normalized.columns().to_vec(),
        )?;
        assert!(normalize_batch(&untagged, &target).is_err());
        let unknown = ctx
            .sql("SELECT ST_GeomFromText('POINT(13 52)') AS geom")
            .await?
            .collect()
            .await?;
        assert!(normalize_batch(&unknown[0], &target).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn asymmetric_relations_hold_in_both_argument_orders() -> Result<()> {
        let ctx = datafusion::prelude::SessionContext::new();
        register_geo_functions(&ctx);
        let batch = crate::arrow_utils::value_to_record_batch_with_fields(
            vec![
                json!({"id": 1, "geom": {"type": "Point", "coordinates": [2.0, 2.0]}}),
                json!({"id": 2, "geom": {"type": "Point", "coordinates": [9.0, 9.0]}}),
                json!({"id": 3, "geom": {"type": "Polygon", "coordinates": [[[-1.0, -1.0], [5.0, -1.0], [5.0, 5.0], [-1.0, 5.0], [-1.0, -1.0]]]}}),
            ],
            Some(vec![
                Arc::new(Field::new("id", DataType::Int64, false)),
                Arc::new(geometry_field("geom", true)),
            ]),
        )?;
        ctx.register_batch("shapes", batch)?;

        let area = "flow_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')";
        for (predicate, expected) in [
            (format!("ST_Contains({area}, geom)"), vec![1]),
            (format!("ST_Within(geom, {area})"), vec![1]),
            (format!("ST_Covers({area}, geom)"), vec![1]),
            (format!("ST_CoveredBy(geom, {area})"), vec![1]),
            (format!("ST_Within({area}, geom)"), vec![3]),
            (format!("ST_Contains(geom, {area})"), vec![3]),
            (format!("ST_CoveredBy({area}, geom)"), vec![3]),
            (format!("ST_Covers(geom, {area})"), vec![3]),
        ] {
            let sql = format!("SELECT id FROM shapes WHERE {predicate} ORDER BY id");
            let batches = ctx.sql(&sql).await?.collect().await?;
            let ids: Vec<i64> = batches
                .iter()
                .map(crate::arrow_utils::record_batch_to_value)
                .collect::<Result<Vec<_>>>()?
                .concat()
                .iter()
                .filter_map(|row| row["id"].as_i64())
                .collect();
            assert_eq!(ids, expected, "{predicate}");
        }

        let batches = ctx
            .sql(&format!(
                "SELECT ST_Contains({area}, flow_geomfromtext('POINT(2 2)')) AS inside"
            ))
            .await?
            .collect()
            .await?;
        assert_eq!(
            crate::arrow_utils::record_batch_to_value(&batches[0])?[0]["inside"],
            json!(true)
        );
        Ok(())
    }

    #[tokio::test]
    async fn nested_results_keep_geometry_through_structs_and_array_agg() -> Result<()> {
        let ctx = datafusion::prelude::SessionContext::new();
        register_geo_functions(&ctx);
        let batch = crate::arrow_utils::value_to_record_batch_with_fields(
            vec![
                json!({"id": 1, "layer_id": "a", "geometry": {"type": "Point", "coordinates": [1.0, 1.0]}}),
                json!({"id": 2, "layer_id": "a", "geometry": {"type": "Point", "coordinates": [2.0, 2.0]}}),
                json!({"id": 3, "layer_id": "b", "geometry": {"type": "Point", "coordinates": [3.0, 3.0]}}),
                json!({"id": 4, "layer_id": "b", "geometry": {"type": "Point", "coordinates": [9.0, 9.0]}}),
            ],
            Some(vec![
                Arc::new(Field::new("id", DataType::Int64, false)),
                Arc::new(Field::new("layer_id", DataType::Utf8, false)),
                Arc::new(geometry_field("geometry", true)),
            ]),
        )?;
        ctx.register_batch("entities", batch)?;
        let point = |x: f64| json!({"type": "Point", "coordinates": [x, x]});
        let rows = |sql: &'static str| {
            let ctx = ctx.clone();
            async move {
                let batches = ctx.sql(sql).await?.collect().await?;
                Ok::<_, flow_like_types::Error>(
                    batches
                        .iter()
                        .map(crate::arrow_utils::record_batch_to_value)
                        .collect::<Result<Vec<_>>>()?
                        .concat(),
                )
            }
        };

        assert_eq!(
            rows("WITH viewport AS (SELECT ST_GeomFromText('POLYGON((0 0,5 0,5 5,0 5,0 0))') AS geometry) \
                  SELECT e.layer_id, ARRAY_AGG(NAMED_STRUCT('id', e.id, 'geometry', e.geometry)) AS features \
                  FROM entities e CROSS JOIN viewport v WHERE ST_Intersects(e.geometry, v.geometry) \
                  GROUP BY e.layer_id ORDER BY e.layer_id")
            .await?,
            vec![
                json!({"layer_id": "a", "features": [{"id": 1, "geometry": point(1.)}, {"id": 2, "geometry": point(2.)}]}),
                json!({"layer_id": "b", "features": [{"id": 3, "geometry": point(3.)}]}),
            ]
        );
        assert_eq!(
            rows("SELECT row(geometry) AS s FROM entities WHERE id = 1").await?,
            vec![json!({"s": {"c0": point(1.)}})]
        );
        assert_eq!(
            rows("SELECT ARRAY_AGG(geometry ORDER BY id DESC) AS g FROM entities WHERE id < 3")
                .await?,
            vec![json!({"g": [point(2.), point(1.)]})]
        );
        assert_eq!(
            rows("SELECT ARRAY_AGG(DISTINCT geometry) AS g FROM entities WHERE id = 3").await?,
            vec![json!({"g": [point(3.)]})]
        );
        assert_eq!(
            rows(
                "SELECT ARRAY_AGG(geometry) OVER (PARTITION BY layer_id ORDER BY id) AS g \
                  FROM entities WHERE layer_id = 'a' ORDER BY id"
            )
            .await?,
            vec![
                json!({"g": [point(1.)]}),
                json!({"g": [point(1.), point(2.)]})
            ]
        );
        assert_eq!(
            rows("SELECT ARRAY_AGG(id) AS ids FROM entities").await?[0]["ids"],
            json!([1, 2, 3, 4])
        );
        Ok(())
    }

    #[tokio::test]
    async fn native_centroid_and_wkb_aliases_preserve_values_and_crs() -> Result<()> {
        let ctx = datafusion::prelude::SessionContext::new();
        register_geo_functions(&ctx);
        let batches = ctx.sql("SELECT ST_Centroid(flow_geomfromtext('LINESTRING(10 20, 20 30)')) AS center, ST_AsBinary(flow_geomfromtext('POINT(13.405 52.52)')) AS position").await?.collect().await?;
        assert_eq!(
            decode_column(batches[0].column(0).as_ref(), batches[0].schema().field(0))?[0],
            json!({"type":"Point","coordinates":[15.,25.]})
        );
        assert_eq!(
            decode_column(batches[0].column(1).as_ref(), batches[0].schema().field(1))?[0],
            json!({"type":"Point","coordinates":[13.405,52.52]})
        );
        let unknown = ctx
            .sql("SELECT ST_GeomFromText('POINT(1 2)')")
            .await?
            .collect()
            .await?;
        assert!(
            decode_column(unknown[0].column(0).as_ref(), unknown[0].schema().field(0)).is_err()
        );
        Ok(())
    }
}
