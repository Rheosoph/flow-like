mod folder;
mod ingest;
mod search;
mod store;

use std::sync::Arc;

use chrono::Utc;
use flow_like_types::data_classification::DataClassification;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::embed::KnowledgeEmbedder;
use crate::env::{Progress, ToolEnv};
use crate::error::{ToolError, ToolResult, code};
use crate::files::{FileRef, file_text, resolve, workspace_of};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

use folder::{FolderRef, Index, Totals};
pub use search::fuse;
use store::{IndexMeta, Knowledge, meta_row, validate_name};

pub const KNOWLEDGE_CREATE: &str = "knowledge_create";
pub const KNOWLEDGE_ADD: &str = "knowledge_add";
pub const KNOWLEDGE_SEARCH: &str = "knowledge_search";
pub const KNOWLEDGE_LIST: &str = "knowledge_list";
pub const KNOWLEDGE_DROP: &str = "knowledge_drop";

const CAPABILITY: &str = "knowledge";
const DEFAULT_K: usize = 5;
const FAILED_REPORTED: usize = 25;
const PROBE_TEXT: &str = "dimension probe";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KnowledgeCreateArgs {
    /// Lowercase letters, digits and '_' (up to 40), starting with a letter.
    pub name: String,
    /// What the index is for; shown in knowledge_list.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KnowledgeAddArgs {
    pub index: String,
    /// Files to add; adding a file again replaces its chunks.
    #[serde(default)]
    pub files: Vec<FileRef>,
    /// Folders to add recursively. Only new or changed files are indexed again; the folder is
    /// remembered by the index, so `refresh` can re-sync it later.
    #[serde(default)]
    pub folders: Vec<FolderRef>,
    /// With `folders`: also remove the chunks of files that are no longer in the folder.
    #[serde(default)]
    pub sync: bool,
    /// Re-sync every folder the index remembers, removing the chunks of vanished files.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KnowledgeSearchArgs {
    pub query: String,
    /// Indexes to search; every index when omitted.
    #[serde(default)]
    pub indexes: Option<Vec<String>>,
    /// Number of chunks to return (default 5, capped).
    #[serde(default)]
    pub k: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KnowledgeListArgs {
    /// One index to list with its sources; all indexes when omitted.
    #[serde(default)]
    pub index: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KnowledgeDropArgs {
    pub index: String,
}

pub fn specs() -> Vec<ToolSpec> {
    let spec = |name, description, approval, mutating, input_schema| ToolSpec {
        name,
        description,
        capability: Some(CAPABILITY),
        module: "knowledge",
        approval,
        approval_args: &[],
        mutating,
        input_schema,
    };
    vec![
        spec(
            KNOWLEDGE_CREATE,
            "Create a named knowledge index in the workspace, built with the bot's embedding model.",
            Approval::None,
            true,
            schema_of::<KnowledgeCreateArgs>(),
        ),
        spec(
            KNOWLEDGE_ADD,
            "Add files or whole folders to a knowledge index: they are converted to text, split, embedded and stored with their source. Adding a file again replaces its chunks; adding a folder again indexes only new or changed files (sync: true also removes vanished ones); refresh: true re-syncs the folders the index remembers.",
            Approval::None,
            true,
            schema_of::<KnowledgeAddArgs>(),
        ),
        spec(
            KNOWLEDGE_SEARCH,
            "Search one, several or all knowledge indexes (keyword and meaning together). Hits carry the text, the source file handle and the page or offsets.",
            Approval::None,
            false,
            schema_of::<KnowledgeSearchArgs>(),
        ),
        spec(
            KNOWLEDGE_LIST,
            "List the knowledge indexes with their embedding model, size, remembered folders and last sync; name one to see its sources.",
            Approval::None,
            false,
            schema_of::<KnowledgeListArgs>(),
        ),
        spec(
            KNOWLEDGE_DROP,
            "Delete a knowledge index and all its chunks.",
            Approval::Ask,
            true,
            schema_of::<KnowledgeDropArgs>(),
        ),
    ]
}

async fn open(env: &dyn ToolEnv, writing: bool) -> ToolResult<Knowledge> {
    let workspace = workspace_of(env, "keep knowledge indexes")?;
    let tables = env.app_tables(&workspace).await?;
    if writing && !tables.writable {
        return Err(ToolError::refused(
            "The bot's workspace tables are read-only here, so knowledge indexes cannot change",
        ));
    }
    Ok(Knowledge::new(tables.connection))
}

async fn embedder_of(env: &dyn ToolEnv) -> ToolResult<Arc<dyn KnowledgeEmbedder>> {
    env.embedder().await?.ok_or_else(|| {
        ToolError::new(
            code::NO_EMBEDDER,
            "This bot has no embedding model for knowledge indexes. If bot_models is available, recommend a compatible embedding model from the owner's profile, then propose embedding_model with bot_update_configuration for explicit approval. Retry on the next turn after approval. The owner can also choose an embedding model in the bot's settings.",
        )
    })
}

fn mismatch(meta: &IndexMeta, current: &str) -> ToolError {
    ToolError::new(
        code::MODEL_MISMATCH,
        format!(
            "Index '{}' was built with embedding model '{}' but the bot now uses '{current}'. Vectors of different models cannot be mixed: drop the index with knowledge_drop, create it again and re-add the files",
            meta.id, meta.embedding_model
        ),
    )
}

fn ensure_model(meta: &IndexMeta, embedder: &dyn KnowledgeEmbedder) -> ToolResult<()> {
    if meta.embedding_model == embedder.model_id() {
        Ok(())
    } else {
        Err(mismatch(meta, embedder.model_id()))
    }
}

async fn require_index(store: &Knowledge, name: &str) -> ToolResult<IndexMeta> {
    store.meta(name).await?.ok_or_else(|| {
        ToolError::not_found(format!(
            "No knowledge index '{name}'; knowledge_list shows the existing ones"
        ))
    })
}

fn cancelled() -> ToolError {
    ToolError::new(code::CANCELLED, "The call was cancelled")
}

pub async fn knowledge_create(
    env: &dyn ToolEnv,
    args: KnowledgeCreateArgs,
) -> ToolResult<ToolOutput> {
    let name = validate_name(&args.name)?;
    let store = open(env, true).await?;
    let embedder = embedder_of(env).await?;
    if let Some(existing) = store.meta(&name).await? {
        ensure_model(&existing, embedder.as_ref())?;
        return Ok(ToolOutput::new(json!({
            "index": name,
            "created": false,
            "embedding_model": existing.embedding_model,
        })));
    }
    let probe = embedder
        .embed_documents(&[PROBE_TEXT.to_string()])
        .await?
        .into_iter()
        .next()
        .filter(|vector| !vector.is_empty())
        .ok_or_else(|| {
            ToolError::new(
                code::FAILED,
                format!(
                    "Embedding model '{}' returned no vector",
                    embedder.model_id()
                ),
            )
        })?;
    let meta = meta_row(
        &name,
        args.description.as_deref().unwrap_or_default(),
        embedder.model_id(),
        probe.len(),
        Utc::now().timestamp_millis(),
    );
    store.create(&meta).await?;
    Ok(ToolOutput::new(json!({
        "index": name,
        "created": true,
        "embedding_model": meta.embedding_model,
        "dimension": probe.len(),
    })))
}

pub async fn knowledge_add(env: &dyn ToolEnv, args: KnowledgeAddArgs) -> ToolResult<ToolOutput> {
    let limits = env.limits().clone();
    if args.files.is_empty() && args.folders.is_empty() && !args.refresh {
        return Err(ToolError::invalid(
            "Name at least one file or folder to add, or set refresh to re-sync the index's folders",
        ));
    }
    if args.files.len() > limits.max_files_per_add {
        return Err(ToolError::invalid(format!(
            "Add at most {} files per call; split the list",
            limits.max_files_per_add
        )));
    }
    let name = validate_name(&args.index)?;
    let store = open(env, true).await?;
    let embedder = embedder_of(env).await?;
    let meta = require_index(&store, &name).await?;
    ensure_model(&meta, embedder.as_ref())?;
    let dimension = usize::try_from(meta.dimension).unwrap_or_default();

    let mut added = Vec::new();
    let mut failed = Vec::new();
    let mut warnings = Vec::new();
    let mut classification: Option<DataClassification> = None;
    let mut embedded_chars = 0u64;
    let total = args.files.len();
    let folder_runs = folder_runs(&store, &name, &args).await?;
    for (position, reference) in args.files.iter().enumerate() {
        if env.is_cancelled() {
            return Err(cancelled());
        }
        env.progress(Progress {
            message: format!("Adding file {} of {total}", position + 1),
            fraction: Some(position as f32 / total as f32),
        });
        let label = reference.label();
        let outcome = add_one(env, &store, embedder.as_ref(), &name, dimension, reference).await;
        match outcome {
            Ok(done) => {
                classification = classification.max(done.classification);
                embedded_chars += done.chars;
                if done.dropped {
                    warnings.push(format!(
                        "'{label}' was cut at {} chunks",
                        limits.max_chunks_per_source
                    ));
                }
                added.push(done.entry);
            }
            Err(error) if error.code == code::CANCELLED => return Err(error),
            Err(error) => failed.push(json!({
                "file": label,
                "code": error.code,
                "error": error.message,
            })),
        }
    }
    let mut totals = Totals {
        failed,
        warnings,
        classification,
        chars: embedded_chars,
    };
    let index = Index {
        env,
        store: &store,
        embedder: embedder.as_ref(),
        name: &name,
        dimension,
    };
    let mut folders = Vec::new();
    for (folder, sync) in &folder_runs {
        match index.sync_folder(folder, *sync, &mut totals).await {
            Ok(report) => folders.push(report),
            Err(error) if error.code == code::CANCELLED => return Err(error),
            Err(error) => totals.failed.push(json!({
                "file": format!("{folder:?}"),
                "code": error.code,
                "error": error.message,
            })),
        }
    }
    let Totals {
        mut failed,
        warnings,
        classification,
        chars: embedded_chars,
    } = totals;
    let failed_total = failed.len();
    failed.truncate(FAILED_REPORTED);
    if added.is_empty() && folders.is_empty() {
        let first = failed.first().cloned().unwrap_or(Value::Null);
        return Err(ToolError::new(
            first["code"]
                .as_str()
                .and_then(known_code)
                .unwrap_or(code::FAILED),
            format!(
                "Nothing was added to '{name}': {}",
                first["error"].as_str().unwrap_or("unknown error")
            ),
        ));
    }
    let mut result = json!({
        "index": name,
        "added": added,
        "failed": failed,
    });
    if !folder_runs.is_empty() {
        result["folders"] = json!(folders);
    }
    if failed_total > FAILED_REPORTED {
        result["failed_total"] = json!(failed_total);
    }
    let mut output = ToolOutput::new(result).classified(classification);
    output.warnings = warnings;
    output.usage.model_in = embedded_chars / 4;
    Ok(output)
}

fn known_code(value: &str) -> Option<&'static str> {
    [
        code::NOT_FOUND,
        code::SCOPE_REFUSED,
        code::TOO_LARGE,
        code::UNSUPPORTED_FILE,
        code::HANDLE_MISMATCH,
        code::NO_WORKSPACE,
        code::INVALID_ARGS,
    ]
    .into_iter()
    .find(|known| *known == value)
}

struct Added {
    entry: Value,
    classification: Option<DataClassification>,
    chars: u64,
    dropped: bool,
    chunk_count: usize,
}

async fn folder_runs(
    store: &Knowledge,
    index: &str,
    args: &KnowledgeAddArgs,
) -> ToolResult<Vec<(FolderRef, bool)>> {
    let mut runs: Vec<(FolderRef, bool)> = args
        .folders
        .iter()
        .map(|folder| (folder.clone(), args.sync))
        .collect();
    if args.refresh {
        let remembered = store.folders(index).await?;
        if remembered.is_empty() && runs.is_empty() && args.files.is_empty() {
            return Err(ToolError::invalid(format!(
                "Index '{index}' remembers no folders to refresh; add one with folders"
            )));
        }
        for record in remembered {
            let folder = FolderRef {
                app_id: Some(record.app_id),
                prefix: record.prefix,
            };
            match runs.iter_mut().find(|(known, _)| *known == folder) {
                Some(run) => run.1 = true,
                None => runs.push((folder, true)),
            }
        }
    }
    Ok(runs)
}

async fn add_one(
    env: &dyn ToolEnv,
    store: &Knowledge,
    embedder: &dyn KnowledgeEmbedder,
    index: &str,
    dimension: usize,
    reference: &FileRef,
) -> ToolResult<Added> {
    let file = resolve(env, reference, env.limits().max_ingest_bytes).await?;
    index_file(env, store, embedder, index, dimension, file).await
}

async fn index_file(
    env: &dyn ToolEnv,
    store: &Knowledge,
    embedder: &dyn KnowledgeEmbedder,
    index: &str,
    dimension: usize,
    file: crate::files::ResolvedFile,
) -> ToolResult<Added> {
    let limits = env.limits();
    let document = file_text(env, &file).await?.ok_or_else(|| {
        ToolError::new(
            code::UNSUPPORTED_FILE,
            format!(
                "'{}' ({}) cannot be read as text here: no document converter is available for this type",
                file.name, file.mime
            ),
        )
    })?;
    let (chunks, dropped) =
        ingest::chunk_document(embedder, &document, limits.max_chunks_per_source)?;
    if chunks.is_empty() {
        return Err(ToolError::new(
            code::UNSUPPORTED_FILE,
            format!("'{}' has no text to index", file.name),
        ));
    }
    let chars: u64 = chunks.iter().map(|chunk| chunk.text.len() as u64).sum();
    let vectors = ingest::embed_chunks(embedder, &chunks, limits.embed_batch, dimension).await?;
    let count = chunks.len();
    let rows = ingest::chunk_rows(
        &file,
        chunks,
        vectors,
        embedder.model_id(),
        Utc::now().timestamp_millis(),
    )?;
    let replaced = store
        .replace_source(index, &file.source_key(), &rows)
        .await?;
    Ok(Added {
        entry: json!({
            "source": file.handle(),
            "chunks": count,
            "replaced_chunks": replaced,
        }),
        classification: file.classification,
        chars,
        dropped,
        chunk_count: count,
    })
}

pub async fn knowledge_search(
    env: &dyn ToolEnv,
    args: KnowledgeSearchArgs,
) -> ToolResult<ToolOutput> {
    let limits = env.limits();
    if args.query.trim().is_empty() {
        return Err(ToolError::invalid("The query is empty"));
    }
    let k = args.k.unwrap_or(DEFAULT_K).clamp(1, limits.knowledge_max_k);
    let store = open(env, false).await?;
    let metas = selected_indexes(&store, args.indexes).await?;
    if metas.is_empty() {
        return Err(ToolError::not_found(
            "The bot has no knowledge indexes yet; create one with knowledge_create",
        ));
    }
    let embedder = embedder_of(env).await?;
    for meta in &metas {
        ensure_model(meta, embedder.as_ref())?;
    }
    let vector = embedder.embed_query(&args.query).await?;
    let pool = (k * 5).clamp(20, 100);
    let mut rankings = Vec::new();
    for meta in &metas {
        if env.is_cancelled() {
            return Err(cancelled());
        }
        let (nearest, text) = store.rankings(&meta.id, &args.query, &vector, pool).await?;
        rankings.push((meta.id.clone(), nearest, text));
    }
    let hits = fuse(&rankings, k);
    let classification = hits
        .iter()
        .filter_map(|hit| hit.row.get("classification").and_then(Value::as_str))
        .filter_map(DataClassification::parse)
        .max();
    let budget = limits.inline_cap_bytes.saturating_sub(2048) / hits.len().max(1);
    let rendered: Vec<Value> = hits
        .iter()
        .map(|hit| search::hit_json(hit, budget.max(200)))
        .collect();
    Ok(ToolOutput::new(json!({
        "query": args.query,
        "indexes": metas.iter().map(|meta| meta.id.clone()).collect::<Vec<_>>(),
        "hits": rendered,
    }))
    .classified(classification))
}

async fn selected_indexes(
    store: &Knowledge,
    requested: Option<Vec<String>>,
) -> ToolResult<Vec<IndexMeta>> {
    let Some(names) = requested.filter(|names| !names.is_empty()) else {
        return Ok(store.all_meta().await?);
    };
    let mut metas = Vec::new();
    for name in names {
        let name = validate_name(&name)?;
        if !metas.iter().any(|meta: &IndexMeta| meta.id == name) {
            metas.push(require_index(store, &name).await?);
        }
    }
    Ok(metas)
}

pub async fn knowledge_list(env: &dyn ToolEnv, args: KnowledgeListArgs) -> ToolResult<ToolOutput> {
    let store = open(env, false).await?;
    let metas = match args.index.as_deref() {
        Some(name) => vec![require_index(&store, &validate_name(name)?).await?],
        None => store.all_meta().await?,
    };
    let mut indexes = Vec::new();
    for meta in metas {
        let mut entry = json!({
            "name": meta.id,
            "description": meta.description,
            "embedding_model": meta.embedding_model,
            "dimension": meta.dimension,
            "chunks": store.chunk_count(&meta.id).await?,
            "created_at": meta.created_at,
        });
        if args.index.is_some() {
            let sources: Vec<Value> = store
                .sources(&meta.id)
                .await?
                .into_iter()
                .map(|(source, chunks)| json!({ "source": source, "chunks": chunks }))
                .collect();
            entry["sources"] = json!(sources);
        }
        let folders = store.folders(&meta.id).await?;
        if !folders.is_empty() {
            entry["folders"] = json!(
                folders
                    .iter()
                    .map(|folder| json!({
                        "app_id": folder.app_id,
                        "prefix": folder.prefix,
                        "last_sync_ms": folder.last_sync_ms,
                        "files": folder.files,
                        "last_added": folder.added,
                        "last_updated": folder.updated,
                        "last_removed": folder.removed,
                    }))
                    .collect::<Vec<_>>()
            );
        }
        indexes.push(entry);
    }
    Ok(ToolOutput::new(json!({ "indexes": indexes })))
}

pub async fn knowledge_drop(env: &dyn ToolEnv, args: KnowledgeDropArgs) -> ToolResult<ToolOutput> {
    let name = validate_name(&args.index)?;
    let store = open(env, true).await?;
    require_index(&store, &name).await?;
    let chunks = store.chunk_count(&name).await?;
    store.drop(&name).await?;
    Ok(ToolOutput::new(
        json!({ "index": name, "dropped_chunks": chunks }),
    ))
}
