use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use flow_like::{
    bit::{Bit, BitPack},
    flow_like_types::intercom::{InterComCallback, InterComEvent},
    hub::BitSearchQuery,
    utils::download::BitDownloadEvent,
};
use serde_json::{Value, json};

use crate::{
    catalog::{bit_name, bit_type_name, human_bytes, index_by_hash, parse_bit_type, search},
    context::Context,
    spec,
    store::{self, Entry, IssueKind},
};

pub fn status(ctx: &Context, as_json: bool) -> Result<()> {
    let entries = store::scan(&ctx.bit_dir)?;
    let issues = store::issues(&entries);
    let populated = entries.iter().filter(|entry| !entry.is_empty()).count();
    let artifacts: usize = entries.iter().map(|entry| entry.artifacts.len()).sum();
    let total: u64 = entries.iter().map(Entry::size).sum();
    let reclaimable: u64 = issues
        .iter()
        .filter(|issue| issue.removable.is_some())
        .map(|issue| issue.bytes)
        .sum();

    if as_json {
        return print_json(&json!({
            "store": ctx.bit_dir,
            "hub": ctx.hub_domain,
            "directories": entries.len(),
            "populated": populated,
            "artifacts": artifacts,
            "bytes": total,
            "issues": issues.len(),
            "reclaimable_bytes": reclaimable,
        }));
    }

    println!("Store       {}", ctx.bit_dir.display());
    println!("Hub         {}", ctx.hub_domain);
    println!(
        "Bits        {} directories, {populated} with artifacts",
        entries.len()
    );
    println!("Artifacts   {artifacts} files, {}", human_bytes(total));
    println!(
        "Issues      {} ({} removable by doctor --fix, {})",
        issues.len(),
        issues
            .iter()
            .filter(|issue| issue.removable.is_some())
            .count(),
        human_bytes(reclaimable)
    );
    Ok(())
}

pub async fn list(ctx: &Context, resolve: bool, as_json: bool) -> Result<()> {
    let entries = store::scan(&ctx.bit_dir)?;
    let index = if resolve {
        index_by_hash(&ctx.hub().await?).await?
    } else {
        Default::default()
    };

    if as_json {
        let rows: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let bit = index.get(&entry.hash);
                json!({
                    "hash": entry.hash,
                    "bytes": entry.size(),
                    "artifacts": entry.artifacts.iter().map(|artifact| json!({
                        "name": artifact.name,
                        "bytes": artifact.size,
                        "partial": artifact.partial,
                    })).collect::<Vec<_>>(),
                    "bit": bit.map(|bit| json!({
                        "id": bit.id,
                        "name": bit_name(bit),
                        "type": bit_type_name(bit),
                    })),
                })
            })
            .collect();
        return print_json(&json!(rows));
    }

    for entry in &entries {
        let label = match index.get(&entry.hash) {
            Some(bit) => format!("{} [{}]", bit_name(bit), bit_type_name(bit)),
            None if entry.is_empty() => "empty".to_string(),
            None => entry
                .artifacts
                .iter()
                .map(|artifact| artifact.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        };
        println!(
            "{}  {:>10}  {label}",
            short(&entry.hash),
            human_bytes(entry.size())
        );
    }

    println!(
        "\n{} directories, {}",
        entries.len(),
        human_bytes(entries.iter().map(Entry::size).sum())
    );
    Ok(())
}

pub async fn search_catalog(
    ctx: &Context,
    query: Option<String>,
    bit_types: Vec<String>,
    limit: u64,
    as_json: bool,
) -> Result<()> {
    let mut request = BitSearchQuery::builder().with_limit(limit);
    if let Some(query) = query {
        request = request.with_search(&query);
    }
    if !bit_types.is_empty() {
        let parsed = bit_types
            .iter()
            .map(|bit_type| parse_bit_type(bit_type))
            .collect::<Result<Vec<_>>>()?;
        request = request.with_bit_types(parsed);
    }

    let hub = ctx.hub().await?;
    let bits = search(&hub, &request).await?;
    let entries = store::scan(&ctx.bit_dir)?;

    if as_json {
        let rows: Vec<Value> = bits.iter().map(|bit| describe(bit, &entries)).collect();
        return print_json(&json!(rows));
    }

    for bit in &bits {
        println!(
            "{:<26} {:<16} {:>10}  {:<10} {}",
            bit.id,
            bit_type_name(bit),
            human_bytes(bit.size.unwrap_or(0)),
            state(bit, &entries),
            bit_name(bit)
        );
    }

    println!("\n{} bits", bits.len());
    Ok(())
}

pub async fn info(ctx: &Context, bit_id: &str, as_json: bool) -> Result<()> {
    let hub = ctx.hub().await?;
    let bit = hub
        .get_bit(bit_id)
        .await
        .map_err(|err| anyhow!("could not resolve bit {bit_id}: {err}"))?;
    let pack = bit.pack(ctx.state.clone()).await?;
    let entries = store::scan(&ctx.bit_dir)?;

    let artifacts = unique_artifacts(&pack);

    if as_json {
        return print_json(&json!({
            "bit": describe(&bit, &entries),
            "pack_bytes": pack.size(),
            "artifacts": artifacts.iter().map(|bit| describe(bit, &entries)).collect::<Vec<_>>(),
        }));
    }

    println!("Bit         {}", bit.id);
    println!("Name        {}", bit_name(&bit));
    println!("Type        {}", bit_type_name(&bit));
    println!("Hub         {}", bit.hub);
    if let Some(version) = &bit.version {
        println!("Version     {version}");
    }
    println!("Hash        {}", bit.hash);
    println!(
        "Pack        {} artifacts, {}",
        artifacts.len(),
        human_bytes(pack.size())
    );

    let missing = missing_artifacts(&artifacts, &entries);
    for artifact in &artifacts {
        println!(
            "  {:<10} {:>10}  {}  {}",
            state(artifact, &entries),
            human_bytes(artifact.size.unwrap_or(0)),
            short(&artifact.hash),
            artifact
                .file_name
                .clone()
                .unwrap_or_else(|| "-".to_string())
        );
    }

    if !missing.is_empty() {
        println!(
            "\n{} artifacts missing, {} to download",
            missing.len(),
            human_bytes(missing.iter().map(|bit| bit.size.unwrap_or(0)).sum())
        );
    }
    Ok(())
}

pub async fn install(ctx: &Context, bit_ids: &[String]) -> Result<()> {
    if bit_ids.is_empty() {
        bail!("pass at least one bit id");
    }

    let hub = ctx.hub().await?;
    for bit_id in bit_ids {
        let bit = hub
            .get_bit(bit_id)
            .await
            .map_err(|err| anyhow!("could not resolve bit {bit_id}: {err}"))?;
        let pack = bit.pack(ctx.state.clone()).await?;
        println!(
            "installing {} ({}) - {} artifacts, {}",
            bit_name(&bit),
            bit.id,
            unique_artifacts(&pack).len(),
            human_bytes(pack.size())
        );

        let downloaded = pack
            .download(ctx.state.clone(), progress_callback())
            .await?;
        println!("installed {} artifacts for {}", downloaded.len(), bit.id);
    }

    Ok(())
}

pub async fn remove(
    ctx: &Context,
    targets: &[String],
    with_dependencies: bool,
    confirmed: bool,
) -> Result<()> {
    if targets.is_empty() {
        bail!("pass at least one bit id or store hash");
    }

    let entries = store::scan(&ctx.bit_dir)?;
    let mut plan: BTreeMap<String, (PathBuf, u64, String)> = BTreeMap::new();

    for target in targets {
        if let Some(entry) = store::find(&entries, target) {
            plan.insert(
                entry.hash.clone(),
                (entry.path.clone(), entry.size(), target.clone()),
            );
            continue;
        }

        let hub = ctx.hub().await?;
        let bit = hub
            .get_bit(target)
            .await
            .map_err(|err| anyhow!("{target} is neither a store hash nor a known bit: {err}"))?;
        let mut hashes = vec![bit.hash.clone()];
        if with_dependencies {
            let pack = bit.pack(ctx.state.clone()).await?;
            hashes.extend(pack.bits.iter().map(|bit| bit.hash.clone()));
        }

        for hash in hashes {
            let Some(entry) = store::find(&entries, &hash) else {
                continue;
            };
            plan.insert(
                entry.hash.clone(),
                (entry.path.clone(), entry.size(), bit.id.clone()),
            );
        }
    }

    if plan.is_empty() {
        println!("nothing to remove");
        return Ok(());
    }

    let total: u64 = plan.values().map(|(_, size, _)| *size).sum();
    for (hash, (path, size, source)) in &plan {
        println!("{}  {:>10}  {source}", short(hash), human_bytes(*size));
        if !confirmed {
            println!("             {}", path.display());
        }
    }

    if !confirmed {
        println!(
            "\n{} directories, {} — pass --yes to delete",
            plan.len(),
            human_bytes(total)
        );
        return Ok(());
    }

    for (path, _, _) in plan.values() {
        store::remove(path)?;
    }
    println!(
        "\nremoved {} directories, {}",
        plan.len(),
        human_bytes(total)
    );
    Ok(())
}

pub fn doctor(ctx: &Context, fix: bool, as_json: bool) -> Result<()> {
    let entries = store::scan(&ctx.bit_dir)?;
    let issues = store::issues(&entries);

    if as_json {
        let rows: Vec<Value> = issues
            .iter()
            .map(|issue| {
                json!({
                    "hash": issue.hash,
                    "kind": issue.kind.label(),
                    "detail": issue.detail,
                    "bytes": issue.bytes,
                    "removable": issue.removable,
                })
            })
            .collect();
        return print_json(&json!(rows));
    }

    if issues.is_empty() {
        println!("no problems found in {}", ctx.bit_dir.display());
        return Ok(());
    }

    for issue in &issues {
        println!(
            "{:<14} {}  {}",
            issue.kind.label(),
            short(&issue.hash),
            issue.detail
        );
    }

    let removable: Vec<_> = issues
        .iter()
        .filter(|issue| issue.removable.is_some())
        .collect();
    let unfinished = issues
        .iter()
        .filter(|issue| issue.kind == IssueKind::Unfinished)
        .count();

    if !fix {
        println!(
            "\n{} problems, {} removable by --fix{}",
            issues.len(),
            removable.len(),
            if unfinished > 0 {
                format!(", {unfinished} resumable downloads left alone")
            } else {
                String::new()
            }
        );
        return Ok(());
    }

    let mut reclaimed = 0;
    for issue in &removable {
        let Some(path) = &issue.removable else {
            continue;
        };
        store::remove(path)?;
        reclaimed += issue.bytes;
    }
    println!(
        "\nremoved {} entries, {}",
        removable.len(),
        human_bytes(reclaimed)
    );
    Ok(())
}

/// A bit without a download link is served by the hub itself, so there is no
/// artifact to hold locally and it is never missing.
fn state(bit: &Bit, entries: &[Entry]) -> &'static str {
    let Some(file_name) = bit
        .file_name
        .as_ref()
        .filter(|_| bit.download_link.is_some())
    else {
        return "proxied";
    };

    let present = store::find(entries, &bit.hash)
        .and_then(|entry| entry.artifact(file_name))
        .is_some_and(|artifact| bit.size.is_none_or(|size| size == artifact.size));

    if present { "installed" } else { "missing" }
}

fn missing_artifacts<'a>(artifacts: &[&'a Bit], entries: &[Entry]) -> Vec<&'a Bit> {
    artifacts
        .iter()
        .copied()
        .filter(|bit| state(bit, entries) == "missing")
        .collect()
}

/// A dependency tree names shared files more than once. The download path
/// deduplicates by stored artifact, so listings and counts have to match it.
fn unique_artifacts(pack: &BitPack) -> Vec<&Bit> {
    let mut seen = BTreeSet::new();
    pack.bits
        .iter()
        .filter(|bit| seen.insert((bit.hash.clone(), bit.file_name.clone())))
        .collect()
}

fn describe(bit: &Bit, entries: &[Entry]) -> Value {
    json!({
        "id": bit.id,
        "name": bit_name(bit),
        "type": bit_type_name(bit),
        "hash": bit.hash,
        "file_name": bit.file_name,
        "bytes": bit.size,
        "hub": bit.hub,
        "version": bit.version,
        "state": state(bit, entries),
    })
}

/// One progress line per artifact, rate limited so a multi-gigabyte download
/// stays readable in a terminal and in captured output.
fn progress_callback() -> InterComCallback {
    let last: Arc<Mutex<BTreeMap<String, Instant>>> = Arc::new(Mutex::new(BTreeMap::new()));

    Some(Arc::new(move |event: InterComEvent| {
        let last = last.clone();
        Box::pin(async move {
            let Ok(progress) = serde_json::from_value::<BitDownloadEvent>(event.payload) else {
                return Ok(());
            };

            let complete = progress.max > 0 && progress.downloaded >= progress.max;
            {
                let mut last = last.lock().unwrap();
                let previous = last.get(&progress.hash).copied();
                if !complete && previous.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
                    return Ok(());
                }
                last.insert(progress.hash.clone(), Instant::now());
            }

            let percent = if progress.max > 0 {
                progress.downloaded as f64 / progress.max as f64 * 100.0
            } else {
                0.0
            };
            println!(
                "  {} {:>5.1}%  {} / {}",
                progress.path,
                percent,
                human_bytes(progress.downloaded),
                human_bytes(progress.max)
            );
            Ok(())
        })
    }))
}

fn short(hash: &str) -> String {
    hash.chars().take(16).collect()
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Mirrors `GlobalPermission` in the API: the two bits that decide whether a
/// credential may author bits.
const GLOBAL_ADMIN: i64 = 1;
const GLOBAL_WRITE_BITS: i64 = 1024;

pub async fn hub_whoami(ctx: &Context) -> Result<()> {
    let user = ctx.admin()?.whoami().await?;
    let field = |key: &str| {
        user.get(key)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_string()
    };
    let permission = user.get("permission").and_then(Value::as_i64).unwrap_or(0);
    let may_write = permission & (GLOBAL_ADMIN | GLOBAL_WRITE_BITS) != 0;

    println!("Hub         {}", ctx.hub_domain);
    println!("User        {} ({})", field("name"), field("id"));
    println!("Email       {}", field("email"));
    println!("Permissions {permission}");
    println!(
        "Write bits  {}",
        if may_write {
            "yes"
        } else {
            "no, the catalog will refuse every write"
        }
    );
    Ok(())
}

pub async fn hub_pull(
    ctx: &Context,
    bit_id: &str,
    with_dependencies: bool,
    out: Option<PathBuf>,
) -> Result<()> {
    let hub = ctx.hub().await?;
    let bit = hub
        .get_bit(bit_id)
        .await
        .map_err(|err| anyhow!("could not resolve bit {bit_id}: {err}"))?;

    let mut bits = Vec::new();
    if with_dependencies {
        let dependencies = hub
            .get_bit_dependencies(bit_id)
            .await
            .map_err(|err| anyhow!("could not read the dependencies of {bit_id}: {err}"))?;
        let mut seen = BTreeSet::new();
        bits.extend(
            dependencies
                .into_iter()
                .filter(|dependency| dependency.id != bit.id && seen.insert(dependency.id.clone())),
        );
    }
    bits.push(bit);

    let spec = serde_json::to_string_pretty(&spec::from_bits(&bits))?;
    match out {
        Some(path) => {
            std::fs::write(&path, format!("{spec}\n"))
                .map_err(|err| anyhow!("could not write {}: {err}", path.display()))?;
            println!("wrote {} bits to {}", bits.len(), path.display());
        }
        None => println!("{spec}"),
    }
    Ok(())
}

pub async fn hub_push(ctx: &Context, path: &Path, dry_run: bool) -> Result<()> {
    let spec = spec::load(path)?;
    let planned = spec::plan(&spec, &ctx.hub_domain)?;

    if dry_run {
        for entry in &planned {
            let dependencies: Vec<String> = entry
                .dependencies
                .iter()
                .map(spec::Dependency::describe)
                .collect();
            println!(
                "{:<26} {:<16} {}{}",
                entry.id,
                entry.bit_type,
                entry
                    .alias
                    .as_ref()
                    .map(|alias| format!("@{alias} "))
                    .unwrap_or_default(),
                if dependencies.is_empty() {
                    String::new()
                } else {
                    format!("depends on {}", dependencies.join(", "))
                }
            );
        }
        println!(
            "\n{} bits would be written to {} in this order",
            planned.len(),
            ctx.hub_domain
        );
        return Ok(());
    }

    let admin = ctx.admin()?;
    let mut stored_bits: BTreeMap<String, Bit> = BTreeMap::new();
    for entry in &planned {
        println!("pushing {} [{}]", entry.id, entry.bit_type);
        let payload = entry.push_payload(&stored_bits, &ctx.hub_domain)?;

        let mut last = 0.0;
        let mut stored = admin
            .upsert_bit(&payload, |progress| {
                report_stage(progress, &mut last);
            })
            .await?;

        if let Some(update) = spec::hosted_update_payload(&payload, &stored) {
            stored = admin
                .upsert_bit(&update, |progress| {
                    report_stage(progress, &mut last);
                })
                .await?;
        }

        for (language, meta) in &entry.meta {
            admin.push_meta(&stored.id, language, meta).await?;
        }

        stored_bits.insert(entry.id.clone(), stored.clone());
        println!(
            "  stored {} ({}), hash {}{}",
            stored.id,
            human_bytes(stored.size.unwrap_or(0)),
            short(&stored.hash),
            if entry.meta.is_empty() {
                String::new()
            } else {
                format!(", metadata for {}", entry.meta.len())
            }
        );
    }

    println!("\n{} bits written to {}", planned.len(), ctx.hub_domain);
    Ok(())
}

pub async fn hub_delete(ctx: &Context, bit_ids: &[String], confirmed: bool) -> Result<()> {
    if bit_ids.is_empty() {
        bail!("pass at least one bit id");
    }

    let hub = ctx.hub().await?;
    for bit_id in bit_ids {
        match hub.get_bit(bit_id).await {
            Ok(bit) => println!("{} [{}]  {}", bit.id, bit_type_name(&bit), bit_name(&bit)),
            Err(_) => println!("{bit_id}  (not found in the catalog)"),
        }
    }

    if !confirmed {
        println!(
            "\n{} bits would be deleted from {} — pass --yes to delete",
            bit_ids.len(),
            ctx.hub_domain
        );
        return Ok(());
    }

    let admin = ctx.admin()?;
    for bit_id in bit_ids {
        admin.delete_bit(bit_id).await?;
    }
    println!("\ndeleted {} bits from {}", bit_ids.len(), ctx.hub_domain);
    Ok(())
}

/// Progress frames arrive per chunk. Report a stage change and then only every
/// tenth of the transfer, so a multi-gigabyte mirror stays readable.
fn report_stage(progress: &Value, last: &mut f64) {
    let percent = progress
        .get("percent")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    let stage = progress
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or("progress");

    if percent <= 0.0 {
        let message = progress
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(stage);
        println!("  {stage}: {message}");
        *last = 0.0;
        return;
    }

    if percent < *last + 10.0 && percent < 100.0 {
        return;
    }
    *last = percent;

    let downloaded = progress
        .get("downloaded")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let total = progress
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    println!(
        "  {stage} {percent:>5.1}%  {} / {}",
        human_bytes(downloaded),
        human_bytes(total)
    );
}
