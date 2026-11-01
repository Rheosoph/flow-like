use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{LazyLock, Mutex},
};

use flow_like::flow_like_storage::{
    arrow_schema::Schema, contracts::column_metadata::ColumnMetadata,
    databases::vector::schema::DatabaseSchemaField,
};
use reqwest::Method;
use serde_json::{Value, json};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use super::{NativePlatformSession, boolean, err, limit, required, segment, string};
use crate::{
    functions::app::tables,
    offline_writes::commands::{OfflineTableRoute, offline_writes_table_route},
};

static PENDING_SCHEMAS: LazyLock<Mutex<HashMap<String, VecDeque<Value>>>> =
    LazyLock::new(Default::default);
fn capability_key(session: &NativePlatformSession, app: &str) -> String {
    json!([
        session.profile.id,
        session.hub,
        session.auth.as_ref().map(|a| &a.generation),
        app
    ])
    .to_string()
}
pub(super) fn schema_unavailable(session: &NativePlatformSession, app: &str) -> bool {
    PENDING_SCHEMAS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&capability_key(session, app))
}
fn pending_schema_result(
    db: &Database<'_>,
    fields: &Value,
    if_not_exists: bool,
    skipped: bool,
) -> Value {
    let mut all = PENDING_SCHEMAS.lock().unwrap_or_else(|e| e.into_inner());
    let pending = all
        .entry(capability_key(db.session, &db.app_id))
        .or_default();
    pending
        .retain(|value| value["table_name"] != db.table || value["user_scoped"] != db.user_scoped);
    pending.push_back(json!({"table_name":db.table,"user_scoped":db.user_scoped,"if_not_exists":if_not_exists,"fields":fields}));
    while pending.len() > 128 {
        pending.pop_front();
    }
    json!({
    "status":"partial","code":"explicit_schema_create_not_deployed","app_id":db.app_id,"table_name":db.table,"created":false,"user_scoped":db.user_scoped,"requested_fields":fields,"pending_schema_count":pending.len(),"network_request_skipped":skipped,"message":"Explicit table-schema creation is unavailable on the connected API (the first POST returned 405). This schema is retained as pending in the native session. Submit each other required schema once; cached requests will not call the API or ask for approval. Continue the workflow/board build now and retry pending schemas after the matching API is deployed and the native session is restarted.","next_action":"continue_workflow_build"}
    )
}

pub(super) fn supports(args: &Value) -> bool {
    matches!(
        string(args, "operation", "operation").unwrap_or("list_tables"),
        "list_tables"
            | "create_table"
            | "delete_table"
            | "describe_table"
            | "query"
            | "insert"
            | "add_items"
            | "delete"
            | "remove_items"
            | "update"
            | "build_index"
            | "drop_index"
            | "optimize"
            | "add_column"
            | "drop_columns"
            | "alter_column"
            | "set_column_metadata"
            | "import_geojson"
    )
}

pub(super) struct Database<'a> {
    session: &'a NativePlatformSession,
    app_id: String,
    table: String,
    user_scoped: bool,
    local: bool,
    managed: bool,
    token: Option<String>,
}

impl Database<'_> {
    pub(super) async fn read(
        &self,
        operation: &str,
        query: Value,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        self.session.assert_current_profile().await?;
        let app = self.session.app_handle.clone();
        let id = self.app_id.clone();
        let table = self.table.clone();
        let user = Some(self.user_scoped);
        let sub = self.session.subject.clone();
        let token = self.token.clone();
        if self.local || self.managed && operation != "indices" {
            return match operation {
                "schema" => serde_json::to_value(
                    tables::db_schema(app, id, table, None, user, sub, None, token)
                        .await
                        .map_err(err)?,
                )
                .map_err(err),
                "indices" => serde_json::to_value(
                    tables::db_indices(app, id, table, None, user, sub, None, token)
                        .await
                        .map_err(err)?,
                )
                .map_err(err),
                "count" => Ok(json!(
                    tables::db_count(app, id, Some(table), None, user, sub, None, token)
                        .await
                        .map_err(err)?
                )),
                "list" => Ok(json!(
                    tables::db_list(
                        app,
                        id,
                        table,
                        None,
                        Some(limit as u64),
                        Some(offset as u64),
                        user,
                        sub,
                        None,
                        token
                    )
                    .await
                    .map_err(err)?
                )),
                "query" => Ok(json!(
                    tables::db_query(
                        app,
                        id,
                        table,
                        None,
                        Some(limit as u64),
                        Some(offset as u64),
                        serde_json::from_value(query).map_err(err)?,
                        user,
                        sub,
                        None,
                        token
                    )
                    .await
                    .map_err(err)?
                )),
                _ => Err("Unsupported database read.".into()),
            };
        }
        let suffix = match operation {
            "list" => "",
            other => other,
        };
        let route = format!(
            "{}{}?offset={offset}&limit={limit}&user_scoped={}",
            self.base(),
            if suffix.is_empty() {
                String::new()
            } else {
                format!("/{suffix}")
            },
            self.user_scoped
        );
        self.session
            .request(
                if operation == "query" {
                    Method::POST
                } else {
                    Method::GET
                },
                &route,
                (operation == "query").then_some(&query),
            )
            .await
    }

    fn base(&self) -> String {
        format!("apps/{}/db/{}", segment(&self.app_id), segment(&self.table))
    }

    pub(super) async fn write(&self, operation: &str, payload: Value) -> Result<Value, String> {
        self.session.assert_current_profile().await?;
        let structural = !matches!(operation, "insert" | "delete" | "update");
        if self.managed && structural {
            return Err(
                "Database structure changes require the hub while this table is kept offline."
                    .into(),
            );
        }
        if !self.local && !self.managed {
            let (method, suffix) = match operation {
                "create_table" => (Method::POST, String::new()),
                "delete_table" => (Method::DELETE, "/table".into()),
                "insert" => (Method::PUT, String::new()),
                "delete" => (Method::DELETE, String::new()),
                "update" => (Method::PUT, "/update".into()),
                "build_index" => (Method::POST, "/index".into()),
                "drop_index" => (
                    Method::DELETE,
                    format!(
                        "/index/{}",
                        segment(payload["index_name"].as_str().unwrap_or(""))
                    ),
                ),
                "optimize" => (Method::POST, "/optimize".into()),
                "add_column" => (Method::POST, "/columns".into()),
                "drop_columns" => (Method::DELETE, "/columns".into()),
                "alter_column" => (Method::PUT, "/columns".into()),
                "set_column_metadata" => (
                    Method::PUT,
                    format!(
                        "/columns/{}/metadata",
                        segment(payload["column"].as_str().unwrap_or(""))
                    ),
                ),
                _ => return Err("Unsupported database write.".into()),
            };
            let body = if operation == "set_column_metadata" {
                &payload["metadata"]
            } else {
                &payload
            };
            return self
                .session
                .request(
                    method,
                    &format!("{}{suffix}?user_scoped={}", self.base(), self.user_scoped),
                    Some(body),
                )
                .await;
        }
        let app = self.session.app_handle.clone();
        let id = self.app_id.clone();
        let table = self.table.clone();
        let user = Some(self.user_scoped);
        let sub = self.session.subject.clone();
        let token = self.token.clone();
        match operation {
            "create_table" => {
                return serde_json::to_value(
                    tables::db_create_table(
                        app,
                        id,
                        table,
                        serde_json::from_value(payload["fields"].clone()).map_err(err)?,
                        payload["if_not_exists"].as_bool(),
                        None,
                        user,
                        sub,
                        token,
                    )
                    .await
                    .map_err(err)?,
                )
                .map_err(err);
            }
            "delete_table" => {
                return serde_json::to_value(
                    tables::db_drop_table(app, id, table, None, user, sub, token)
                        .await
                        .map_err(err)?,
                )
                .map_err(err);
            }
            "insert" => tables::db_add(
                app,
                id,
                table,
                None,
                serde_json::from_value(payload["items"].clone()).map_err(err)?,
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "delete" => tables::db_delete(
                app,
                id,
                table,
                None,
                required(&payload, "query", "query")?,
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "update" => tables::db_update(
                app,
                id,
                table,
                None,
                required(&payload, "filter", "filter")?,
                serde_json::from_value(payload["updates"].clone()).map_err(err)?,
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "build_index" => tables::build_index(
                app,
                id,
                table,
                None,
                required(&payload, "column", "column")?,
                required(&payload, "index_type", "index_type")?,
                payload["optimize"].as_bool(),
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "drop_index" => tables::db_drop_index(
                app,
                id,
                table,
                None,
                required(&payload, "index_name", "index_name")?,
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "optimize" => tables::db_optimize(
                app,
                id,
                table,
                None,
                payload["keep_versions"].as_bool(),
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "add_column" => tables::db_add_column(
                app,
                id,
                table,
                None,
                serde_json::from_value(payload).map_err(err)?,
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "drop_columns" => tables::db_drop_columns(
                app,
                id,
                table,
                None,
                serde_json::from_value(payload["columns"].clone()).map_err(err)?,
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "alter_column" => tables::db_alter_column(
                app,
                id,
                table,
                None,
                required(&payload, "column", "column")?,
                boolean(&payload, "nullable", "nullable", true),
                user,
                sub,
                None,
                token,
            )
            .await
            .map_err(err)?,
            "set_column_metadata" => {
                return serde_json::to_value(
                    tables::db_set_column_metadata(
                        app,
                        id,
                        table,
                        None,
                        required(&payload, "column", "column")?,
                        serde_json::from_value(payload["metadata"].clone()).map_err(err)?,
                        user,
                        sub,
                        None,
                        token,
                    )
                    .await
                    .map_err(err)?,
                )
                .map_err(err);
            }
            _ => return Err("Unsupported database write.".into()),
        }
        Ok(Value::Null)
    }
}

pub(super) async fn create_schema(
    db: &Database<'_>,
    fields: &Value,
    if_not_exists: bool,
) -> Result<Value, String> {
    if !db.local && schema_unavailable(db.session, &db.app_id) {
        return Ok(pending_schema_result(db, fields, if_not_exists, true));
    }
    match db
        .write(
            "create_table",
            json!({"fields":fields,"if_not_exists":if_not_exists}),
        )
        .await
    {
        Err(error) if !db.local && error.starts_with("The hub returned HTTP 405 ") => {
            Ok(pending_schema_result(db, fields, if_not_exists, false))
        }
        result => result,
    }
}

pub(super) async fn metadata(
    session: &NativePlatformSession,
    app_id: &str,
    user_scoped: bool,
    table: Option<&str>,
) -> Result<Value, String> {
    session.assert_current_profile().await?;
    let local = session.is_local(app_id).await?;
    let Some(table) = table else {
        return if local {
            if user_scoped {
                Ok(json!(
                    tables::db_table_names_user(
                        session.app_handle.clone(),
                        app_id.into(),
                        None,
                        session.subject.clone(),
                        None
                    )
                    .await
                    .map_err(err)?
                ))
            } else {
                Ok(json!(
                    tables::db_table_names(session.app_handle.clone(), app_id.into(), None, None)
                        .await
                        .map_err(err)?
                ))
            }
        } else {
            session
                .request(
                    Method::GET,
                    &format!(
                        "apps/{}/db{}",
                        segment(app_id),
                        if user_scoped { "/user" } else { "" }
                    ),
                    None,
                )
                .await
        };
    };
    let token = if local {
        None
    } else {
        Some(session.token().await?)
    };
    let managed = if let Some(token) = &token {
        offline_writes_table_route(
            session.app_handle.clone(),
            app_id.into(),
            token.clone(),
            table.into(),
            user_scoped,
        )
        .await
        .map_err(err)?
            == OfflineTableRoute::Device
    } else {
        false
    };
    Database {
        session,
        app_id: app_id.into(),
        table: table.into(),
        user_scoped,
        local,
        managed,
        token,
    }
    .read("schema", Value::Null, 0, 0)
    .await
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let operation = string(args, "operation", "operation").unwrap_or("list_tables");
    let user_scoped = boolean(args, "user_scoped", "userScoped", false);
    let local = session.is_local(&app_id).await?;
    if operation == "list_tables" {
        let (project, user) = if local {
            let project =
                tables::db_table_names(session.app_handle.clone(), app_id.clone(), None, None)
                    .await
                    .map_err(err)?;
            let user = tables::db_table_names_user(
                session.app_handle.clone(),
                app_id.clone(),
                None,
                session.subject.clone(),
                None,
            )
            .await
            .map_err(err)?;
            (json!(project), json!(user))
        } else {
            let project = session
                .request(Method::GET, &format!("apps/{}/db", segment(&app_id)), None)
                .await?;
            let user = session
                .request(
                    Method::GET,
                    &format!("apps/{}/db/user", segment(&app_id)),
                    None,
                )
                .await?;
            if !project.is_array() || !user.is_array() {
                return Err("The hub returned an invalid table inventory.".into());
            }
            (project, user)
        };
        let mut result =
            json!({"status":"ok","app_id":app_id,"project_tables":project,"user_tables":user});
        if let Some(pending) = PENDING_SCHEMAS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&capability_key(session, &app_id))
        {
            result["explicit_schema_create_supported"] = json!(false);
            result["pending_schema_requests"] = json!(pending);
        }
        return Ok(result);
    }
    let requested_table = required(args, "table_name", "tableName")?;
    let table = if matches!(
        operation,
        "create_table" | "delete_table" | "import_geojson"
    ) {
        normalize_table(&requested_table)?
    } else {
        requested_table.clone()
    };
    let token = if local {
        None
    } else {
        Some(session.token().await?)
    };
    let managed = if let Some(token) = &token {
        offline_writes_table_route(
            session.app_handle.clone(),
            app_id.clone(),
            token.clone(),
            table.clone(),
            user_scoped,
        )
        .await
        .map_err(err)?
            == OfflineTableRoute::Device
    } else {
        false
    };
    let db = Database {
        session,
        app_id: app_id.clone(),
        table: table.clone(),
        user_scoped,
        local,
        managed,
        token,
    };
    let limit = limit(
        args,
        "limit",
        "limit",
        if operation == "describe_table" {
            10
        } else {
            50
        },
        200,
    );
    let offset = args["offset"].as_u64().unwrap_or(0).min(100_000) as usize;
    match operation {
        "import_geojson" => {
            super::geojson_import::execute(
                session,
                args,
                &db,
                &table,
                &requested_table,
                user_scoped,
            )
            .await
        }
        "describe_table" => {
            let schema = db.read("schema", Value::Null, 0, limit).await?;
            let indices = db.read("indices", Value::Null, 0, limit).await?;
            let count = db.read("count", Value::Null, 0, limit).await?;
            let columns = summarize_columns(&schema)?;
            let mut result = json!({"status":"ok","table_name":table,"user_scoped":user_scoped,"columns":columns,"schema":schema,"indices":indices,"row_count":count});
            if boolean(args, "include_sample", "includeSample", true) {
                result["sample"] = db.read("list", Value::Null, 0, limit).await?;
            }
            Ok(result)
        }
        "query" => {
            let query = args
                .get("query")
                .filter(|q| q.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let rows = db.read("query", query.clone(), offset, limit).await?;
            let rows = rows
                .as_array()
                .ok_or("Database query returned an invalid row set.")?;
            let mut result = query_page(rows, &query, offset, limit);
            result["status"] = json!("ok");
            result["table_name"] = json!(table);
            result["user_scoped"] = json!(user_scoped);
            Ok(result)
        }
        "create_table" => {
            let fields: Vec<DatabaseSchemaField> =
                serde_json::from_value(args["fields"].clone()).map_err(err)?;
            if fields.is_empty() {
                return Err("create_table requires a non-empty fields array.".into());
            }
            flow_like::flow_like_storage::databases::vector::schema::database_fields_to_arrow_schema(&fields).map_err(err)?;
            let mut result = create_schema(
                &db,
                &args["fields"],
                boolean(args, "if_not_exists", "ifNotExists", true),
            )
            .await?;
            if !result.is_object() {
                return Err("The database returned an invalid create result.".into());
            }
            if result.get("status").is_none() {
                result["status"] = json!("ok");
            }
            result["user_scoped"] = json!(user_scoped);
            if table != requested_table {
                result["table_name"] = json!(table);
                result["requested_table_name"] = json!(requested_table);
                result["name_normalized"] = json!(true);
                result["name_normalization"] = json!(
                    "Human-facing table labels are stored as stable physical identifiers; use table_name in all workflow references."
                );
            }
            Ok(result)
        }
        "delete_table" => {
            let confirmation = required(args, "confirm_table_name", "confirmTableName")?;
            if normalize_table(&confirmation)? != table {
                return Err("delete_table confirmation mismatch: confirm_table_name must match table_name. Nothing was deleted.".into());
            }
            let result = db.write(operation, json!({})).await?;
            let ontologies = result
                .get("ontologies")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let saved_queries = result
                .get("saved_queries")
                .cloned()
                .unwrap_or_else(|| json!([]));
            Ok(json!({
            "status":"ok","app_id":app_id,"table_name":table,"user_scoped":user_scoped,"dropped":result["dropped"],"irreversible":true,"ontologies_pruned":ontologies,"saved_queries_referencing":saved_queries,"warnings":result.get("warnings").cloned().unwrap_or_else(||json!([])),"message":"Report whether the table was deleted, pruned ontology references, and affected saved queries to the user."}
            ))
        }
        "insert" | "add_items" => {
            let items = args["items"]
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or("insert requires non-empty items.")?;
            let schema = db.read("schema", Value::Null, 0, limit).await;
            match schema {
                Ok(schema) => assert_row_keys(items, &schema)?,
                Err(error) => {
                    let tables = if local {
                        json!(if user_scoped {
                            tables::db_table_names_user(
                                session.app_handle.clone(),
                                app_id.clone(),
                                None,
                                session.subject.clone(),
                                None,
                            )
                            .await
                            .map_err(err)?
                        } else {
                            tables::db_table_names(
                                session.app_handle.clone(),
                                app_id.clone(),
                                None,
                                None,
                            )
                            .await
                            .map_err(err)?
                        })
                    } else {
                        session
                            .request(
                                Method::GET,
                                &format!(
                                    "apps/{}/db{}",
                                    segment(&app_id),
                                    if user_scoped { "/user" } else { "" }
                                ),
                                None,
                            )
                            .await?
                    };
                    if tables
                        .as_array()
                        .is_none_or(|tables| tables.iter().any(|name| name == &table))
                    {
                        return Err(error);
                    }
                }
            }
            db.write("insert", json!({"items":items})).await?;
            Ok(json!({"status":"ok","inserted":items.len(),"table_name":table}))
        }
        "delete" | "remove_items" | "update" => {
            let filter = required(args, "filter", "filter")?;
            let op = if operation == "update" {
                "update"
            } else {
                "delete"
            };
            let payload = if op == "update" {
                if !args["updates"].is_object() {
                    return Err("update requires filter and updates.".into());
                }
                json!({"filter":filter,"updates":args["updates"]})
            } else {
                json!({"query":filter})
            };
            db.write(op, payload).await?;
            Ok(json!({"status":"ok","table_name":table,"filter":filter}))
        }
        "build_index" | "alter_column" | "set_column_metadata" => {
            let column = required(args, "column", "column")?;
            let payload = match operation {
                "build_index" => {
                    json!({"column":column,"index_type":index_type(args)?,"optimize":boolean(args,"optimize","optimize",false)})
                }
                "alter_column" => {
                    json!({"column":column,"nullable":boolean(args,"nullable","nullable",true)})
                }
                _ => {
                    let metadata = args
                        .get("column_metadata")
                        .or_else(|| args.get("columnMetadata"))
                        .ok_or("set_column_metadata requires column_metadata.")?;
                    validate_metadata(metadata)?;
                    json!({"column":column,"metadata":metadata})
                }
            };
            let metadata = db.write(operation, payload).await?;
            let mut result = json!({"status":"ok","table_name":table,"column":column});
            if operation == "set_column_metadata" {
                result["column_metadata"] = metadata;
                result["user_scoped"] = json!(user_scoped);
            }
            Ok(result)
        }
        "drop_index" => {
            let name = required(args, "index_name", "indexName")?;
            db.write(operation, json!({"index_name":name})).await?;
            Ok(json!({"status":"ok","table_name":table,"index_name":name}))
        }
        "optimize" => {
            db.write(
                operation,
                json!({"keep_versions":boolean(args,"keep_versions","keepVersions",true)}),
            )
            .await?;
            Ok(json!({"status":"ok","table_name":table}))
        }
        "add_column" => {
            let column = &args["column_definition"];
            let name = required(column, "name", "name")?;
            if string(column, "sql_expression", "sql_expression").is_some()
                == string(column, "type", "type").is_some()
            {
                return Err("add_column requires column_definition.name and exactly one of sql_expression or type.".into());
            }
            db.write(operation, column.clone()).await?;
            Ok(json!({"status":"ok","table_name":table,"column":name}))
        }
        "drop_columns" => {
            let columns: Vec<_> = args["columns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if columns.is_empty() {
                return Err("drop_columns requires columns.".into());
            }
            db.write(operation, json!({"columns":columns})).await?;
            Ok(json!({"status":"ok","table_name":table,"columns":columns}))
        }
        _ => Err(format!(
            "Unsupported database_tool operation '{operation}'."
        )),
    }
}

fn normalize_table(value: &str) -> Result<String, String> {
    if value.encode_utf16().count() <= 256
        && !value.is_empty()
        && !value.contains("..")
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Ok(value.into());
    }
    let mut output = String::new();
    for character in value
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
    {
        if character == '&' {
            if !output.is_empty() && !output.ends_with('_') {
                output.push('_');
            }
            output.push_str("and_");
        } else if character.is_alphanumeric() {
            output.push(character);
        } else if !output.is_empty() && !output.ends_with('_') {
            output.push('_');
        }
    }
    let output: String = output.trim_matches('_').chars().take(256).collect();
    if output.is_empty() {
        Err("create_table table_name must contain at least one letter or number.".into())
    } else {
        Ok(output)
    }
}

fn query_page(rows: &[Value], query: &Value, offset: usize, limit: usize) -> Value {
    if string(query, "sql", "sql").is_some() {
        let page: Vec<_> = rows.iter().skip(offset).take(limit).cloned().collect();
        json!({"row_count":page.len(),"total_rows":rows.len(),"truncated":offset.saturating_add(page.len())<rows.len(),"rows":page})
    } else {
        json!({"row_count":rows.len(),"rows":rows})
    }
}

pub(super) fn summarize_columns(value: &Value) -> Result<Value, String> {
    let schema: Schema = serde_json::from_value(value.clone()).map_err(err)?;
    let mut columns = Vec::new();
    for field in schema.fields() {
        use flow_like::flow_like_storage::arrow_schema::{DataType, TimeUnit};
        let data_type = if flow_like::flow_like_storage::geometry::is_geometry_field(field) {
            "geometry".into()
        } else {
            match field.data_type() {
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "string".into(),
                DataType::Boolean => "boolean".into(),
                DataType::Binary | DataType::LargeBinary | DataType::BinaryView => "binary".into(),
                DataType::Timestamp(TimeUnit::Millisecond, Some(zone))
                    if zone.as_ref() == "UTC" =>
                {
                    "timestamp:ms:UTC".into()
                }
                DataType::FixedSizeList(child, _) if child.data_type() == &DataType::Float32 => {
                    "vector".into()
                }
                other => {
                    let name = format!("{other:?}");
                    if name.starts_with("Int")
                        || name.starts_with("UInt")
                        || name.starts_with("Float")
                        || name == "Date32"
                    {
                        name.to_lowercase()
                    } else {
                        name
                    }
                }
            }
        };
        let mut column =
            json!({"name":field.name(),"type":data_type,"nullable":field.is_nullable()});
        if let DataType::FixedSizeList(child, size) = field.data_type()
            && child.data_type() == &DataType::Float32
        {
            column["vector_size"] = json!(size);
        }
        if field
            .metadata()
            .contains_key("lance-schema:unenforced-primary-key:position")
            || field
                .metadata()
                .get("lance-schema:unenforced-primary-key")
                .is_some_and(|value| !value.eq_ignore_ascii_case("false"))
        {
            column["primary_key"] = json!(true);
        }
        let metadata = ColumnMetadata::from_field_metadata(field.metadata());
        if metadata != ColumnMetadata::default() {
            column["column_metadata"] = serde_json::to_value(metadata).map_err(err)?;
        }
        columns.push(column);
    }
    Ok(json!(columns))
}

fn assert_row_keys(rows: &[Value], schema: &Value) -> Result<(), String> {
    let columns: BTreeSet<_> = schema["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|field| field["name"].as_str())
        .collect();
    let unknown: BTreeSet<_> = rows
        .iter()
        .filter_map(Value::as_object)
        .flat_map(|row| row.keys().map(String::as_str))
        .filter(|key| !columns.contains(key))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "The table has no columns {:?}; add them with add_column or rename the keys before inserting.",
            unknown.into_iter().take(20).collect::<Vec<_>>()
        ))
    }
}

fn index_type(args: &Value) -> Result<&str, String> {
    const NAMES: [&str; 18] = [
        "FullText",
        "BTree",
        "Bitmap",
        "LabelList",
        "Auto",
        "Vector",
        "Fm",
        "IvfFlat",
        "IvfPq",
        "IvfSq",
        "IvfRq",
        "IvfHnswFlat",
        "IvfHnswPq",
        "IvfHnswSq",
        "NGram",
        "ZoneMap",
        "BloomFilter",
        "RTree",
    ];
    let value = args.get("index_type").or_else(|| args.get("indexType"));
    if let Some(index) = value.and_then(Value::as_u64) {
        return Ok(NAMES.get(index as usize).copied().unwrap_or("Auto"));
    }
    let normalized = value
        .and_then(Value::as_str)
        .unwrap_or("Auto")
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized == "fts" || normalized == "inverted" {
        return Ok("FullText");
    }
    Ok(NAMES
        .into_iter()
        .find(|name| name.to_lowercase() == normalized)
        .unwrap_or("Auto"))
}

fn validate_metadata(metadata: &Value) -> Result<(), String> {
    let object = metadata
        .as_object()
        .ok_or("column_metadata must be an object.")?;
    if object.keys().any(|key| {
        ![
            "display_name",
            "description",
            "classification",
            "pii",
            "read_attributes",
        ]
        .contains(&key.as_str())
    }) {
        return Err("column_metadata contains unknown fields.".into());
    }
    let metadata: ColumnMetadata = serde_json::from_value(metadata.clone()).map_err(err)?;
    metadata.normalized("column").map(|_| ()).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_names_match_web_normalization() {
        for (input, expected) in [
            ("report.v1", "report.v1"),
            ("Revenue & Costs", "revenue_and_costs"),
            ("Café Reports", "cafe_reports"),
            ("  Deals / Q1  ", "deals_q1"),
        ] {
            assert_eq!(normalize_table(input).unwrap(), expected);
        }
        assert!(normalize_table("///").is_err());
    }
    #[test]
    fn unknown_row_keys_cannot_be_silently_dropped() {
        let schema = json!({"fields":[{"name":"id"}]});
        assert!(assert_row_keys(&[json!({"id":1})], &schema).is_ok());
        assert!(assert_row_keys(&[json!({"id":1,"secret":"value"})], &schema).is_err());
    }
    #[test]
    fn sql_results_are_paged_once_after_query_execution() {
        let rows = vec![json!(0), json!(1), json!(2)];
        assert_eq!(
            query_page(&rows, &json!({"sql":"SELECT * FROM x"}), 1, 1),
            json!({"row_count":1,"total_rows":3,"truncated":true,"rows":[1]})
        );
        assert_eq!(
            query_page(&rows, &json!({"filter":"id > 0"}), 1, 1),
            json!({"row_count":3,"rows":rows})
        );
    }
}
