use std::{cmp::Ordering, collections::VecDeque};

use arrow_schema::DataType;
use datafusion::{execution::SendableRecordBatchStream, prelude::SessionContext};
use flow_like_storage_contracts::database::{DatabaseDiff, DatabaseDiffRow, DatabaseSelector};
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;

use super::{VectorStore, lancedb::LanceDBVectorStore};
use crate::arrow_utils::record_batch_to_value;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Integer(i128),
    Text(String),
}

fn row_key(row: &Value, column: &str) -> Result<Key> {
    match row.get(column) {
        Some(Value::String(value)) => Ok(Key::Text(value.clone())),
        Some(Value::Number(value)) => value
            .as_i64()
            .map(|v| Key::Integer(v as i128))
            .or_else(|| value.as_u64().map(|v| Key::Integer(v as i128)))
            .ok_or_else(|| anyhow!("Comparison key must be a string or integer")),
        _ => Err(anyhow!(
            "Comparison key '{column}' contains a null or unsupported value"
        )),
    }
}

struct SortedRows {
    stream: SendableRecordBatchStream,
    pending: VecDeque<Value>,
    previous: Option<Key>,
}

impl SortedRows {
    async fn next(&mut self, key_column: &str) -> Result<Option<(Key, Value)>> {
        loop {
            if let Some(row) = self.pending.pop_front() {
                let key = row_key(&row, key_column)?;
                if self
                    .previous
                    .as_ref()
                    .is_some_and(|previous| previous >= &key)
                {
                    return Err(anyhow!(
                        "Comparison key '{key_column}' must be unique and consistently ordered"
                    ));
                }
                self.previous = Some(key.clone());
                return Ok(Some((key, row)));
            }
            match self.stream.try_next().await? {
                Some(batch) => self.pending.extend(record_batch_to_value(&batch)?),
                None => return Ok(None),
            }
        }
    }
}

impl LanceDBVectorStore {
    /// Compare pinned views by a unique application key, keeping only bounded row previews.
    pub async fn compare(&self, other: &Self, key: &str, limit: usize) -> Result<DatabaseDiff> {
        if limit > 1000 {
            return Err(anyhow!("Comparison preview limit cannot exceed 1000"));
        }
        let source_ref = self.reference().await?;
        let target_ref = other.reference().await?;
        let source = self
            .checkout(DatabaseSelector {
                branch: source_ref.branch,
                version: Some(source_ref.version),
                tag: None,
                read_only: true,
            })
            .await?;
        let target = other
            .checkout(DatabaseSelector {
                branch: target_ref.branch,
                version: Some(target_ref.version),
                tag: None,
                read_only: true,
            })
            .await?;
        let source_schema = source.schema().await?;
        let target_schema = target.schema().await?;
        let source_key = source_schema
            .field_with_name(key)
            .map_err(|_| anyhow!("Source has no comparison key '{key}'"))?;
        let target_key = target_schema
            .field_with_name(key)
            .map_err(|_| anyhow!("Target has no comparison key '{key}'"))?;
        if source_key.data_type() != target_key.data_type() {
            return Err(anyhow!("Comparison keys must have the same data type"));
        }
        if !matches!(
            source_key.data_type(),
            DataType::Utf8
                | DataType::LargeUtf8
                | DataType::Utf8View
                | DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64
        ) {
            return Err(anyhow!("Comparison key must be a string or integer column"));
        }
        let mut schema_changes = Vec::new();
        for field in source_schema.fields() {
            match target_schema.field_with_name(field.name()) {
                Err(_) => schema_changes.push(format!("Removed column {}", field.name())),
                Ok(other) if field.as_ref() != other => {
                    schema_changes.push(format!("Changed column {}", field.name()))
                }
                _ => {}
            }
        }
        for field in target_schema.fields() {
            if source_schema.field_with_name(field.name()).is_err() {
                schema_changes.push(format!("Added column {}", field.name()));
            }
        }
        let session = SessionContext::new();
        session.register_table("source_view", source.to_datafusion().await?)?;
        session.register_table("target_view", target.to_datafusion().await?)?;
        let quoted_key = format!("\"{}\"", key.replace('"', "\"\""));
        let mut left = SortedRows {
            stream: session
                .sql(&format!("SELECT * FROM source_view ORDER BY {quoted_key}"))
                .await?
                .execute_stream()
                .await?,
            pending: VecDeque::new(),
            previous: None,
        };
        let mut right = SortedRows {
            stream: session
                .sql(&format!("SELECT * FROM target_view ORDER BY {quoted_key}"))
                .await?
                .execute_stream()
                .await?,
            pending: VecDeque::new(),
            previous: None,
        };
        let mut result = DatabaseDiff {
            source: source.reference().await?,
            target: target.reference().await?,
            added: 0,
            removed: 0,
            changed: 0,
            unchanged: 0,
            schema_changes,
            rows: Vec::new(),
            truncated: false,
        };
        let mut before = left.next(key).await?;
        let mut after = right.next(key).await?;
        while before.is_some() || after.is_some() {
            let order = match (&before, &after) {
                (Some((a, _)), Some((b, _))) => a.cmp(b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => break,
            };
            let row = match order {
                Ordering::Less => {
                    let (_, row) = before.take().expect("source row exists");
                    result.removed += 1;
                    let diff = DatabaseDiffRow {
                        kind: "removed".into(),
                        key: row[key].clone(),
                        before: Some(row),
                        after: None,
                    };
                    before = left.next(key).await?;
                    Some(diff)
                }
                Ordering::Greater => {
                    let (_, row) = after.take().expect("target row exists");
                    result.added += 1;
                    let diff = DatabaseDiffRow {
                        kind: "added".into(),
                        key: row[key].clone(),
                        before: None,
                        after: Some(row),
                    };
                    after = right.next(key).await?;
                    Some(diff)
                }
                Ordering::Equal => {
                    let (_, a) = before.take().expect("source row exists");
                    let (_, b) = after.take().expect("target row exists");
                    let diff = if a == b {
                        result.unchanged += 1;
                        None
                    } else {
                        result.changed += 1;
                        Some(DatabaseDiffRow {
                            kind: "changed".into(),
                            key: a[key].clone(),
                            before: Some(a),
                            after: Some(b),
                        })
                    };
                    before = left.next(key).await?;
                    after = right.next(key).await?;
                    diff
                }
            };
            if let Some(row) = row {
                if result.rows.len() < limit {
                    result.rows.push(row);
                } else {
                    result.truncated = true;
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[tokio::test]
    async fn compares_branch_rows_without_changing_source_and_bounds_preview() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "flow-like-compare-{}",
            flow_like_types::create_id()
        ));
        let mut source = LanceDBVectorStore::new(path.clone(), "samples".into()).await?;
        source
            .insert(vec![
                json!({"id":1,"v":"same"}),
                json!({"id":2,"v":"old"}),
                json!({"id":3,"v":"removed"}),
            ])
            .await?;
        let branch = source.create_branch("candidate").await?;
        branch.delete("id = 3").await?;
        let mut branch = branch;
        branch
            .upsert(
                vec![json!({"id":2,"v":"new"}), json!({"id":4,"v":"added"})],
                "id".into(),
            )
            .await?;
        let diff = source.compare(&branch, "id", 1).await?;
        assert_eq!(
            (diff.added, diff.removed, diff.changed, diff.unchanged),
            (1, 1, 1, 1)
        );
        assert_eq!(diff.rows.len(), 1);
        assert!(diff.truncated);
        assert!(diff.source.pinned && diff.target.pinned);
        assert_eq!(source.reference().await?.branch, "main");
        branch.insert(vec![json!({"id":4,"v":"duplicate"})]).await?;
        assert!(
            source
                .compare(&branch, "id", 10)
                .await
                .unwrap_err()
                .to_string()
                .contains("unique")
        );
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
