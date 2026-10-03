//! Load an ONNX embedding model through the exact path `LocalEmbeddingModel` uses and report
//! whether it is usable as a bit, then score it on a small multilingual retrieval set.
//!
//! Run with `--features local-ml`.
//! Usage: embedding_probe <dir> <Mean|CLS> <max_tokens>
//! `<dir>` holds model.onnx plus the four tokenizer files a bit's dependencies supply.

use std::{path::PathBuf, time::Instant};

use flow_like_model_provider::{
    embedding::local::embed,
    fastembed::{
        self, InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
    },
    ml::ort_runtime::{ensure_ort_initialized, session_execution_providers},
};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|v| v * v).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|v| v * v).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Passages an index would hold, and queries whose correct answer is the passage at the same
/// position. The first `ANSWERS` entries are the gold passages; everything after them is a hard
/// negative — same topic, same vocabulary, wrong answer — so lexical overlap cannot win and the
/// score reflects ranking rather than keyword matching.
const ANSWERS: usize = 12;

const DOCUMENTS: [&str; 36] = [
    // --- gold passages, one per query ---
    "To rotate a personal access token, open Settings, choose Tokens, revoke the old token and generate a replacement. Existing sessions keep working until the token expires.",
    "Billing runs on the first of each month. Invoices are issued in euros and a failed charge is retried three times over ten days before the workspace is suspended.",
    "Der Exportvorgang schreibt alle Boards, Seiten und Medien in ein einziges Archiv. Sehr große Anwendungen werden in mehrere Teile zerlegt, damit der Upload nicht am Größenlimit scheitert.",
    "Las claves de cifrado se guardan en el almacén regional y nunca salen de la región donde se creó el espacio de trabajo.",
    "fn parse_header(input: &str) -> Result<Header> { let (name, value) = input.split_once(':').ok_or(Error::Malformed)?; Ok(Header::new(name.trim(), value.trim())) }",
    "The scheduler stores each cron expression alongside the event row. Deleting the event cascades to the schedule, but a schedule created directly in the console is not removed.",
    "向量索引在后台重建。重建期间查询仍然可用，返回的是上一个快照的结果。",
    "Nodes that reach the network must declare the NetworkHttp permission. A node without the declaration is refused at compile time rather than failing at run time.",
    "Undo history is kept per board as a timeline plus a cursor. A remote sync merges into the tail and never clears the history; only an explicit reset discards it.",
    "Le fuseau horaire d'un rapport suit celui de l'espace de travail, pas celui du navigateur, afin que deux personnes voient les mêmes totaux journaliers.",
    "Uploading more than one hundred files at once is chunked by the orchestrator. Each chunk is acknowledged before the next begins so a dropped connection resumes rather than restarts.",
    "Row level security is enforced in SQL, not in the API layer, so a query that forgets the tenant predicate returns nothing instead of leaking another tenant's rows.",
    // --- hard negatives: adjacent topic, overlapping vocabulary, wrong answer ---
    "Personal access tokens are listed under Settings with their creation date and last use. The list is read only; creating and revoking happen on the Tokens page.",
    "Service account keys differ from personal access tokens: they never expire, cannot be rotated in the console, and must be replaced by recreating the service account.",
    "Usage is metered per execution and shown on the Billing page in near real time. The figure is an estimate until the invoice closes at the end of the month.",
    "A declined card produces a notification to every workspace owner. Updating the payment method clears the dunning state immediately without waiting for the next retry.",
    "Der Importvorgang liest ein Archiv ein und legt die Boards neu an. Bestehende Boards werden ersetzt oder zusammengeführt, je nach gewähltem Modus.",
    "Medien werden getrennt von den Boards gespeichert und beim Export nur referenziert, wenn das Archiv im schlanken Modus erzeugt wird.",
    "Los datos en tránsito se cifran con TLS y las conexiones internas entre servicios usan certificados rotados automáticamente cada noventa días.",
    "La región del espacio de trabajo se elige al crearlo y no se puede cambiar después sin exportar e importar en un espacio nuevo.",
    "fn parse_query(input: &str) -> Result<Query> { let (key, rest) = input.split_once('=').ok_or(Error::Malformed)?; Ok(Query::new(key.trim(), rest.trim())) }",
    "fn format_header(header: &Header) -> String { format!(\"{}: {}\", header.name(), header.value()) }",
    "Cron expressions are validated when the event is saved. An expression that never fires is rejected rather than stored, so a silent schedule is not possible.",
    "Scheduled runs appear in run history with a sink prefix on the user id, which is how a cron execution is told apart from one a person started.",
    "全文索引与向量索引分开维护。全文索引在写入时同步更新，因此新文档可以立即通过关键词搜索到。",
    "重建向量索引会占用较多内存，建议在低峰时段进行，并确保磁盘有两倍索引大小的可用空间。",
    "Nodes that read or write files declare StorageRead and StorageWrite. These are checked at compile time in the same pass as the network permissions.",
    "Native catalog nodes are trusted by default and carry no permission declarations; only WASM nodes must enumerate what they touch.",
    "Board version history is separate from undo: publishing a version hashes the persisted projection and stores it, and versions survive a reset that clears undo.",
    "The undo stack is held per board and per user in the browser, so two people editing the same board do not share a history and cannot undo each other's work.",
    "Le fuseau horaire d'une exécution planifiée suit celui de la planification elle-même, ce qui peut différer du fuseau de l'espace de travail.",
    "Les totaux horaires sont agrégés côté serveur et mis en cache une minute, donc un rafraîchissement immédiat peut afficher la valeur précédente.",
    "The bulk upload orchestrator caps concurrency at eight transfers so a large batch does not saturate the connection or trip rate limits.",
    "A file larger than the single request limit is uploaded with a multipart session; the parts are committed together once the last one lands.",
    "API keys are scoped to a workspace and every request is checked against the caller's role before the query is built, in addition to the SQL predicate.",
    "Audit rows record the tenant id of the actor rather than of the target, so a cross tenant read attempt is attributable even when it returns nothing.",
];

const QUERIES: [&str; 12] = [
    "how do I replace an API key that leaked",
    "when am I charged and what happens if my card is declined",
    "how big can an application archive get before it is split",
    "where are encryption keys physically stored",
    "rust function that splits a header line into name and value",
    "why does my cron job still fire after I deleted the event",
    "can I search while the index is being rebuilt",
    "what do I need to declare for a node to call an external API",
    "does syncing wipe my undo stack",
    "why do my daily totals differ from a colleague's",
    "what happens if the connection drops during a large upload",
    "how is tenant isolation actually enforced",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("dir"));
    let pooling = match args.next().unwrap_or_else(|| "Mean".into()).as_str() {
        "CLS" => fastembed::Pooling::Cls,
        _ => fastembed::Pooling::Mean,
    };
    let max_tokens: usize = args
        .next()
        .unwrap_or_else(|| "512".into())
        .parse()
        .unwrap_or(512);
    // A bit carries `prefix.query` and `prefix.paragraph`; asymmetric models need them.
    let query_prefix = args.next().unwrap_or_default().replace("\\n", "\n");
    let doc_prefix = args.next().unwrap_or_default().replace("\\n", "\n");

    // A bit stores the weights as one file; `LocalEmbeddingModel` reads exactly that file.
    let model = match std::fs::read(dir.join("model.onnx")) {
        Ok(bytes) => bytes,
        Err(e) => return println!("RESULT load_model_failed: {e}"),
    };

    let read = |name: &str| std::fs::read(dir.join(name));
    let files = match (
        read("tokenizer.json"),
        read("config.json"),
        read("special_tokens_map.json"),
        read("tokenizer_config.json"),
    ) {
        (Ok(t), Ok(c), Ok(s), Ok(tc)) => TokenizerFiles {
            tokenizer_file: t,
            config_file: c,
            special_tokens_map_file: s,
            tokenizer_config_file: tc,
        },
        _ => return println!("RESULT missing_tokenizer_files"),
    };

    if let Err(e) = ensure_ort_initialized() {
        return println!("RESULT ort_init_failed: {e}");
    }
    let providers = match session_execution_providers(true) {
        Ok(p) => p,
        Err(e) => return println!("RESULT provider_select_failed: {e}"),
    };

    let user_model = UserDefinedEmbeddingModel::new(model, files).with_pooling(pooling.clone());
    let options = InitOptionsUserDefined::new()
        .with_max_length(max_tokens)
        .with_execution_providers(providers);

    let start = Instant::now();
    let mut embedder = match TextEmbedding::try_new_from_user_defined(user_model, options) {
        Ok(m) => m,
        Err(e) => return println!("RESULT session_failed: {e}"),
    };
    let load_ms = start.elapsed().as_millis();

    let docs: Vec<String> = DOCUMENTS
        .iter()
        .map(|s| format!("{doc_prefix}{s}"))
        .collect();
    let queries: Vec<String> = QUERIES
        .iter()
        .map(|s| format!("{query_prefix}{s}"))
        .collect();

    let start = Instant::now();
    let doc_vectors = match embed(&mut embedder, docs.clone(), Some(12), &pooling) {
        Ok(v) => v,
        Err(e) => return println!("RESULT embed_failed: {e}"),
    };
    let index_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let query_vectors = match embed(&mut embedder, queries.clone(), Some(12), &pooling) {
        Ok(v) => v,
        Err(e) => return println!("RESULT embed_failed: {e}"),
    };
    let query_ms = start.elapsed().as_millis();

    let dims = doc_vectors[0].len();

    // Recall@1 and MRR over the whole set: for each query the correct passage shares its index.
    assert_eq!(doc_vectors.len(), DOCUMENTS.len());
    assert_eq!(query_vectors.len(), ANSWERS);

    let mut hits = 0usize;
    let mut reciprocal = 0.0f64;
    let mut margins = Vec::new();
    for (i, query) in query_vectors.iter().enumerate() {
        let mut scored: Vec<(usize, f32)> = doc_vectors
            .iter()
            .enumerate()
            .map(|(j, doc)| (j, cosine(query, doc)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let rank = scored
            .iter()
            .position(|(j, _)| *j == i)
            .unwrap_or(usize::MAX);
        if rank == 0 {
            hits += 1;
        }
        if rank != usize::MAX {
            reciprocal += 1.0 / (rank as f64 + 1.0);
        }
        // How far the correct passage sits above the best wrong one; negative means it lost.
        let correct = scored
            .iter()
            .find(|(j, _)| *j == i)
            .map(|(_, s)| *s)
            .unwrap_or(0.0);
        let best_wrong = scored
            .iter()
            .find(|(j, _)| *j != i)
            .map(|(_, s)| *s)
            .unwrap_or(0.0);
        margins.push(correct - best_wrong);
    }

    let recall_at_1 = hits as f64 / queries.len() as f64;
    let mrr = reciprocal / queries.len() as f64;
    let mean_margin = margins.iter().sum::<f32>() / margins.len() as f32;

    // Cross-lingual: query 2 is English, its answer is German; query 6 English, answer Chinese.
    let de_en = cosine(&query_vectors[2], &doc_vectors[2]);
    let zh_en = cosine(&query_vectors[6], &doc_vectors[6]);

    // Throughput at document scale, batched the way the runtime plans batches.
    let corpus: Vec<String> = (0..64)
        .map(|i| format!("{} Passage {i}.", DOCUMENTS[i % DOCUMENTS.len()]))
        .collect();
    let bulk = Instant::now();
    let bulk_ok = embed(&mut embedder, corpus.clone(), Some(16), &pooling).is_ok();
    let bulk_ms = bulk.elapsed().as_millis();
    let per_doc = bulk_ms as f64 / corpus.len() as f64;

    println!(
        "RESULT ok dims={dims} load_ms={load_ms} index12_ms={index_ms} query12_ms={query_ms} \
         recall@1={recall_at_1:.3} mrr={mrr:.3} margin={mean_margin:.3} \
         de_en={de_en:.3} zh_en={zh_en:.3} per_doc_ms={per_doc:.1} bulk_ok={bulk_ok}"
    );
}
