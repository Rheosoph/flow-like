use std::collections::{BTreeMap, BTreeSet};

use flow_like_types::Result;

use crate::model::{Seq, SeqSpan};
use crate::store::{PreparedSegments, SegmentStore};
use crate::summary::SegmentSummary;

/// Summary edits stay private until the engine validates the revision and claims the commit.
pub(super) struct SegmentDraft {
    original: BTreeMap<String, SegmentSummary>,
    rows: BTreeMap<String, SegmentSummary>,
}

pub(super) struct SegmentChanges {
    rows: PreparedSegments,
    deleted: Vec<String>,
    parents: BTreeMap<Option<String>, Vec<String>>,
}

impl SegmentChanges {
    pub(super) async fn apply(self, store: &SegmentStore) -> Result<()> {
        store.delete(&self.deleted).await?;
        store.put_prepared(self.rows).await?;
        for (parent, ids) in self.parents {
            store.set_parent(&ids, parent.as_deref()).await?;
        }
        Ok(())
    }
}

impl SegmentDraft {
    pub(super) async fn load(store: &SegmentStore) -> Result<Self> {
        let rows: BTreeMap<_, _> = store
            .all()
            .await?
            .into_iter()
            .map(|summary| (summary.id.clone(), summary))
            .collect();
        Ok(Self {
            original: rows.clone(),
            rows,
        })
    }

    pub(super) fn get(&self, id: &str) -> Option<&SegmentSummary> {
        self.rows.get(id)
    }

    fn sorted(&self, predicate: impl Fn(&SegmentSummary) -> bool) -> Vec<SegmentSummary> {
        let mut rows: Vec<_> = self
            .rows
            .values()
            .filter(|row| predicate(row))
            .cloned()
            .collect();
        rows.sort_by_key(|row| (row.span.from, row.level, row.span.to));
        rows
    }

    pub(super) fn active(&self) -> Vec<SegmentSummary> {
        self.sorted(|row| row.parent.is_none())
    }

    pub(super) fn children(&self, id: &str) -> Vec<SegmentSummary> {
        self.sorted(|row| row.parent.as_deref() == Some(id))
    }

    pub(super) fn covering(&self, deleted: &BTreeSet<Seq>) -> Vec<SegmentSummary> {
        let mut rows = self.sorted(|row| deleted.iter().any(|seq| row.span.contains(*seq)));
        rows.sort_by_key(|row| row.level);
        rows
    }

    pub(super) fn put(&mut self, summary: SegmentSummary) {
        self.rows.insert(summary.id.clone(), summary);
    }

    pub(super) fn delete(&mut self, id: &str) {
        self.rows.remove(id);
    }

    pub(super) fn set_parent(&mut self, ids: &[String], parent: Option<&str>) {
        for id in ids {
            if let Some(row) = self.rows.get_mut(id) {
                row.parent = parent.map(str::to_owned);
            }
        }
    }

    pub(super) fn drop_leftovers(&mut self, through: Option<Seq>, reuse: &[SeqSpan]) {
        loop {
            let leftovers: Vec<_> = self
                .active()
                .into_iter()
                .filter(|summary| {
                    through.is_none_or(|through| summary.span.to > through)
                        && !(summary.level == 0 && reuse.contains(&summary.span))
                })
                .collect();
            if leftovers.is_empty() {
                break;
            }
            for summary in leftovers {
                self.set_parent(&summary.children, None);
                self.delete(&summary.id);
            }
        }
    }

    pub(super) async fn prepare(self, store: &SegmentStore) -> Result<SegmentChanges> {
        let deleted = self
            .original
            .keys()
            .filter(|id| !self.rows.contains_key(*id))
            .cloned()
            .collect();
        let mut changed = Vec::new();
        let mut parents: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for row in self.rows.into_values() {
            if let Some(original) = self.original.get(&row.id) {
                if original == &row {
                    continue;
                }
                let mut reparented = original.clone();
                reparented.parent = row.parent.clone();
                if reparented == row {
                    parents.entry(row.parent).or_default().push(row.id);
                    continue;
                }
            }
            changed.push(row);
        }
        Ok(SegmentChanges {
            rows: store.prepare_put(&changed).await?,
            deleted,
            parents,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::llm::Embedder;
    use crate::store::MemoryStores;
    use crate::summary::{SUMMARY_PROMPT_VERSION, SummaryBody};
    use crate::testing::HashEmbedder;

    #[derive(Default)]
    struct WatchedEmbedder {
        calls: AtomicUsize,
        unavailable: AtomicBool,
    }

    #[async_trait::async_trait]
    impl Embedder for WatchedEmbedder {
        fn model_id(&self) -> &str {
            "watched"
        }

        async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.unavailable.load(Ordering::SeqCst) {
                flow_like_types::bail!("embedder unavailable");
            }
            HashEmbedder::new(self.model_id(), 16)
                .embed_documents(texts)
                .await
        }

        async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
            HashEmbedder::new(self.model_id(), 16)
                .embed_query(text)
                .await
        }
    }

    #[tokio::test]
    async fn parent_only_changes_preserve_vectors_without_embedding() {
        let directory = tempfile::tempdir().unwrap();
        let embedder = Arc::new(WatchedEmbedder::default());
        let stores = MemoryStores::open_local(directory.path(), embedder.clone())
            .await
            .unwrap();
        let now = chrono::Utc::now();
        let span = SeqSpan::new(1, 2);
        let summary = SegmentSummary {
            id: SegmentSummary::id_for(0, span),
            level: 0,
            span,
            t_from: now,
            t_to: now,
            body: SummaryBody {
                title: "The release plan".to_owned(),
                ..Default::default()
            },
            children: Vec::new(),
            child_spans: Vec::new(),
            user_seqs: Vec::new(),
            missing: Vec::new(),
            parent: None,
            version: 1,
            prompt_version: SUMMARY_PROMPT_VERSION,
            model: None,
            created_at: now,
        };
        stores
            .segments
            .put(std::slice::from_ref(&summary))
            .await
            .unwrap();
        assert_eq!(embedder.calls.load(Ordering::SeqCst), 1);
        embedder.unavailable.store(true, Ordering::SeqCst);

        for parent in [Some("s1-1-4"), None] {
            let mut draft = SegmentDraft::load(&stores.segments).await.unwrap();
            let before = draft.get(&summary.id).unwrap().clone();
            draft.set_parent(std::slice::from_ref(&summary.id), parent);
            let changes = draft.prepare(&stores.segments).await.unwrap();
            changes.apply(&stores.segments).await.unwrap();
            assert_eq!(embedder.calls.load(Ordering::SeqCst), 1);
            let expected = SegmentSummary {
                parent: parent.map(str::to_owned),
                ..before
            };
            assert_eq!(
                stores.segments.get(&summary.id).await.unwrap(),
                Some(expected)
            );
            assert_eq!(stores.segments.reembed().await.unwrap(), 0);
            assert_eq!(embedder.calls.load(Ordering::SeqCst), 1);
        }
    }
}
