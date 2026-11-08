//! Minimal expiry markers stop old evidence from recreating a fact after its row is removed.
//! Markers contain hashes and a cutoff time, without the fact's text, coordinates or vector.

use std::collections::BTreeMap;

use flow_like_storage::arrow_schema::Schema;
use flow_like_storage::lancedb::{Connection, table::WriteOptions};
use flow_like_types::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::lance::{Table, in_list, int_field, millis, quote, text_field};
use crate::facts::Fact;

const COLUMNS: [&str; 5] = ["id", "content_hash", "geo_hash", "time_hash", "cutoff"];

fn schema() -> Schema {
    Schema::new(vec![
        text_field("id", false),
        text_field("content_hash", false),
        text_field("geo_hash", true),
        text_field("time_hash", true),
        int_field("cutoff", false),
    ])
}

fn canonical(value: &Value) -> String {
    match value {
        Value::Object(fields) => {
            let mut keys: Vec<_> = fields.keys().collect();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| { format!("{}:{}", json!(key), canonical(&fields[key])) })
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

fn digest(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex().to_string()
}

fn context(fact: &Fact) -> (Option<String>, Option<String>) {
    (
        fact.geojson.as_ref().map(|value| digest(&canonical(value))),
        fact.time.map(|time| digest(&millis(time).to_string())),
    )
}

#[derive(Serialize, Deserialize)]
struct ExpiryRow {
    id: String,
    content_hash: String,
    geo_hash: Option<String>,
    time_hash: Option<String>,
    cutoff: i64,
}

pub(crate) struct ExpiryStore {
    table: Table,
}

impl ExpiryStore {
    pub(crate) async fn open(
        connection: Connection,
        name: String,
        write_options: Option<WriteOptions>,
    ) -> Self {
        Self {
            table: Table::open(connection, name, None, write_options).await,
        }
    }

    /// Commit markers before deleting facts, so interruption can only postpone deletion.
    pub(crate) async fn remember(&self, facts: &[Fact]) -> Result<()> {
        let mut pending = BTreeMap::<String, ExpiryRow>::new();
        for fact in facts {
            let Some(expires_at) = fact.expires_at else {
                continue;
            };
            let (geo_hash, time_hash) = context(fact);
            let id = digest(&json!([fact.content_hash, geo_hash, time_hash]).to_string());
            let row = pending.entry(id.clone()).or_insert(ExpiryRow {
                id,
                content_hash: fact.content_hash.clone(),
                geo_hash,
                time_hash,
                cutoff: millis(expires_at),
            });
            row.cutoff = row.cutoff.max(millis(expires_at));
        }
        if pending.is_empty() {
            return Ok(());
        }
        let _writes = self.table.write_lock().await;
        self.table.refresh().await?;
        let ids: Vec<_> = pending.keys().cloned().collect();
        for ids in ids.chunks(256) {
            let previous: Vec<ExpiryRow> = self
                .table
                .all_rows(&in_list("id", ids.iter().map(String::as_str)), &COLUMNS)
                .await?;
            for old in previous {
                if let Some(row) = pending.get_mut(&old.id) {
                    row.cutoff = row.cutoff.max(old.cutoff);
                }
            }
        }
        self.table.create(schema()).await?;
        let rows: Vec<_> = pending.into_values().collect();
        for rows in rows.chunks(256) {
            self.table.upsert(rows).await?;
        }
        Ok(())
    }

    /// Explicit new saves bypass this check. Automatic mining must have newer evidence or a
    /// different explicit place/time before the same statement can become a fact again.
    pub(crate) async fn blocks(&self, fact: &Fact) -> Result<bool> {
        self.table.refresh().await?;
        let rows: Vec<ExpiryRow> = self
            .table
            .all_rows(
                &format!(
                    "content_hash = {} AND cutoff >= {}",
                    quote(&fact.content_hash),
                    millis(fact.observed_at())
                ),
                &COLUMNS,
            )
            .await?;
        let (geo_hash, time_hash) = context(fact);
        let compatible = |left: &Option<String>, right: &Option<String>| {
            left.is_none() || right.is_none() || left == right
        };
        Ok(rows.iter().any(|row| {
            compatible(&row.geo_hash, &geo_hash) && compatible(&row.time_hash, &time_hash)
        }))
    }

    pub(crate) async fn has_markers(&self) -> Result<bool> {
        Ok(self.table.count(None).await? > 0)
    }

    pub(crate) async fn refresh(&self) -> Result<()> {
        self.table.refresh().await
    }
    pub(crate) async fn drop_all(&self) -> Result<()> {
        self.table.drop_table().await
    }
    pub(crate) async fn clear(&self) -> Result<()> {
        self.table.clear().await
    }
    pub(crate) async fn prune(&self) -> Result<()> {
        self.table
            .ensure_indexes(
                &[
                    ("id", "BTREE"),
                    ("content_hash", "BTREE"),
                    ("cutoff", "BTREE"),
                ],
                None,
            )
            .await?;
        self.table.prune().await
    }
}
