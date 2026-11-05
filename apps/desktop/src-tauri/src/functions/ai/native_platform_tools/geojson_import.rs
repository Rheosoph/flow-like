use super::{
    NativePlatformSession,
    database::{self, Database},
    required, select_forwarded_attachments, string, table_metadata,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

const MAX_FILE: usize = 50 * 1024 * 1024;
const RESERVED: [&str; 4] = ["_rowid", "_distance", "_relevance_score", "__proto__"];
struct Feature {
    index: usize,
    id: Option<String>,
    properties: Map<String, Value>,
    geometry: Value,
    dropped: bool,
}
struct Column {
    name: String,
    property: String,
    renamed: Option<String>,
    kinds: BTreeSet<&'static str>,
    integers: bool,
    timestamps: bool,
    kind: &'static str,
}
struct Plan {
    fields: Value,
    rows: Vec<Value>,
    indices: Vec<usize>,
    columns: Value,
    skipped: Vec<Value>,
    warnings: Vec<String>,
    features: usize,
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
    db: &Database<'_>,
    table: &str,
    requested_table: &str,
    user_scoped: bool,
) -> Result<Value, String> {
    let file_name = required(args, "file_name", "fileName")?;
    let files = super::super::native_tool_bridge::run_options(session.context.as_ref()).attachments;
    let file = select_forwarded_attachments(&files, Some(&json!([file_name])))?
        .into_iter()
        .next()
        .ok_or("No file was explicitly forwarded to this run.")?;
    let display = file
        .name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            file.url
                .as_deref()
                .and_then(|url| url.split('?').next()?.rsplit('/').next())
        })
        .unwrap_or(&file_name)
        .to_owned();
    if file.size.is_some_and(|size| size > MAX_FILE as u64) {
        return Err(format!(
            "import_geojson refuses '{display}': it exceeds the 50 MiB import limit. Split the file and import the parts."
        ));
    }
    let url = reqwest::Url::parse(file.url.as_deref().unwrap_or(""))
        .map_err(|_| "The forwarded file URL is invalid.")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "The forwarded file needs a transferable HTTP or HTTPS URL for native import.".into(),
        );
    }
    session.assert_current_profile().await?;
    let mut response = session
        .client()
        .get(url)
        .send()
        .await
        .map_err(|_| "The forwarded file could not be downloaded.")?;
    if !response.status().is_success() {
        return Err(format!(
            "import_geojson could not download '{display}' (HTTP {}).",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_FILE as u64)
    {
        return Err("The forwarded file exceeds the 50 MiB import limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The forwarded file download was interrupted.")?
    {
        session.assert_current_profile().await?;
        if bytes.len().saturating_add(chunk.len()) > MAX_FILE {
            return Err("The forwarded file exceeds the 50 MiB import limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let parsed: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("import_geojson could not parse '{display}' as JSON: {error}"))?;
    let geometry = string(args, "geometry_column", "geometryColumn").unwrap_or("geometry");
    let key = string(args, "key_column", "keyColumn").unwrap_or("feature_id");
    let mut plan = plan(&parsed, key, geometry)?;
    if plan.rows.is_empty() {
        return Err(format!(
            "import_geojson found no importable feature in '{display}' ({} feature(s); {}).",
            plan.features,
            plan.skipped
                .iter()
                .take(3)
                .map(|s| format!(
                    "#{}: {}",
                    s["feature_index"],
                    s["reason"].as_str().unwrap_or("")
                ))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    let app = session.app_id(args)?;
    let existing = match db.read("schema", Value::Null, 0, 0).await {
        Ok(schema) => Some(schema),
        Err(error) => {
            let names = table_metadata(session, &app, user_scoped, None).await?;
            if names
                .as_array()
                .is_some_and(|names| names.iter().any(|name| name.as_str() == Some(table)))
            {
                return Err(error);
            }
            None
        }
    };
    let mut created = false;
    if let Some(schema) = existing {
        assert_target(table, &schema, &plan, geometry)?;
    } else {
        let result = database::create_schema(db, &plan.fields, true).await?;
        if result["status"] == "partial" {
            plan.warnings.push("Explicit table creation is unavailable on the connected API, so the first insert creates the table from the rows; it has no primary key.".into());
        } else {
            created = result["created"].as_bool().unwrap_or(false);
            if !created {
                let schema = db.read("schema", Value::Null, 0, 0).await?;
                assert_target(table, &schema, &plan, geometry)?;
            }
        }
    }
    if plan.skipped.len() > 100 {
        plan.warnings.push(format!(
            "{} features were skipped; only the first 100 are listed.",
            plan.skipped.len()
        ));
    }
    let mut inserted = 0;
    let mut error = None;
    let mut failed = None;
    for (start, end) in batches(&plan.rows) {
        if let Err(failure) = db
            .write("insert", json!({"items":&plan.rows[start..end]}))
            .await
        {
            error = Some(failure);
            failed = Some(plan.indices[start]);
            break;
        }
        inserted += end - start;
    }
    let mut result = json!({
    "status":if error.is_some(){
    "partial"}
    else{
    "ok"}
    ,"table_name":table,"created":created,"user_scoped":user_scoped,"file_name":display,"features":plan.features,"rows_inserted":inserted,"key_column":key,"geometry_column":geometry,"columns":plan.columns,"skipped":plan.skipped.into_iter().take(100).collect::<Vec<_>>(),"warnings":plan.warnings}
    );

    if table != requested_table {
        result["requested_table_name"] = json!(requested_table);
    }
    if let Some(error) = error {
        result["error"] = json!(error);
        result["failed_feature_index"] = json!(failed);
    }
    Ok(result)
}
fn assert_target(table: &str, schema: &Value, plan: &Plan, geometry: &str) -> Result<(), String> {
    let columns = database::summarize_columns(schema)?;
    let columns = columns.as_array().ok_or("Invalid table schema")?;
    let missing = plan
        .fields
        .as_array()
        .unwrap()
        .iter()
        .filter(|field| !columns.iter().any(|column| column["name"] == field["name"]))
        .map(|field| {
            format!(
                "{} ({})",
                field["name"].as_str().unwrap_or(""),
                field["type"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "Table '{table}' lacks column(s) this import needs: {}. Add them with add_column, or import into a new table_name.",
            missing.join(", ")
        ));
    }
    if !columns
        .iter()
        .any(|column| column["name"] == geometry && column["type"] == "geometry")
    {
        return Err(format!(
            "Column '{geometry}' of table '{table}' is not a geometry column. Pass geometry_column naming a geometry column, or add one with add_column {{type: \"geometry\"}}."
        ));
    }
    Ok(())
}

fn valid_column(name: &str) -> bool {
    name.len() <= 128
        && name
            .as_bytes()
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !RESERVED.contains(&name.to_ascii_lowercase().as_str())
}
fn property_name(property: &str) -> String {
    let normalized = property
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<Vec<_>>();
    let mut snake = String::new();
    for (i, c) in normalized.iter().copied().enumerate() {
        let previous = i.checked_sub(1).and_then(|i| normalized.get(i)).copied();
        let next = normalized.get(i + 1).copied();
        if c.is_ascii_uppercase()
            && (previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
                || previous.is_some_and(|p| p.is_ascii_uppercase())
                    && next.is_some_and(|n| n.is_ascii_lowercase()))
        {
            snake.push('_');
        }
        if c.is_ascii_alphanumeric() {
            snake.push(c.to_ascii_lowercase());
        } else if !snake.ends_with('_') {
            snake.push('_');
        }
    }
    let name = snake
        .trim_matches('_')
        .chars()
        .take(120)
        .collect::<String>();
    let name = name.trim_end_matches('_');
    let name = if name.is_empty() { "property" } else { name };
    if name.as_bytes()[0].is_ascii_digit() {
        format!("_{name}")
    } else {
        name.into()
    }
}
fn datetime(value: &str) -> bool {
    if value.len() < 20 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !bytes.get(10).is_some_and(|b| matches!(b, b'T' | b't'))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || value
            .get(17..19)
            .and_then(|s| s.parse::<u8>().ok())
            .is_none_or(|s| s > 59)
    {
        return false;
    }
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}
fn crs(value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let name = value["properties"]["name"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            value["properties"].get("code").map(|code| {
                format!(
                    "{}:{}",
                    value["type"].as_str().unwrap_or("EPSG"),
                    js_string(code)
                )
            })
        });
    let known = [
        "crs84",
        "ogc:crs84",
        "urn:ogc:def:crs:ogc:1.3:crs84",
        "urn:ogc:def:crs:ogc::crs84",
        "http://www.opengis.net/def/crs/ogc/1.3/crs84",
        "epsg:4326",
        "urn:ogc:def:crs:epsg::4326",
        "urn:ogc:def:crs:epsg:4326",
        "http://www.opengis.net/def/crs/epsg/0/4326",
    ];
    if name
        .as_ref()
        .is_some_and(|n| known.contains(&n.trim().to_lowercase().as_str()))
    {
        None
    } else {
        Some(name.unwrap_or_else(|| value.to_string().chars().take(200).collect()))
    }
}
fn flatten_positions(value: &mut Value, dropped: &mut bool) {
    if let Some(array) = value.as_array_mut() {
        if array.iter().all(Value::is_number) {
            if array.len() > 2 {
                array.truncate(2);
                *dropped = true;
            }
        } else {
            for value in array {
                flatten_positions(value, dropped);
            }
        }
    }
}
fn flatten_geometry(value: &mut Value, dropped: &mut bool) {
    if let Some(object) = value.as_object_mut() {
        object.remove("bbox");
        object.remove("crs");
        if let Some(coordinates) = object.get_mut("coordinates") {
            flatten_positions(coordinates, dropped);
        }
        if let Some(geometries) = object.get_mut("geometries").and_then(Value::as_array_mut) {
            for value in geometries {
                flatten_geometry(value, dropped);
            }
        }
    }
}
fn feature(value: &Value, index: usize) -> Result<Feature, String> {
    if !value.is_object() || value["type"] != "Feature" {
        return Err("expected a GeoJSON Feature".into());
    }
    if let Some(crs) = crs(&value["crs"]) {
        return Err(format!(
            "feature declares crs '{crs}'; only WGS 84 longitude/latitude is supported"
        ));
    }
    let properties = if value["properties"].is_null() {
        Map::new()
    } else {
        value["properties"]
            .as_object()
            .cloned()
            .ok_or("properties must be an object or null")?
    };
    let mut geometry = value["geometry"].clone();
    let mut dropped = false;
    if !geometry.is_null() {
        if !geometry.is_object()
            || !matches!(
                geometry["type"].as_str(),
                Some(
                    "Point"
                        | "LineString"
                        | "Polygon"
                        | "MultiPoint"
                        | "MultiLineString"
                        | "MultiPolygon"
                        | "GeometryCollection"
                )
            )
        {
            return Err("geometry must be a GeoJSON geometry object or null".into());
        }
        if let Some(crs) = crs(&geometry["crs"]) {
            return Err(format!(
                "geometry declares crs '{crs}'; only WGS 84 longitude/latitude is supported"
            ));
        }
        flatten_geometry(&mut geometry, &mut dropped);
        geometry = flow_like_types::geometry::canonicalize_geometry(&geometry, None)
            .map_err(|error| format!("invalid geometry: {error}"))?;
    }
    let id = match &value["id"] {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(js_string(&json!(n))),
        _ => None,
    };
    Ok(Feature {
        index,
        id,
        properties,
        geometry,
        dropped,
    })
}
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n)
            if n.as_f64()
                .is_some_and(|n| n.fract() == 0.0 && n.abs() < 1e21) =>
        {
            format!("{:.0}", n.as_f64().unwrap())
        }
        _ => value.to_string(),
    }
}
fn plan(value: &Value, key: &str, geometry: &str) -> Result<Plan, String> {
    for (option, name) in [("key_column", key), ("geometry_column", geometry)] {
        if !valid_column(name) {
            return Err(format!(
                "import_geojson {option} '{name}' is not a valid column name (ASCII letters, digits and underscores, not starting with a digit, at most 128 characters)."
            ));
        }
    }
    if key.eq_ignore_ascii_case(geometry) {
        return Err(format!(
            "import_geojson key_column and geometry_column must differ (both are '{key}')."
        ));
    }
    let singleton;
    let values = if let Some(array) = value.as_array() {
        array
    } else {
        if let Some(crs) = crs(&value["crs"]) {
            return Err(format!(
                "The GeoJSON declares crs '{crs}', but import_geojson only reads WGS 84 longitude/latitude (CRS84 / EPSG:4326). Reproject the file to WGS 84 and attach it again."
            ));
        }
        match value["type"].as_str(){
Some("FeatureCollection")=>value["features"].as_array().ok_or("The GeoJSON FeatureCollection has no 'features' array.")?,Some("Feature")=>{
singleton=vec![value.clone()];
&singleton}
,_=>return Err("import_geojson expects a GeoJSON FeatureCollection, a Feature, or an array of Features.".into())}
    };
    let mut features = Vec::new();
    let mut skipped = Vec::new();
    for (index, value) in values.iter().enumerate() {
        match feature(value, index) {
            Ok(feature) => features.push(feature),
            Err(reason) => skipped.push(json!({"feature_index":index,"reason":reason})),
        }
    }
    let mut columns = Vec::<Column>::new();
    let mut locations = HashMap::new();
    let mut taken = HashSet::from([key.to_lowercase(), geometry.to_lowercase()]);
    taken.extend(RESERVED.iter().map(|s| s.to_string()));
    for feature in &features {
        for (property, value) in &feature.properties {
            let position = if let Some(position) = locations.get(property) {
                *position
            } else {
                if columns.len() >= 254 {
                    return Err("The GeoJSON properties need more than 256 columns. Remove unneeded properties from the file first.".into());
                }
                let base = property_name(property);
                let mut name = base.clone();
                let mut suffix = 2;
                while taken.contains(&name.to_lowercase()) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                taken.insert(name.to_lowercase());
                let position = columns.len();
                columns.push(Column {
                    name: name.clone(),
                    property: property.clone(),
                    renamed: (name != base).then_some(base),
                    kinds: BTreeSet::new(),
                    integers: true,
                    timestamps: true,
                    kind: "string",
                });
                locations.insert(property.clone(), position);
                position
            };
            let column = &mut columns[position];
            match value {
                Value::Null => {}
                Value::Bool(_) => {
                    column.kinds.insert("boolean");
                }
                Value::Number(number) => {
                    column.kinds.insert("number");
                    column.integers &= number
                        .as_f64()
                        .is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0);
                }
                Value::String(value) => {
                    column.kinds.insert("string");
                    column.timestamps &= datetime(value);
                }
                _ => {
                    column.kinds.insert("json");
                }
            }
        }
    }
    for column in &mut columns {
        column.kind = if column.kinds.len() != 1 {
            "string"
        } else {
            match column.kinds.first().copied() {
                Some("boolean") => "boolean",
                Some("number") if column.integers => "int64",
                Some("number") => "float64",
                Some("string") if column.timestamps => "timestamp:ms:UTC",
                _ => "string",
            }
        };
    }
    let mut taken = features
        .iter()
        .filter_map(|f| f.id.clone())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut keys = vec![String::new(); features.len()];
    let mut duplicates = 0;
    let mut generated = 0;
    for (position, feature) in features.iter().enumerate() {
        if let Some(id) = &feature.id {
            keys[position] = if seen.insert(id.clone()) {
                id.clone()
            } else {
                duplicates += 1;
                suffix_key(id, &mut taken)
            };
        }
    }
    for (position, feature) in features.iter().enumerate() {
        if feature.id.is_none() {
            generated += 1;
            let base = format!("feature-{}", feature.index + 1);
            keys[position] = if taken.insert(base.clone()) {
                base
            } else {
                suffix_key(&base, &mut taken)
            };
        }
    }
    let mut fields = vec![
        json!({"name":key,"type":"string","nullable":false,"primary_key":true}),
        json!({"name":geometry,"type":"geometry","nullable":true}),
    ];
    fields.extend(
        columns
            .iter()
            .map(|column| json!({"name":column.name,"type":column.kind,"nullable":true})),
    );
    let mut summaries = vec![
        json!({"name":key,"type":"string"}),
        json!({"name":geometry,"type":"geometry"}),
    ];
    summaries.extend(columns.iter().map(
        |column| json!({"name":column.name,"type":column.kind,"source_property":column.property}),
    ));
    let rows = features
        .iter()
        .enumerate()
        .map(|(position, feature)| {
            let mut row = Map::new();
            row.insert(key.into(), json!(keys[position]));
            row.insert(geometry.into(), feature.geometry.clone());
            for column in &columns {
                let value = feature
                    .properties
                    .get(&column.property)
                    .unwrap_or(&Value::Null);
                row.insert(
                    column.name.clone(),
                    if column.kind == "string" && !value.is_string() && !value.is_null() {
                        json!(js_string(value))
                    } else {
                        value.clone()
                    },
                );
            }
            Value::Object(row)
        })
        .collect();
    let mut warnings = Vec::new();
    let flattened = features.iter().filter(|f| f.dropped).count();
    if flattened > 0 {
        warnings.push(format!("Dropped Z/M ordinates from {flattened} feature geometr{}; coordinates are stored as 2D longitude/latitude.",if flattened==1{"y"}else{"ies"}));
    }
    if generated > 0 {
        warnings.push(format!("{generated} of {} features had no id; their {key} is feature-<n> (1-based position in the file).",features.len()));
    }
    if duplicates > 0 {
        warnings.push(format!(
            "{duplicates} duplicate feature id(s) were suffixed with -<n> to keep {key} unique."
        ));
    }
    let nulls = features.iter().filter(|f| f.geometry.is_null()).count();
    if nulls > 0 {
        warnings.push(format!(
            "{nulls} feature(s) have a null geometry; {geometry} is null for them."
        ));
    }
    let mixed = columns
        .iter()
        .filter(|c| c.kinds.len() > 1)
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>();
    if !mixed.is_empty() {
        warnings.push(format!(
            "Stored as text because their values mix types: {}.",
            mixed.join(", ")
        ));
    }
    let nested = columns
        .iter()
        .filter(|c| c.kinds.contains("json"))
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>();
    if !nested.is_empty() {
        warnings.push(format!(
            "Nested objects/arrays are stored as JSON text in: {}.",
            nested.join(", ")
        ));
    }
    for column in &columns {
        if let Some(base) = &column.renamed {
            warnings.push(format!(
                "Property '{}' is stored as '{}' because '{base}' is already taken.",
                column.property, column.name
            ));
        }
    }
    Ok(Plan {
        fields: json!(fields),
        rows,
        indices: features.iter().map(|f| f.index).collect(),
        columns: json!(summaries),
        skipped,
        warnings,
        features: values.len(),
    })
}
fn suffix_key(base: &str, taken: &mut HashSet<String>) -> String {
    for suffix in 2.. {
        let key = format!("{base}-{suffix}");
        if taken.insert(key.clone()) {
            return key;
        }
    }
    unreachable!()
}
fn batches(rows: &[Value]) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut bytes = 2;
    for (index, row) in rows.iter().enumerate() {
        let size = row.to_string().len() + 1;
        if index > start && (index - start >= 250 || bytes + size > 1_000_000) {
            result.push((start, index));
            start = index;
            bytes = 2;
        }
        bytes += size;
    }
    if start < rows.len() {
        result.push((start, rows.len()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn property_names_match_web() {
        for (input, expected) in [
            ("physicalSiteId", "physical_site_id"),
            ("HTTPServer", "http_server"),
            ("addr:city", "addr_city"),
            ("2020 value", "_2020_value"),
            ("Café", "cafe"),
        ] {
            assert_eq!(property_name(input), expected);
        }
    }
    #[test]
    fn date_inference_rejects_invalid_calendar_and_leap_seconds() {
        assert!(datetime("2024-02-29T10:20:59Z"));
        assert!(!datetime("2023-02-29T10:20:00Z"));
        assert!(!datetime("2024-01-01T10:20:60Z"));
    }
    #[test]
    fn imported_keys_protect_explicit_ids_and_skip_bad_features() {
        let input = json!([{
        "type":"Feature","geometry":{
        "type":"Point","coordinates":[1,2,3]}
        ,"properties":{
        "feature_id":1}
        }
        ,{
        "type":"Feature","id":"feature-1","geometry":null,"properties":{
        "feature_id":"two"}
        }
        ,{
        "type":"Feature","id":"feature-1","geometry":{
        "type":"Point","coordinates":[200,2]}
        ,"properties":{
        }
        }
        ]);

        let plan = plan(&input, "feature_id", "geometry").unwrap();
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.rows[0]["feature_id"], "feature-1-2");
        assert_eq!(plan.rows[1]["feature_id"], "feature-1");
        assert_eq!(plan.rows[0]["geometry"]["coordinates"], json!([1, 2]));
        assert_eq!(plan.rows[0]["feature_id_2"], "1");
        assert_eq!(plan.skipped[0]["feature_index"], 2);
    }
    #[test]
    fn batch_boundaries_preserve_every_row() {
        let rows = (0..501).map(|i| json!({"id":i})).collect::<Vec<_>>();
        assert_eq!(batches(&rows), vec![(0, 250), (250, 500), (500, 501)]);
    }
}
