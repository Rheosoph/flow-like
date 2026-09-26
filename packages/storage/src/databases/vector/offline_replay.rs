//! Conditional logical mutations for a durable offline outbox.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, anyhow, ensure};
use flow_like_types::Value;
use futures::{StreamExt, TryStreamExt};
use lance::{
    Dataset,
    dataset::{
        builder::DatasetBuilder,
        write::{
            InsertBuilder, WriteMode, WriteParams,
            delete::DeleteBuilder,
            merge_insert::{MergeInsertBuilder, WhenMatched, WhenNotMatched},
            update::UpdateBuilder,
        },
    },
};
use lance_io::object_store::{ObjectStore, ObjectStoreParams, uri_to_url};
use lance_table::{
    format::{IndexMetadata, Manifest, Transaction},
    io::commit::{
        CommitError, CommitHandler, ManifestLocation, ManifestNamingScheme, ManifestWriter,
        commit_handler_from_url,
    },
};
use lancedb::{Connection, Table, table::WriteOptions};
use object_store::{ObjectStoreExt, path::Path};
use sqlparser::{
    ast::{BinaryOperator, Expr, Ident, UnaryOperator, Value as SqlValue},
    dialect::GenericDialect,
    parser::Parser,
    tokenizer::Token,
};

use super::lancedb::connect_lance;

const OPERATION_KEY: &str = "flow_like.offline.operation_id";
const DIGEST_KEY: &str = "flow_like.offline.digest";
const VERSION_KEY: &str = "flow_like.offline.version";
const BASE_KEY: &str = "flow_like.offline.base";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayMarker {
    pub operation_id: String,
    pub digest: String,
    pub expected_version: u64,
    pub expected_fingerprint: Option<String>,
}

impl ReplayMarker {
    fn target_version(&self) -> Result<u64> {
        ensure!(
            !self.operation_id.is_empty()
                && self.operation_id.len() <= 128
                && self.operation_id.is_ascii(),
            "invalid offline operation id"
        );
        ensure!(
            !self.digest.is_empty() && self.digest.len() <= 128 && self.digest.is_ascii(),
            "invalid offline operation digest"
        );
        ensure!(
            match (self.expected_version, &self.expected_fingerprint) {
                (0, None) => true,
                (1.., Some(value)) => value
                    .strip_prefix("blake3:")
                    .is_some_and(|hash| hash.len() == 64
                        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())),
                _ => false,
            },
            "existing offline tables require a manifest fingerprint; absent tables require none"
        );
        self.expected_version
            .checked_add(1)
            .ok_or_else(|| anyhow!("offline table version overflow"))
    }

    fn properties(&self) -> Result<HashMap<String, String>> {
        Ok(HashMap::from([
            (OPERATION_KEY.into(), self.operation_id.clone()),
            (DIGEST_KEY.into(), self.digest.clone()),
            (VERSION_KEY.into(), self.target_version()?.to_string()),
            (
                BASE_KEY.into(),
                self.expected_fingerprint.clone().unwrap_or_default(),
            ),
        ]))
    }

    fn matches(&self, properties: &HashMap<String, String>) -> bool {
        properties.get(OPERATION_KEY) == Some(&self.operation_id)
            && properties.get(DIGEST_KEY) == Some(&self.digest)
            && properties
                .get(VERSION_KEY)
                .and_then(|value| value.parse::<u64>().ok())
                == self.expected_version.checked_add(1)
            && properties.get(BASE_KEY).map(String::as_str)
                == Some(self.expected_fingerprint.as_deref().unwrap_or_default())
    }
}

#[derive(Clone, Debug)]
pub enum ReplayMutation {
    Insert {
        items: Vec<Value>,
    },
    Upsert {
        items: Vec<Value>,
        id_field: String,
    },
    Update {
        filter: String,
        updates: Vec<(String, String)>,
    },
    Delete {
        filter: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayOutcome {
    Applied { version: u64, fingerprint: String },
    NotApplied,
    Conflict { actual_version: u64 },
    Unknown { reason: String },
}

async fn fingerprint(dataset: &Dataset) -> Result<String> {
    let store = dataset.object_store(None).await?;
    let object = store.inner.get(&dataset.manifest_location().path).await?;
    if let (Some(expected), Some(actual)) = (&dataset.manifest_location().e_tag, &object.meta.e_tag)
    {
        ensure!(
            expected == actual,
            "table manifest changed while reading its fingerprint"
        );
    }
    let mut hasher = blake3::Hasher::new();
    let mut stream = object.into_stream();
    while let Some(chunk) = stream.try_next().await? {
        hasher.update(&chunk);
    }
    // A lost local response can leave a published manifest whose explicit sync
    // did not return. Reconciliation must flush that receipt before admitting it.
    if store.scheme() == "file-object-store" {
        let uri = uri_to_url(dataset.uri())?;
        let local = flow_like_types::reqwest::Url::parse(&uri.as_str().replacen(
            "file-object-store:",
            "file:",
            1,
        ))?;
        let root = local
            .to_file_path()
            .map_err(|_| anyhow!("invalid local manifest URI"))?;
        let prefix = Path::from_absolute_path(&root)?;
        let suffix = dataset
            .manifest_location()
            .path
            .prefix_match(&prefix)
            .ok_or_else(|| anyhow!("local manifest is outside its table"))?;
        let file = suffix.fold(root.clone(), |mut path, part| {
            path.push(part.as_ref());
            path
        });
        sync_local_object(root, file, true).await?;
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

async fn stored_fingerprint(store: &ObjectStore, path: &Path) -> Result<String> {
    let mut stream = store.inner.get(path).await?.into_stream();
    let mut hasher = blake3::Hasher::new();
    while let Some(chunk) = stream.try_next().await? {
        hasher.update(&chunk);
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

pub async fn revision(table: &Table) -> Result<(u64, String)> {
    let mut dataset = table
        .dataset()
        .ok_or_else(|| anyhow!("offline replay requires a native Lance table"))?
        .get()
        .await?
        .as_ref()
        .clone();
    dataset.checkout_latest().await?;
    Ok((dataset.manifest().version, fingerprint(&dataset).await?))
}

async fn applied(dataset: &Dataset) -> Result<ReplayOutcome> {
    Ok(ReplayOutcome::Applied {
        version: dataset.manifest().version,
        fingerprint: fingerprint(dataset).await?,
    })
}

/// The caller must durably record the marker before dispatching a mutation.
/// An unknown outcome must be reconciled without changing its expected version.
pub async fn reconcile(table: &Table, marker: &ReplayMarker) -> Result<ReplayOutcome> {
    let target = marker.target_version()?;
    let wrapper = table
        .dataset()
        .ok_or_else(|| anyhow!("offline replay requires a native Lance table"))?;
    let mut latest = wrapper.get().await?.as_ref().clone();
    latest.checkout_latest().await?;
    let version = latest.manifest().version;
    if version == marker.expected_version {
        if marker.expected_fingerprint.as_deref() != Some(fingerprint(&latest).await?.as_str()) {
            return Ok(ReplayOutcome::Conflict {
                actual_version: version,
            });
        }
        return Ok(ReplayOutcome::NotApplied);
    }
    if version < target {
        return Ok(ReplayOutcome::Conflict {
            actual_version: version,
        });
    }
    let committed = match latest.checkout_version(target).await {
        Ok(dataset) => dataset,
        Err(error) => {
            return Ok(ReplayOutcome::Unknown {
                reason: format!("cannot inspect offline operation manifest {target}: {error}"),
            });
        }
    };
    if marker.matches(committed.metadata()) {
        return applied(&committed).await;
    }
    // Every guarded commit stamps the manifest itself. Lance keys its separate
    // transaction cache by version, which can be reused after table recreation.
    // That cache must never serve as alternate proof of an offline mutation.
    Ok(ReplayOutcome::Conflict {
        actual_version: version,
    })
}

#[derive(Debug)]
struct GuardedCommit {
    marker: ReplayMarker,
    delegate: Arc<dyn CommitHandler>,
}

#[async_trait::async_trait]
impl CommitHandler for GuardedCommit {
    async fn commit(
        &self,
        manifest: &mut Manifest,
        indices: Option<Vec<IndexMetadata>>,
        base_path: &Path,
        object_store: &ObjectStore,
        manifest_writer: ManifestWriter,
        naming_scheme: ManifestNamingScheme,
        mut transaction: Option<Transaction>,
    ) -> std::result::Result<ManifestLocation, CommitError> {
        let target = self.marker.expected_version + 1;
        if manifest.version != target {
            return Err(CommitError::OtherError(lance::Error::invalid_input(
                "offline mutation conflicts with a newer table version",
            )));
        }
        if let Some(expected) = &self.marker.expected_fingerprint {
            // Normal row writers compete for the same create-only next manifest.
            // This fingerprint read and that create are separate object calls:
            // external DROP/recreate must be excluded while buffering is active.
            let base_manifest =
                naming_scheme.manifest_path(base_path, self.marker.expected_version);
            let actual = stored_fingerprint(object_store, &base_manifest)
                .await
                .map_err(|error| {
                    CommitError::OtherError(lance::Error::invalid_input(error.to_string()))
                })?;
            if &actual != expected {
                return Err(CommitError::OtherError(lance::Error::invalid_input(
                    "offline base manifest was replaced",
                )));
            }
        }
        let properties = self.marker.properties().map_err(|error| {
            CommitError::OtherError(lance::Error::invalid_input(error.to_string()))
        })?;
        // Lance 8 embeds the transaction in the manifest. The receipt therefore
        // commits atomically with rows, including after a lost HTTP response.
        if let Some(transaction) = &mut transaction {
            transaction.inner.uuid.clone_from(&self.marker.operation_id);
            transaction
                .inner
                .transaction_properties
                .extend(properties.clone());
        }
        // A fixed-size last receipt survives ordinary commits and history cleanup.
        // Older ambiguous operations still require their exact version manifest.
        manifest.table_metadata.extend(properties);
        self.delegate
            .commit(
                manifest,
                indices,
                base_path,
                object_store,
                manifest_writer,
                naming_scheme,
                transaction,
            )
            .await
            .map_err(|error| match error {
                CommitError::CommitConflict => {
                    CommitError::OtherError(lance::Error::invalid_input(
                        "offline mutation lost its conditional manifest commit",
                    ))
                }
                error => error,
            })
    }
}

async fn guarded_handler(uri: &str, marker: &ReplayMarker) -> Result<Arc<dyn CommitHandler>> {
    marker.target_version()?;
    let url = uri_to_url(uri)?;
    ensure!(
        matches!(
            url.scheme(),
            "file"
                | "file-object-store"
                | "s3"
                | "gs"
                | "az"
                | "abfss"
                | "memory"
                | "oss"
                | "cos"
                | "shared-memory"
        ),
        "offline replay does not support external/catalog commit handlers"
    );
    Ok(Arc::new(GuardedCommit {
        marker: marker.clone(),
        delegate: commit_handler_from_url(uri, &None).await?,
    }))
}

#[derive(Debug)]
struct BoundReplayStores {
    bindings: Vec<(String, Arc<ObjectStore>)>,
    original_registry: Arc<lance_io::object_store::ObjectStoreRegistry>,
}

impl BoundReplayStores {
    fn binding(&self, url: &flow_like_types::reqwest::Url) -> lance::Result<&Arc<ObjectStore>> {
        self.bindings
            .iter()
            .find_map(|(uri, store)| (uri == url.as_str()).then_some(store))
            .ok_or_else(|| {
                lance::Error::invalid_input(
                    "offline replay attempted to open an unbound object store",
                )
            })
    }
}

#[async_trait::async_trait]
impl lance_io::object_store::ObjectStoreProvider for BoundReplayStores {
    async fn new_store(
        &self,
        url: flow_like_types::reqwest::Url,
        _: &ObjectStoreParams,
    ) -> lance::Result<ObjectStore> {
        Ok(self.binding(&url)?.as_ref().clone())
    }
    fn extract_path(&self, url: &flow_like_types::reqwest::Url) -> lance::Result<Path> {
        self.binding(url)?;
        ObjectStore::extract_path_from_uri(self.original_registry.clone(), url.as_str())
    }
    fn calculate_object_store_prefix(
        &self,
        url: &flow_like_types::reqwest::Url,
        _: Option<&HashMap<String, String>>,
    ) -> lance::Result<String> {
        self.binding(url)?;
        Ok(format!("offline-replay:{url}"))
    }
}

async fn guarded_dataset(table: &Table, marker: &ReplayMarker) -> Result<Arc<Dataset>> {
    let dataset = table
        .dataset()
        .ok_or_else(|| anyhow!("offline replay requires a native Lance table"))?
        .get()
        .await?;
    let handler = guarded_handler(dataset.uri(), marker).await?;
    let mut bindings = vec![(
        uri_to_url(dataset.uri())?.to_string(),
        dataset.object_store(None).await?,
    )];
    for base in dataset.manifest().base_paths.values() {
        bindings.push((
            uri_to_url(&base.path)?.to_string(),
            dataset.object_store(Some(base.id)).await?,
        ));
    }
    let provider = Arc::new(BoundReplayStores {
        bindings,
        original_registry: dataset.session().store_registry(),
    });
    let registry = Arc::new(lance_io::object_store::ObjectStoreRegistry::empty());
    for (uri, _) in &provider.bindings {
        registry.insert(uri_to_url(uri)?.scheme(), provider.clone());
    }
    let session = Arc::new(lance::session::Session::new(
        16 * 1024 * 1024,
        16 * 1024 * 1024,
        registry,
    ));
    let builder = DatasetBuilder::from_uri(dataset.uri())
        .with_session(session)
        .with_commit_handler(handler)
        .with_version(marker.expected_version);
    Ok(Arc::new(builder.load().await?))
}

/// Accept only deterministic scalar syntax. Function calls, subqueries, session
/// values and casts are intentionally unsupported until their replay semantics
/// are explicitly established. Quoted column names remain available.
pub fn validate_expression(expression: &str) -> Result<()> {
    ensure!(
        expression.len() <= 64 * 1024,
        "offline expression exceeds 64 KiB"
    );
    let expression = parse_expression(expression)?;
    fn deterministic(expression: &Expr, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        let child = |expression: &Expr| deterministic(expression, depth + 1);
        match expression {
            Expr::Identifier(identifier) => {
                identifier.quote_style.is_some()
                    || !matches!(
                        identifier.value.to_ascii_lowercase().as_str(),
                        "current_date"
                            | "current_time"
                            | "current_timestamp"
                            | "localtime"
                            | "localtimestamp"
                            | "current_user"
                            | "session_user"
                            | "user"
                    )
            }
            Expr::CompoundIdentifier(_) | Expr::Value(_) => true,
            Expr::Nested(expression)
            | Expr::UnaryOp {
                expr: expression, ..
            }
            | Expr::IsNull(expression)
            | Expr::IsNotNull(expression)
            | Expr::IsTrue(expression)
            | Expr::IsNotTrue(expression)
            | Expr::IsFalse(expression)
            | Expr::IsNotFalse(expression) => child(expression),
            Expr::BinaryOp { left, right, .. }
            | Expr::IsDistinctFrom(left, right)
            | Expr::IsNotDistinctFrom(left, right) => child(left) && child(right),
            Expr::Between {
                expr, low, high, ..
            } => child(expr) && child(low) && child(high),
            Expr::InList { expr, list, .. } => child(expr) && list.iter().all(child),
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                child(expr) && child(pattern)
            }
            _ => false,
        }
    }
    ensure!(
        deterministic(&expression, 0),
        "offline mutations require deterministic scalar predicates and update expressions"
    );
    Ok(())
}

fn parse_expression(expression: &str) -> Result<Expr> {
    let mut parser = Parser::new(&GenericDialect).try_with_sql(expression)?;
    let expression = parser.parse_expr()?;
    ensure!(
        parser.peek_token().token == Token::EOF,
        "unexpected trailing offline expression tokens"
    );
    Ok(expression)
}

/// Two key-set deletes (`k = v`, `k IN (…)` or `false`) as one flat `IN` list.
/// An `OR` chain would nest one level per merged delete, and Lance plans filters
/// recursively. `None` unless both sides are key sets of the same column.
pub fn merge_key_deletes(first: &str, next: &str) -> Option<String> {
    let (column, mut keys) = key_set(first)?;
    let (other, more) = key_set(next)?;
    let column = match (column, other) {
        (Some(column), Some(other)) if column != other => return None,
        (column, other) => column.or(other),
    };
    let Some(column) = column else {
        return Some("false".into());
    };
    keys.extend(more);
    Some(
        Expr::InList {
            expr: Box::new(Expr::Identifier(column)),
            list: keys,
            negated: false,
        }
        .to_string(),
    )
}

fn key_set(filter: &str) -> Option<(Option<Ident>, Vec<Expr>)> {
    fn literal(expression: &Expr) -> bool {
        match expression {
            Expr::Value(literal) => matches!(
                literal.value,
                SqlValue::Number(..) | SqlValue::SingleQuotedString(_)
            ),
            Expr::UnaryOp {
                op: UnaryOperator::Minus,
                expr,
            } => {
                matches!(&**expr, Expr::Value(literal) if matches!(literal.value, SqlValue::Number(..)))
            }
            _ => false,
        }
    }
    match parse_expression(filter).ok()? {
        Expr::Value(literal) if literal.value == SqlValue::Boolean(false) => {
            Some((None, Vec::new()))
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => match *left {
            Expr::Identifier(column) if literal(&right) => Some((Some(column), vec![*right])),
            _ => None,
        },
        Expr::InList {
            expr,
            list,
            negated: false,
        } => match *expr {
            Expr::Identifier(column) if list.iter().all(literal) => Some((Some(column), list)),
            _ => None,
        },
        _ => None,
    }
}

pub async fn replay(
    table: &Table,
    marker: &ReplayMarker,
    mutation: ReplayMutation,
) -> Result<ReplayOutcome> {
    match reconcile(table, marker).await? {
        ReplayOutcome::NotApplied => {}
        outcome => return Ok(outcome),
    }
    ensure!(
        marker.expected_version > 0,
        "use create for an absent offline table"
    );
    let dataset = guarded_dataset(table, marker).await?;
    ensure!(
        marker.expected_fingerprint.as_deref() == Some(fingerprint(&dataset).await?.as_str()),
        "offline base manifest fingerprint changed"
    );
    let schema: arrow_schema::Schema = dataset.schema().into();
    let fields = schema.fields().iter().cloned().collect::<Vec<_>>();
    let result: Result<()> = async {
        match mutation {
            ReplayMutation::Insert { items } => {
                ensure!(!items.is_empty(), "offline insert cannot be empty");
                let reader =
                    crate::arrow_utils::value_to_batch_reader_with_fields(items, Some(fields))?;
                let params = WriteParams {
                    mode: WriteMode::Append,
                    skip_auto_cleanup: true,
                    ..Default::default()
                };
                InsertBuilder::new(dataset)
                    .with_params(&params)
                    .execute_stream(reader)
                    .await?;
            }
            ReplayMutation::Upsert { items, id_field } => {
                ensure!(!items.is_empty(), "offline upsert cannot be empty");
                ensure!(
                    items
                        .iter()
                        .all(|item| item.get(&id_field).is_some_and(|key| !key.is_null())),
                    "offline upsert requires a non-null key in every row"
                );
                let keyed = super::schema::primary_key_columns(&schema) == [id_field.as_str()];
                let reader =
                    crate::arrow_utils::value_to_batch_reader_with_fields(items, Some(fields))?;
                let mut builder = MergeInsertBuilder::try_new(dataset, vec![id_field])?;
                builder
                    .when_matched(WhenMatched::UpdateAll)
                    .when_not_matched(WhenNotMatched::InsertAll)
                    .conflict_retries(0)
                    .use_index(!keyed);
                builder.try_build()?.execute_reader(reader).await?;
            }
            ReplayMutation::Update { filter, updates } => {
                validate_expression(&filter)?;
                ensure!(!updates.is_empty(), "offline update cannot be empty");
                let mut builder = UpdateBuilder::new(dataset)
                    .update_where(&filter)?
                    .conflict_retries(0);
                for (column, expression) in updates {
                    ensure!(
                        !crate::geometry::is_geometry_field(schema.field_with_name(&column)?),
                        "geometry columns require validated upserts"
                    );
                    validate_expression(&expression)?;
                    builder = builder.set(column, &expression)?;
                }
                builder.build()?.execute().await?;
            }
            ReplayMutation::Delete { filter } => {
                validate_expression(&filter)?;
                DeleteBuilder::new(dataset, &filter)
                    .conflict_retries(0)
                    .execute()
                    .await?;
            }
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => reconcile(table, marker).await,
        Err(error) => match reconcile(table, marker).await {
            Ok(outcome @ ReplayOutcome::Applied { .. }) => Ok(outcome),
            Ok(ReplayOutcome::Conflict { actual_version }) => {
                Ok(ReplayOutcome::Conflict { actual_version })
            }
            _ => Err(error),
        },
    }
}

/// Create-only table replay. A competing creator can never be overwritten.
pub async fn create(
    connection: &Connection,
    name: &str,
    marker: &ReplayMarker,
    items: Vec<Value>,
) -> Result<ReplayOutcome> {
    ensure!(
        marker.expected_version == 0,
        "offline table creation requires version zero"
    );
    marker.target_version()?;
    super::lancedb::LanceDBVectorStore::validate_table_name(name)?;
    match connection.open_table(name).execute().await {
        Ok(table) => return reconcile(&table, marker).await,
        Err(lancedb::Error::TableNotFound { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    ensure!(
        !items.is_empty(),
        "offline table creation requires schema-bearing rows"
    );
    let reader = crate::arrow_utils::value_to_batch_reader_with_utc_timestamp_inference(items)?;
    let handler = guarded_handler(connection.uri(), marker).await?;
    let params = WriteParams {
        mode: WriteMode::Create,
        commit_handler: Some(handler),
        skip_auto_cleanup: true,
        ..Default::default()
    };
    let result = connection
        .create_table(name, reader)
        .write_options(WriteOptions {
            lance_write_params: Some(params),
        })
        .execute()
        .await;
    match result {
        Ok(table) => reconcile(&table, marker).await,
        Err(error) => match connection.open_table(name).execute().await {
            Ok(table) => match reconcile(&table, marker).await? {
                outcome @ ReplayOutcome::Applied { .. } => Ok(outcome),
                ReplayOutcome::Conflict { actual_version } => {
                    Ok(ReplayOutcome::Conflict { actual_version })
                }
                _ => Err(error.into()),
            },
            Err(_) => Err(error.into()),
        },
    }
}

pub struct MaterializedTable {
    pub table: Table,
    pub source_version: u64,
    pub source_fingerprint: String,
}

#[derive(Debug)]
struct WriteBudget {
    maximum: u64,
    used: std::sync::atomic::AtomicU64,
}

impl WriteBudget {
    fn charge(&self, bytes: u64) -> object_store::Result<()> {
        self.used
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |used| used.checked_add(bytes).filter(|next| *next <= self.maximum),
            )
            .map(|_| ())
            .map_err(|_| object_store::Error::Generic {
                store: "offline-materialization",
                source: "offline snapshot exceeds its disk write budget".into(),
            })
    }
}

#[derive(Debug)]
struct BudgetWrapper(Arc<WriteBudget>);

impl lance_io::object_store::WrappingObjectStore for BudgetWrapper {
    fn wrap(
        &self,
        _: &str,
        original: Arc<dyn object_store::ObjectStore>,
    ) -> Arc<dyn object_store::ObjectStore> {
        Arc::new(BudgetedStore {
            inner: original,
            budget: self.0.clone(),
        })
    }
}

#[derive(Debug)]
struct BudgetedStore {
    inner: Arc<dyn object_store::ObjectStore>,
    budget: Arc<WriteBudget>,
}

impl std::fmt::Display for BudgetedStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "bounded offline snapshot {}", self.inner)
    }
}

#[async_trait::async_trait]
impl object_store::ObjectStore for BudgetedStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: object_store::PutPayload,
        options: object_store::PutOptions,
    ) -> object_store::Result<object_store::PutResult> {
        self.budget.charge(payload.content_length() as u64 + 4096)?;
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        options: object_store::PutMultipartOptions,
    ) -> object_store::Result<Box<dyn object_store::MultipartUpload>> {
        self.budget.charge(4096)?;
        Ok(Box::new(BudgetedUpload {
            inner: self.inner.put_multipart_opts(location, options).await?,
            budget: self.budget.clone(),
        }))
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: object_store::GetOptions,
    ) -> object_store::Result<object_store::GetResult> {
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: futures::stream::BoxStream<'static, object_store::Result<Path>>,
    ) -> futures::stream::BoxStream<'static, object_store::Result<Path>> {
        self.inner.delete_stream(locations)
    }

    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> futures::stream::BoxStream<'static, object_store::Result<object_store::ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&Path>,
    ) -> object_store::Result<object_store::ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: object_store::CopyOptions,
    ) -> object_store::Result<()> {
        let size = self
            .inner
            .get_opts(
                from,
                object_store::GetOptions {
                    head: true,
                    ..Default::default()
                },
            )
            .await?
            .meta
            .size;
        self.budget.charge(size + 4096)?;
        self.inner.copy_opts(from, to, options).await
    }
}

#[derive(Debug)]
struct BudgetedUpload {
    inner: Box<dyn object_store::MultipartUpload>,
    budget: Arc<WriteBudget>,
}

#[async_trait::async_trait]
impl object_store::MultipartUpload for BudgetedUpload {
    fn put_part(&mut self, payload: object_store::PutPayload) -> object_store::UploadPart {
        if let Err(error) = self.budget.charge(payload.content_length() as u64) {
            return Box::pin(async move { Err(error) });
        }
        self.inner.put_part(payload)
    }
    async fn complete(&mut self) -> object_store::Result<object_store::PutResult> {
        self.inner.complete().await
    }
    async fn abort(&mut self) -> object_store::Result<()> {
        self.inner.abort().await
    }
}

struct ByteCounter {
    bytes: u64,
    maximum: u64,
}

impl std::io::Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(buffer.len() as u64)
            .filter(|bytes| *bytes <= self.maximum)
            .ok_or_else(|| {
                std::io::Error::other("offline snapshot exceeds serialized byte budget")
            })?;
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct LocalBudgetStore {
    inner: Arc<dyn object_store::ObjectStore>,
    root: std::path::PathBuf,
    prefix: Path,
    budget: Arc<WriteBudget>,
    locks: Vec<Arc<futures::lock::Mutex<()>>>,
}

fn sync_directory(directory: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(0x02000000)
            .open(directory)?
            .sync_all()
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(directory)?.sync_all()
    }
}

async fn sync_local_object(
    root: std::path::PathBuf,
    file: std::path::PathBuf,
    exists: bool,
) -> object_store::Result<()> {
    flow_like_types::tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if exists {
            let metadata = std::fs::symlink_metadata(&file)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(std::io::Error::other(
                    "offline durable object must be a regular file",
                ));
            }
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&file)?
                .sync_all()?;
        }
        let mut parent = file.parent();
        while let Some(directory) = parent {
            if !directory.starts_with(&root) {
                break;
            }
            sync_directory(directory)?;
            if directory == root {
                break;
            }
            parent = directory.parent();
        }
        Ok(())
    })
    .await
    .map_err(|error| object_store::Error::Generic {
        store: "offline-local-durability",
        source: Box::new(error),
    })?
    .map_err(|error| object_store::Error::Generic {
        store: "offline-local-durability",
        source: Box::new(error),
    })
}

/// Flush existing overlay files once at startup, while holding the scope's
/// exclusive writer lock. Later writes sync only their changed objects.
pub async fn flush_durable_local(root: &std::path::Path) -> Result<()> {
    let root = root.to_owned();
    flow_like_types::tokio::task::spawn_blocking(move || -> Result<()> {
        ensure!(
            !std::fs::symlink_metadata(&root)?.file_type().is_symlink(),
            "offline database root cannot be a symlink"
        );
        let root = root.canonicalize()?;
        let mut stack = vec![(root.clone(), std::fs::read_dir(&root)?)];
        while let Some((_, entries)) = stack.last_mut() {
            match entries.next() {
                Some(entry) => {
                    let entry = entry?;
                    let metadata = std::fs::symlink_metadata(entry.path())?;
                    ensure!(
                        !metadata.file_type().is_symlink(),
                        "symlinks are forbidden in the offline database"
                    );
                    if metadata.is_dir() {
                        ensure!(
                            stack.len() < 64,
                            "offline database directory nesting exceeds 64 levels"
                        );
                        stack.push((entry.path(), std::fs::read_dir(entry.path())?));
                    } else {
                        ensure!(
                            metadata.is_file(),
                            "offline database contains a non-regular file"
                        );
                        std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(entry.path())?
                            .sync_all()?;
                    }
                }
                None => {
                    let (directory, _) = stack.pop().expect("directory exists");
                    sync_directory(&directory)?;
                }
            }
        }
        if let Some(parent) = root.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    })
    .await??;
    Ok(())
}

impl std::fmt::Display for LocalBudgetStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("private bounded offline database")
    }
}

impl LocalBudgetStore {
    fn local_path(&self, path: &Path) -> object_store::Result<std::path::PathBuf> {
        self.check(path)?;
        Ok(path
            .prefix_match(&self.prefix)
            .expect("checked scope")
            .fold(self.root.clone(), |mut file, part| {
                file.push(part.as_ref());
                file
            }))
    }
    fn check(&self, path: &Path) -> object_store::Result<()> {
        let suffix =
            path.prefix_match(&self.prefix)
                .ok_or_else(|| object_store::Error::Generic {
                    store: "offline-local-budget",
                    source: "object is outside the private offline database".into(),
                })?;
        let mut file = self.root.clone();
        for component in suffix {
            file.push(component.as_ref());
            match std::fs::symlink_metadata(&file) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(object_store::Error::Generic {
                        store: "offline-local-budget",
                        source: "symlinks are forbidden in the offline database".into(),
                    });
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(object_store::Error::Generic {
                        store: "offline-local-budget",
                        source: Box::new(error),
                    });
                }
            }
        }
        Ok(())
    }

    fn lock_index(&self, path: &Path) -> usize {
        let digest = blake3::hash(path.as_ref().as_bytes());
        digest.as_bytes()[0] as usize % self.locks.len()
    }

    async fn old_cost(&self, path: &Path) -> object_store::Result<u64> {
        match self.inner.head(path).await {
            Ok(meta) => Ok(meta.size.saturating_add(4096)),
            Err(object_store::Error::NotFound { .. }) => Ok(0),
            Err(error) => Err(error),
        }
    }

    fn release(&self, bytes: u64) {
        let _ = self.budget.used.fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |used| Some(used.saturating_sub(bytes)),
        );
    }
}

#[async_trait::async_trait]
impl object_store::ObjectStore for LocalBudgetStore {
    async fn put_opts(
        &self,
        path: &Path,
        payload: object_store::PutPayload,
        options: object_store::PutOptions,
    ) -> object_store::Result<object_store::PutResult> {
        let _lock = self.locks[self.lock_index(path)].clone().lock_owned().await;
        self.check(path)?;
        let old = self.old_cost(path).await?;
        self.budget.charge(payload.content_length() as u64 + 4096)?;
        let result = self.inner.put_opts(path, payload, options).await;
        if result.is_ok() {
            sync_local_object(self.root.clone(), self.local_path(path)?, true).await?;
            self.release(old);
        }
        result
    }

    async fn put_multipart_opts(
        &self,
        path: &Path,
        options: object_store::PutMultipartOptions,
    ) -> object_store::Result<Box<dyn object_store::MultipartUpload>> {
        let lock = self.locks[self.lock_index(path)].clone().lock_owned().await;
        self.check(path)?;
        let old = self.old_cost(path).await?;
        self.budget.charge(4096)?;
        Ok(Box::new(LocalBudgetUpload {
            inner: self.inner.put_multipart_opts(path, options).await?,
            budget: self.budget.clone(),
            old,
            finished: false,
            root: self.root.clone(),
            path: self.local_path(path)?,
            _lock: Some(lock),
        }))
    }

    async fn get_opts(
        &self,
        path: &Path,
        options: object_store::GetOptions,
    ) -> object_store::Result<object_store::GetResult> {
        self.check(path)?;
        self.inner.get_opts(path, options).await
    }

    fn delete_stream(
        &self,
        locations: futures::stream::BoxStream<'static, object_store::Result<Path>>,
    ) -> futures::stream::BoxStream<'static, object_store::Result<Path>> {
        // Store-owned state is cloned because ObjectStore's deletion stream is static.
        let inner = self.inner.clone();
        let root = self.root.clone();
        let prefix = self.prefix.clone();
        let budget = self.budget.clone();
        let locks = self.locks.clone();
        locations
            .then(move |path| {
                let store = Self {
                    inner: inner.clone(),
                    root: root.clone(),
                    prefix: prefix.clone(),
                    budget: budget.clone(),
                    locks: locks.clone(),
                };
                async move {
                    let path = path?;
                    let _lock = store.locks[store.lock_index(&path)]
                        .clone()
                        .lock_owned()
                        .await;
                    store.check(&path)?;
                    let old = store.old_cost(&path).await?;
                    store.inner.delete(&path).await?;
                    sync_local_object(store.root.clone(), store.local_path(&path)?, false).await?;
                    store.release(old);
                    Ok(path)
                }
            })
            .boxed()
    }

    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> futures::stream::BoxStream<'static, object_store::Result<object_store::ObjectMeta>> {
        let prefix = prefix.unwrap_or(&self.prefix);
        if let Err(error) = self.check(prefix) {
            return futures::stream::once(async move { Err(error) }).boxed();
        }
        self.inner.list(Some(prefix))
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&Path>,
    ) -> object_store::Result<object_store::ListResult> {
        let prefix = prefix.unwrap_or(&self.prefix);
        self.check(prefix)?;
        self.inner.list_with_delimiter(Some(prefix)).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: object_store::CopyOptions,
    ) -> object_store::Result<()> {
        let from_index = self.lock_index(from);
        let to_index = self.lock_index(to);
        let _first = self.locks[from_index.min(to_index)]
            .clone()
            .lock_owned()
            .await;
        let _second = if from_index == to_index {
            None
        } else {
            Some(
                self.locks[from_index.max(to_index)]
                    .clone()
                    .lock_owned()
                    .await,
            )
        };
        self.check(from)?;
        self.check(to)?;
        let source = self.inner.head(from).await?.size.saturating_add(4096);
        let old = self.old_cost(to).await?;
        self.budget.charge(source)?;
        let result = self.inner.copy_opts(from, to, options).await;
        if result.is_ok() {
            sync_local_object(self.root.clone(), self.local_path(to)?, true).await?;
            self.release(old);
        }
        result
    }
}

#[derive(Debug)]
struct LocalBudgetUpload {
    inner: Box<dyn object_store::MultipartUpload>,
    budget: Arc<WriteBudget>,
    old: u64,
    finished: bool,
    root: std::path::PathBuf,
    path: std::path::PathBuf,
    _lock: Option<futures::lock::OwnedMutexGuard<()>>,
}

#[async_trait::async_trait]
impl object_store::MultipartUpload for LocalBudgetUpload {
    fn put_part(&mut self, payload: object_store::PutPayload) -> object_store::UploadPart {
        if self.finished {
            return Box::pin(async {
                Err(object_store::Error::Generic {
                    store: "offline-local-budget",
                    source: "multipart upload already finished".into(),
                })
            });
        }
        let bytes = payload.content_length() as u64;
        if let Err(error) = self.budget.charge(bytes) {
            return Box::pin(async move { Err(error) });
        }
        self.inner.put_part(payload)
    }
    async fn complete(&mut self) -> object_store::Result<object_store::PutResult> {
        let result = self.inner.complete().await;
        if result.is_ok() && !self.finished {
            sync_local_object(self.root.clone(), self.path.clone(), true).await?;
            self.finished = true;
            let _ = self.budget.used.fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |used| Some(used.saturating_sub(self.old)),
            );
            drop(self._lock.take());
        }
        result
    }
    async fn abort(&mut self) -> object_store::Result<()> {
        let result = self.inner.abort().await;
        if result.is_ok() && !self.finished {
            self.finished = true;
            // LocalFileSystem unlinks the staging path without waiting for
            // in-flight part writers. They can still retain its disk blocks.
            // Keep this reservation until the next exclusive startup scan.
            drop(self._lock.take());
        }
        result
    }
}

#[derive(Debug)]
struct LocalBudgetProvider(Arc<LocalBudgetStore>);

#[async_trait::async_trait]
impl lance_io::object_store::ObjectStoreProvider for LocalBudgetProvider {
    async fn new_store(
        &self,
        url: flow_like_types::reqwest::Url,
        params: &ObjectStoreParams,
    ) -> lance::Result<ObjectStore> {
        let path = self.extract_path(&url)?;
        self.0
            .check(&path)
            .map_err(|error| lance::Error::io_source(Box::new(error)))?;
        Ok(ObjectStore::new(
            self.0.clone(),
            url,
            params.block_size,
            None,
            false,
            false,
            16,
            0,
            None,
        ))
    }
    fn extract_path(&self, url: &flow_like_types::reqwest::Url) -> lance::Result<Path> {
        let local = flow_like_types::reqwest::Url::parse(&url.as_str().replacen(
            "file-object-store:",
            "file:",
            1,
        ))
        .map_err(|error| lance::Error::invalid_input(error.to_string()))?;
        let file = local
            .to_file_path()
            .map_err(|_| lance::Error::invalid_input("invalid private offline database URI"))?;
        let path = Path::from_absolute_path(&file)
            .map_err(|error| lance::Error::invalid_input(error.to_string()))?;
        self.0
            .check(&path)
            .map_err(|error| lance::Error::io_source(Box::new(error)))?;
        Ok(path)
    }
    fn calculate_object_store_prefix(
        &self,
        url: &flow_like_types::reqwest::Url,
        _: Option<&HashMap<String, String>>,
    ) -> lance::Result<String> {
        self.extract_path(url)?;
        Ok(format!("offline-local:{}", self.0.prefix))
    }
}

/// The owner must hold an exclusive process lock for `root` for this connection's
/// lifetime. Reuse this connection for every overlay table in the directory.
/// The one-time usage scan also counts abandoned fragments from prior crashes.
pub async fn budgeted_local_connection(
    root: &std::path::Path,
    maximum_bytes: u64,
) -> Result<Connection> {
    ensure!(
        maximum_bytes > 0,
        "offline local database budget must be positive"
    );
    ensure!(
        !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
        "offline database root cannot be a symlink"
    );
    let root = root.canonicalize()?;
    ensure!(root.is_dir(), "offline database root must be a directory");
    flush_durable_local(&root).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            std::fs::metadata(&root)?.permissions().mode() & 0o077 == 0,
            "offline database directory must be private (0700)"
        );
    }
    let mut used = 0_u64;
    let mut stack = vec![std::fs::read_dir(&root)?];
    while let Some(directory) = stack.last_mut() {
        match directory.next() {
            None => {
                stack.pop();
            }
            Some(entry) => {
                let entry = entry?;
                let metadata = std::fs::symlink_metadata(entry.path())?;
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "symlinks are forbidden in the offline database"
                );
                if metadata.is_dir() {
                    ensure!(
                        stack.len() < 64,
                        "offline database directory nesting exceeds 64 levels"
                    );
                    stack.push(std::fs::read_dir(entry.path())?);
                } else {
                    ensure!(
                        metadata.is_file(),
                        "offline database contains a non-regular file"
                    );
                    used = used
                        .checked_add(metadata.len().saturating_add(4096))
                        .filter(|used| *used <= maximum_bytes)
                        .ok_or_else(|| {
                            anyhow!("existing offline database exceeds its disk budget")
                        })?;
                }
            }
        }
    }
    let budget = Arc::new(WriteBudget {
        maximum: maximum_bytes,
        used: std::sync::atomic::AtomicU64::new(used),
    });
    let store = Arc::new(LocalBudgetStore {
        inner: Arc::new(object_store::local::LocalFileSystem::new()),
        prefix: Path::from_absolute_path(&root)?,
        root: root.clone(),
        budget,
        locks: (0..64)
            .map(|_| Arc::new(futures::lock::Mutex::new(())))
            .collect(),
    });
    let registry = Arc::new(lance_io::object_store::ObjectStoreRegistry::empty());
    registry.insert("file-object-store", Arc::new(LocalBudgetProvider(store)));
    let session = Arc::new(lance::session::Session::new(
        16 * 1024 * 1024,
        16 * 1024 * 1024,
        registry,
    ));
    let uri = uri_to_url(
        root.to_str()
            .ok_or_else(|| anyhow!("offline database path is not UTF-8"))?,
    )?;
    // The native `file` scheme bypasses ObjectStore wrappers for reads, writes
    // and copies. This scheme routes every operation through the budget.
    let uri = uri.as_str().replacen("file:", "file-object-store:", 1);
    Ok(connect_lance(&uri).session(session).execute().await?)
}

/// Prune obsolete history without rewriting the current local table or changing
/// its version. Tags and objects referenced by other branches remain protected.
/// The caller must hold the directory's exclusive process lock, have no readers
/// or writes in progress, and need no historical version for pending replay or
/// rollback. Run this only at startup after checking the durable outbox is idle
/// and its acknowledged local baseline equals the selected branch's latest.
pub async fn compact_idle_local(table: &Table) -> Result<lance::dataset::cleanup::RemovalStats> {
    let mut dataset = table
        .dataset()
        .ok_or_else(|| anyhow!("offline cleanup requires a native Lance table"))?
        .get()
        .await?
        .as_ref()
        .clone();
    ensure!(
        dataset.object_store(None).await?.scheme() == "file-object-store",
        "offline cleanup requires a budgeted_local_connection table"
    );
    dataset.checkout_latest().await?;
    Ok(dataset
        .cleanup_with_policy(lance::dataset::cleanup::CleanupPolicy {
            before_version: Some(dataset.manifest().version),
            // Exclusive startup access excludes in-flight fragments. Lance
            // still limits reclamation to objects no newer than its earliest
            // retained manifest, including when this flag is enabled.
            delete_unverified: true,
            error_if_tagged_old_versions: false,
            // Preserve all history of other branches. Lance still protects the
            // parent objects and manifests referenced by those branches.
            clean_referenced_branches: false,
            ..Default::default()
        })
        .await?)
}

/// Stream a pinned snapshot into a fresh private local connection. The caller
/// owns that directory and removes it on cancellation or failure. No partial
/// table manifest is published if the input stream or budget check fails.
/// `maximum_bytes` caps both accumulated Arrow/IPC bytes and destination object
/// writes; filesystem allocation and directory metadata add platform overhead.
pub async fn materialize(
    source: &Table,
    destination: &Connection,
    name: &str,
    maximum_bytes: u64,
) -> Result<MaterializedTable> {
    ensure!(
        maximum_bytes >= 1024 * 1024,
        "offline snapshot budget must be at least 1 MiB"
    );
    ensure!(
        uri_to_url(destination.uri())?.scheme() == "file-object-store",
        "offline snapshots require a budgeted_local_connection destination"
    );
    super::lancedb::LanceDBVectorStore::validate_table_name(name)?;
    match destination.open_table(name).execute().await {
        Ok(_) => anyhow::bail!("offline snapshot destination already exists"),
        Err(lancedb::Error::TableNotFound { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let dataset = source
        .dataset()
        .ok_or_else(|| anyhow!("offline snapshots require a native Lance table"))?
        .get()
        .await?;
    // Pin the existing handle rather than switching to latest halfway through.
    let dataset = Arc::new(dataset.checkout_version(dataset.manifest().version).await?);
    let source_version = dataset.manifest().version;
    let source_fingerprint = fingerprint(&dataset).await?;
    let mut scan = dataset.scan();
    scan.batch_size(1024)
        .batch_size_bytes(1024 * 1024)
        .batch_readahead(1);
    let schema = scan.schema().await?;
    let input = scan.try_into_stream().await?;
    let mut arrow_bytes = 0_u64;
    let mut serialized_bytes = 0_u64;
    let stream = input.map(move |batch| {
        let batch = batch.map_err(lancedb::Error::from)?;
        let size = batch.get_array_memory_size() as u64;
        let failure = |message: &str| lancedb::Error::InvalidInput {
            message: message.into(),
        };
        if size > maximum_bytes.min(8 * 1024 * 1024) {
            return Err(failure(
                "offline snapshot Arrow batch exceeds its memory budget",
            ));
        }
        arrow_bytes = arrow_bytes
            .checked_add(size)
            .filter(|size| *size <= maximum_bytes)
            .ok_or_else(|| failure("offline snapshot exceeds Arrow byte budget"))?;
        let mut counter = ByteCounter {
            bytes: 0,
            maximum: maximum_bytes - serialized_bytes,
        };
        {
            let mut writer =
                arrow::ipc::writer::StreamWriter::try_new(&mut counter, &batch.schema())
                    .map_err(|error| failure(&error.to_string()))?;
            writer
                .write(&batch)
                .map_err(|error| failure(&error.to_string()))?;
            writer
                .finish()
                .map_err(|error| failure(&error.to_string()))?;
        }
        serialized_bytes += counter.bytes;
        Ok(batch)
    });
    let stream: lancedb::arrow::SendableRecordBatchStream =
        Box::pin(lancedb::arrow::SimpleRecordBatchStream { schema, stream });
    let budget = Arc::new(WriteBudget {
        maximum: maximum_bytes,
        used: std::sync::atomic::AtomicU64::new(0),
    });
    let params = WriteParams {
        mode: WriteMode::Create,
        max_rows_per_file: 64 * 1024,
        max_rows_per_group: 1024,
        max_bytes_per_file: maximum_bytes.min(16 * 1024 * 1024) as usize,
        store_params: Some(ObjectStoreParams {
            object_store_wrapper: Some(Arc::new(BudgetWrapper(budget))),
            ..Default::default()
        }),
        skip_auto_cleanup: true,
        ..Default::default()
    };
    let result = destination
        .create_table(name, stream)
        .write_options(WriteOptions {
            lance_write_params: Some(params),
        })
        .execute()
        .await;
    let table = match result {
        Ok(table) => table,
        Err(error @ lancedb::Error::TableAlreadyExists { .. }) => return Err(error.into()),
        Err(error) => {
            // The private destination is exclusively owned by this writer. A
            // missing manifest can still leave fragments, and drop_table removes
            // that prefix without needing a readable table manifest.
            match destination.drop_table(name, &[]).await {
                Ok(()) | Err(lancedb::Error::TableNotFound { .. }) => {}
                Err(cleanup) => {
                    return Err(anyhow!(
                        "offline snapshot failed: {error}; partial snapshot cleanup also failed: {cleanup}"
                    ));
                }
            }
            return Err(error.into());
        }
    };
    // Do not retain the one-shot writer budget on future local overlay commits.
    let table = destination.open_table(table.name()).execute().await?;
    Ok(MaterializedTable {
        table,
        source_version,
        source_fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn connection() -> Result<(Directory, Connection)> {
        let path = std::env::temp_dir().join(format!(
            "flow-like offline-replay-{}",
            flow_like_types::create_id()
        ));
        std::fs::create_dir_all(&path)?;
        let connection = lancedb::connect(path.to_str().unwrap()).execute().await?;
        Ok((Directory(path), connection))
    }

    async fn table(connection: &Connection) -> Result<Table> {
        let reader = crate::arrow_utils::value_to_batch_reader_with_utc_timestamp_inference(vec![
            json!({"id": 1, "value": "base"}),
        ])?;
        Ok(connection.create_table("records", reader).execute().await?)
    }

    async fn marker(table: &Table) -> Result<ReplayMarker> {
        let (expected_version, fingerprint) = revision(table).await?;
        Ok(ReplayMarker {
            operation_id: flow_like_types::create_id(),
            digest: "payload-digest".into(),
            expected_version,
            expected_fingerprint: Some(fingerprint),
        })
    }

    #[tokio::test]
    async fn lost_response_is_proven_after_another_writer_advances() -> Result<()> {
        let (_directory, connection) = connection().await?;
        let table = table(&connection).await?;
        let marker = marker(&table).await?;
        let operation = ReplayMutation::Insert {
            items: vec![json!({"id": 2, "value": "offline"})],
        };
        let first = replay(&table, &marker, operation.clone()).await?;
        assert!(matches!(first, ReplayOutcome::Applied { version: 2, .. }));
        table
            .add(crate::arrow_utils::value_to_batch_reader_with_fields(
                vec![json!({"id": 3, "value": "other writer"})],
                Some(table.schema().await?.fields().iter().cloned().collect()),
            )?)
            .execute()
            .await?;
        assert_eq!(reconcile(&table, &marker).await?, first);
        assert_eq!(replay(&table, &marker, operation).await?, first);
        table.checkout_latest().await?;
        assert_eq!(table.count_rows(None).await?, 3);
        Ok(())
    }

    #[tokio::test]
    async fn guard_rejects_lances_automatic_compatible_append_rebase() -> Result<()> {
        let (_directory, connection) = connection().await?;
        let table = table(&connection).await?;
        let marker = marker(&table).await?;
        let guarded = guarded_dataset(&table, &marker).await?;
        let fields = table
            .schema()
            .await?
            .fields()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        table
            .add(crate::arrow_utils::value_to_batch_reader_with_fields(
                vec![json!({"id": 2, "value": "other writer"})],
                Some(fields.clone()),
            )?)
            .execute()
            .await?;
        let reader = crate::arrow_utils::value_to_batch_reader_with_fields(
            vec![json!({"id": 3, "value": "must conflict"})],
            Some(fields),
        )?;
        let params = WriteParams {
            mode: WriteMode::Append,
            ..Default::default()
        };
        assert!(
            InsertBuilder::new(guarded)
                .with_params(&params)
                .execute_stream(reader)
                .await
                .is_err()
        );
        assert_eq!(
            reconcile(&table, &marker).await?,
            ReplayOutcome::Conflict { actual_version: 2 }
        );
        table.checkout_latest().await?;
        assert_eq!(table.count_rows(None).await?, 2);
        Ok(())
    }

    #[tokio::test]
    async fn same_version_recreation_fails_the_fingerprint_precondition() -> Result<()> {
        let (_directory, connection) = connection().await?;
        let old = table(&connection).await?;
        let marker = marker(&old).await?;
        connection.drop_table("records", &[]).await?;
        let recreated = table(&connection).await?;
        assert_eq!(
            reconcile(&recreated, &marker).await?,
            ReplayOutcome::Conflict { actual_version: 1 }
        );
        assert_eq!(
            replay(
                &recreated,
                &marker,
                ReplayMutation::Delete {
                    filter: "true".into()
                }
            )
            .await?,
            ReplayOutcome::Conflict { actual_version: 1 }
        );
        assert_eq!(recreated.count_rows(None).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn conditional_upsert_update_delete_work_on_a_local_branch() -> Result<()> {
        let (_directory, connection) = connection().await?;
        let original = table(&connection).await?;
        let branch = original.create_branch("pending", 1_u64).await?;
        for operation in [
            ReplayMutation::Upsert {
                items: vec![json!({"id": 2, "value": "pending"})],
                id_field: "id".into(),
            },
            ReplayMutation::Update {
                filter: "id = 2".into(),
                updates: vec![("value".into(), "'changed'".into())],
            },
            ReplayMutation::Delete {
                filter: "id = 1".into(),
            },
        ] {
            let marker = marker(&branch).await?;
            assert!(matches!(
                replay(&branch, &marker, operation).await?,
                ReplayOutcome::Applied { .. }
            ));
            assert!(matches!(
                reconcile(&branch, &marker).await?,
                ReplayOutcome::Applied { .. }
            ));
        }
        branch.checkout_latest().await?;
        assert_eq!(
            branch
                .count_rows(Some("id = 2 AND value = 'changed'".into()))
                .await?,
            1
        );
        assert_eq!(
            original
                .count_rows(Some("id = 1 AND value = 'base'".into()))
                .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn create_only_retry_recovers_its_receipt_without_duplicate_rows() -> Result<()> {
        let (_directory, connection) = connection().await?;
        let marker = ReplayMarker {
            operation_id: flow_like_types::create_id(),
            digest: "create-digest".into(),
            expected_version: 0,
            expected_fingerprint: None,
        };
        let items = vec![json!({"id": 1})];
        let outcome = create(&connection, "created", &marker, items.clone()).await?;
        assert!(matches!(outcome, ReplayOutcome::Applied { version: 1, .. }));
        assert_eq!(
            create(&connection, "created", &marker, items.clone()).await?,
            outcome
        );
        let competitor = ReplayMarker {
            operation_id: flow_like_types::create_id(),
            ..marker
        };
        assert_eq!(
            create(&connection, "created", &competitor, items).await?,
            ReplayOutcome::Conflict { actual_version: 1 }
        );
        assert_eq!(
            connection
                .open_table("created")
                .execute()
                .await?
                .count_rows(None)
                .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn materialization_has_a_pinned_revision_and_rejects_over_budget_input() -> Result<()> {
        let (_source_directory, source_connection) = connection().await?;
        let source = table(&source_connection).await?;
        let (destination_directory, _) = connection().await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &destination_directory.0,
                std::fs::Permissions::from_mode(0o700),
            )?;
        }
        let destination =
            budgeted_local_connection(&destination_directory.0, 16 * 1024 * 1024).await?;
        let result = materialize(&source, &destination, "snapshot", 1024 * 1024).await?;
        assert_eq!(
            (result.source_version, result.source_fingerprint),
            revision(&source).await?
        );
        assert_eq!(result.table.count_rows(None).await?, 1);
        let large = source_connection
            .create_table(
                "large",
                crate::arrow_utils::value_to_batch_reader_with_utc_timestamp_inference(vec![
                    json!({"id": 1, "value": "x".repeat(2 * 1024 * 1024)}),
                ])?,
            )
            .execute()
            .await?;
        assert!(
            materialize(&large, &destination, "too_large", 1024 * 1024)
                .await
                .is_err()
        );
        assert!(matches!(
            destination.open_table("too_large").execute().await,
            Err(lancedb::Error::TableNotFound { .. })
        ));
        Ok(())
    }

    #[test]
    fn replay_expressions_reject_volatile_and_external_evaluation() {
        for expression in [
            "id IN (1, 2) AND value = 'now()'",
            "id + 1",
            "'literal'",
            "NULL",
            "-12",
            "true",
        ] {
            assert!(validate_expression(expression).is_ok(), "{expression}");
        }
        for expression in [
            "random() > 0.5",
            "current_timestamp",
            "now()",
            "id IN (SELECT id FROM other)",
            "1; SELECT 2",
            "CAST('today' AS DATE)",
        ] {
            assert!(validate_expression(expression).is_err(), "{expression}");
        }
    }

    #[test]
    fn key_deletes_merge_into_one_flat_in_list() {
        let mut filter = "`id` = 0".to_string();
        for id in 1..200 {
            filter = merge_key_deletes(&filter, &format!("`id` = {id}")).unwrap();
        }
        let expected: Vec<_> = (0..200).map(|id| id.to_string()).collect();
        assert_eq!(filter, format!("`id` IN ({})", expected.join(", ")));
        validate_expression(&filter).unwrap();

        assert_eq!(
            merge_key_deletes("`k` IN ('a', 'it''s')", "`k` = -3").unwrap(),
            "`k` IN ('a', 'it''s', -3)"
        );
        assert_eq!(merge_key_deletes("false", "`k` = 1").unwrap(), "`k` IN (1)");
        assert_eq!(merge_key_deletes("false", "false").unwrap(), "false");
        for (first, next) in [
            ("`k` = 1", "`other` = 2"),
            ("`k` = 1", "`k` = 2 OR `k` = 3"),
            ("`k` = 1", "`k` NOT IN (2)"),
            ("`k` = 1", "`k` > 2"),
            ("`k` = 1", "`k` = now()"),
        ] {
            assert!(merge_key_deletes(first, next).is_none(), "{first} + {next}");
        }
    }

    #[tokio::test]
    async fn destination_write_budget_fences_multipart_and_ordinary_puts() -> Result<()> {
        let memory = Arc::new(object_store::memory::InMemory::new());
        let store = BudgetedStore {
            inner: memory.clone(),
            budget: Arc::new(WriteBudget {
                maximum: 4098,
                used: std::sync::atomic::AtomicU64::new(0),
            }),
        };
        use object_store::ObjectStore;
        let mut upload = store
            .put_multipart_opts(&Path::from("part"), Default::default())
            .await?;
        upload.put_part("ok".into()).await?;
        assert!(upload.put_part("overflow".into()).await.is_err());
        upload.abort().await?;
        assert!(store.put(&Path::from("another"), "x".into()).await.is_err());
        assert!(memory.get(&Path::from("another")).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn aborted_local_upload_keeps_its_reservation_until_restart() -> Result<()> {
        use object_store::ObjectStore;
        let (directory, _) = connection().await?;
        let root = directory.0.canonicalize()?;
        let budget = Arc::new(WriteBudget {
            maximum: 8192,
            used: std::sync::atomic::AtomicU64::new(0),
        });
        let store = LocalBudgetStore {
            inner: Arc::new(object_store::local::LocalFileSystem::new()),
            prefix: Path::from_absolute_path(&root)?,
            root: root.clone(),
            budget: budget.clone(),
            locks: (0..64)
                .map(|_| Arc::new(futures::lock::Mutex::new(())))
                .collect(),
        };
        let path = Path::from_absolute_path(root.join("aborted"))?;
        let mut upload = store.put_multipart_opts(&path, Default::default()).await?;
        upload.put_part("payload".into()).await?;
        let reserved = budget.used.load(std::sync::atomic::Ordering::Acquire);
        upload.abort().await?;
        assert_eq!(
            budget.used.load(std::sync::atomic::Ordering::Acquire),
            reserved
        );
        assert!(
            store
                .put(&Path::from_absolute_path(root.join("next"))?, "more".into())
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn idle_cleanup_retains_current_data_tags_and_dependent_branches() -> Result<()> {
        async fn update(table: &Table, value: &str) -> Result<()> {
            let marker = marker(table).await?;
            assert!(matches!(
                replay(
                    table,
                    &marker,
                    ReplayMutation::Update {
                        filter: "true".into(),
                        updates: vec![("value".into(), format!("'{value}'"))],
                    },
                )
                .await?,
                ReplayOutcome::Applied { .. }
            ));
            table.checkout_latest().await?;
            Ok(())
        }
        let (directory, _) = connection().await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700))?;
        }
        let connection = budgeted_local_connection(&directory.0, 8 * 1024 * 1024).await?;
        let main = table(&connection).await?;
        let branch = main.create_branch("kept", 1_u64).await?;
        update(&main, "main-tagged").await?;
        main.tags().await?.create("main-history", 2).await?;
        update(&main, "main-discarded").await?;
        update(&main, "main-latest").await?;

        branch.tags().await?.create("branch-history", 1).await?;
        update(&branch, "branch-parent").await?;
        let descendant = branch.create_branch("descendant", ("kept", 2_u64)).await?;
        update(&branch, "branch-discarded").await?;
        update(&branch, "branch-latest").await?;
        let main_revision = revision(&main).await?;
        let branch_revision = revision(&branch).await?;
        let descendant_revision = revision(&descendant).await?;

        let main_stats = compact_idle_local(&main).await?;
        let branch_stats = compact_idle_local(&branch).await?;
        // Lance bounds object listings by the earliest retained manifest's
        // timestamp. Early tags can defer newer orphan data files even after
        // their obsolete manifests have been removed.
        assert!(main_stats.old_versions > 0 && main_stats.bytes_removed > 0);
        assert!(branch_stats.old_versions > 0 && branch_stats.bytes_removed > 0);
        assert_eq!(revision(&main).await?, main_revision);
        assert_eq!(revision(&branch).await?, branch_revision);
        assert_eq!(revision(&descendant).await?, descendant_revision);
        assert_eq!(
            main.count_rows(Some("value = 'main-latest'".into()))
                .await?,
            1
        );
        assert_eq!(
            branch
                .count_rows(Some("value = 'branch-latest'".into()))
                .await?,
            1
        );
        assert_eq!(
            descendant
                .count_rows(Some("value = 'branch-parent'".into()))
                .await?,
            1
        );
        main.checkout_tag("main-history").await?;
        assert_eq!(
            main.count_rows(Some("value = 'main-tagged'".into()))
                .await?,
            1
        );
        branch.checkout_tag("branch-history").await?;
        assert_eq!(branch.count_rows(Some("value = 'base'".into())).await?, 1);
        main.checkout_latest().await?;
        branch.checkout_latest().await?;
        main.tags().await?.delete("main-history").await?;
        branch.tags().await?.delete("branch-history").await?;
        assert!(compact_idle_local(&main).await?.data_files_removed > 0);
        assert!(compact_idle_local(&branch).await?.data_files_removed > 0);
        assert_eq!(revision(&main).await?, main_revision);
        assert_eq!(revision(&branch).await?, branch_revision);
        assert_eq!(
            descendant
                .count_rows(Some("value = 'branch-parent'".into()))
                .await?,
            1
        );
        update(&main, "after-cleanup").await?;
        assert_eq!(
            main.count_rows(Some("value = 'after-cleanup'".into()))
                .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn private_connection_keeps_the_budget_after_reopening_tables() -> Result<()> {
        let (directory, _) = connection().await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700))?;
        }
        let connection = budgeted_local_connection(&directory.0, 128 * 1024).await?;
        let source = table(&connection).await?;
        drop(source);
        let reopened = connection.open_table("records").execute().await?;
        let marker = marker(&reopened).await?;
        let payload = (0_u64..32_768)
            .map(|value| blake3::hash(&value.to_le_bytes()).to_hex().to_string())
            .collect::<String>();
        let result = replay(
            &reopened,
            &marker,
            ReplayMutation::Insert {
                items: vec![json!({"id": 2, "value": payload})],
            },
        )
        .await;
        assert!(result.is_err());
        reopened.checkout_latest().await?;
        assert_eq!(reopened.count_rows(None).await?, 1);
        let outside = directory
            .0
            .parent()
            .unwrap()
            .join("outside-private-offline-table.lance");
        let session = reopened.dataset().unwrap().get().await?.session();
        assert!(
            DatasetBuilder::from_uri(outside.to_str().unwrap())
                .with_session(session)
                .load()
                .await
                .is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_connection_rejects_symlinks_during_its_usage_scan() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (directory, _) = connection().await?;
        std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700))?;
        symlink(std::env::temp_dir(), directory.0.join("escape"))?;
        assert!(
            budgeted_local_connection(&directory.0, 1024 * 1024)
                .await
                .is_err()
        );
        Ok(())
    }
}
