//! Segment summaries go to the inbox store, its one source of truth; the engine's own table is
//! only the search index over them. Publishing is idempotent: a summary version is written under
//! a stable client id, so a repeat returns the first write.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono_tz::Tz;
use flow_like_types::Result;

use crate::model::Seq;
use crate::summary::SegmentSummary;

const MAX_TITLE_CHARS: usize = 200;
const MAX_SUMMARY_BYTES: usize = 32 * 1024;

/// A summary as the inbox store takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedSegment {
    pub id: String,
    pub level: u8,
    pub seq_from: Seq,
    pub seq_to: Seq,
    pub title: Option<String>,
    pub summary: String,
    pub parent_id: Option<String>,
}

/// Where a bot's host writes the summaries of its conversation, as the bot.
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait SegmentSink: Send + Sync {
    /// Client ids the bot has written already.
    async fn published(&self) -> Result<HashSet<String>>;
    async fn write(&self, segment: &PublishedSegment) -> Result<()>;
}

/// `s0-1-28`, then `s0-1-28.v2` for a rewrite, `.m` once a higher-level summary merged it: a
/// version that changed is a new write under a new id, which is how the stores version a range.
pub fn client_id(summary: &SegmentSummary) -> String {
    let mut id = summary.id.clone();
    if summary.version > 1 {
        id.push_str(&format!(".v{}", summary.version));
    }
    if summary.parent.is_some() {
        id.push_str(".m");
    }
    id
}

fn title_of(summary: &SegmentSummary) -> Option<String> {
    let title = summary.body.title.trim();
    (!title.is_empty()).then(|| title.chars().take(MAX_TITLE_CHARS).collect())
}

fn text_of(summary: &SegmentSummary, tz: Tz) -> String {
    let mut text = summary.render(tz);
    if text.len() > MAX_SUMMARY_BYTES {
        let mut end = MAX_SUMMARY_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// The summaries not yet written, highest level first so a parent exists before the summaries it
/// merged. `known` gains what was written; a failed write stays out and is tried again next time.
/// Returns how many were written and the first error.
pub async fn publish_summaries(
    summaries: &[SegmentSummary],
    tz: Tz,
    sink: &dyn SegmentSink,
    known: &mut HashSet<String>,
) -> (usize, Option<flow_like_types::Error>) {
    let ids: HashMap<&str, String> = summaries
        .iter()
        .map(|summary| (summary.id.as_str(), client_id(summary)))
        .collect();
    let mut ordered: Vec<&SegmentSummary> = summaries.iter().collect();
    ordered.sort_by_key(|summary| (std::cmp::Reverse(summary.level), summary.span.from));
    let mut written = 0;
    let mut first_error = None;
    for summary in ordered {
        let id = client_id(summary);
        if known.contains(&id) {
            continue;
        }
        let parent_id = match &summary.parent {
            Some(parent) => match ids.get(parent.as_str()) {
                Some(parent_id) if known.contains(parent_id) => Some(parent_id.clone()),
                _ => continue,
            },
            None => None,
        };
        let text = text_of(summary, tz);
        if text.trim().is_empty() {
            continue;
        }
        let segment = PublishedSegment {
            id: id.clone(),
            level: summary.level,
            seq_from: summary.span.from,
            seq_to: summary.span.to,
            title: title_of(summary),
            summary: text,
            parent_id,
        };
        match sink.write(&segment).await {
            Ok(()) => {
                known.insert(id);
                written += 1;
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    (written, first_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SeqSpan;
    use crate::summary::SummaryBody;
    use chrono::Utc;
    use std::sync::Mutex;

    fn summary(
        level: u8,
        from: Seq,
        to: Seq,
        version: u32,
        parent: Option<&str>,
    ) -> SegmentSummary {
        SegmentSummary {
            id: SegmentSummary::id_for(level, SeqSpan::new(from, to)),
            level,
            span: SeqSpan::new(from, to),
            t_from: Utc::now(),
            t_to: Utc::now(),
            body: SummaryBody {
                title: "Planning".into(),
                sections: Default::default(),
            },
            children: Vec::new(),
            child_spans: Vec::new(),
            user_seqs: Vec::new(),
            missing: Vec::new(),
            parent: parent.map(str::to_string),
            version,
            prompt_version: 1,
            model: None,
            created_at: Utc::now(),
        }
    }

    #[derive(Default)]
    struct Recording {
        written: Mutex<Vec<PublishedSegment>>,
        refuse: Mutex<HashSet<String>>,
    }

    #[async_trait]
    impl SegmentSink for Recording {
        async fn published(&self) -> Result<HashSet<String>> {
            Ok(HashSet::new())
        }

        async fn write(&self, segment: &PublishedSegment) -> Result<()> {
            if self.refuse.lock().unwrap().contains(&segment.id) {
                return Err(flow_like_types::anyhow!("refused"));
            }
            self.written.lock().unwrap().push(segment.clone());
            Ok(())
        }
    }

    #[test]
    fn client_ids_name_the_version_and_the_merge() {
        assert_eq!(client_id(&summary(0, 1, 28, 1, None)), "s0-1-28");
        assert_eq!(client_id(&summary(0, 1, 28, 2, None)), "s0-1-28.v2");
        assert_eq!(
            client_id(&summary(0, 1, 28, 1, Some("s1-1-60"))),
            "s0-1-28.m"
        );
        assert_eq!(
            client_id(&summary(0, 1, 28, 3, Some("s1-1-60"))),
            "s0-1-28.v3.m"
        );
    }

    #[tokio::test]
    async fn parents_are_written_before_the_summaries_they_merged() {
        let sink = Recording::default();
        let summaries = [
            summary(0, 1, 28, 1, Some("s1-1-60")),
            summary(0, 29, 60, 1, Some("s1-1-60")),
            summary(1, 1, 60, 1, None),
        ];
        let mut known = HashSet::new();
        let (written, error) = publish_summaries(&summaries, Tz::UTC, &sink, &mut known).await;
        assert_eq!((written, error.is_none()), (3, true));
        let order: Vec<_> = sink
            .written
            .lock()
            .unwrap()
            .iter()
            .map(|segment| (segment.id.clone(), segment.parent_id.clone()))
            .collect();
        assert_eq!(
            order,
            [
                ("s1-1-60".to_string(), None),
                ("s0-1-28.m".to_string(), Some("s1-1-60".to_string())),
                ("s0-29-60.m".to_string(), Some("s1-1-60".to_string())),
            ]
        );
        let (again, _) = publish_summaries(&summaries, Tz::UTC, &sink, &mut known).await;
        assert_eq!(again, 0, "known ids are not written twice");
    }

    #[tokio::test]
    async fn a_refused_write_is_retried_and_holds_back_its_children() {
        let sink = Recording::default();
        sink.refuse.lock().unwrap().insert("s1-1-60".into());
        let summaries = [
            summary(0, 1, 28, 1, Some("s1-1-60")),
            summary(1, 1, 60, 1, None),
        ];
        let mut known = HashSet::new();
        let (written, error) = publish_summaries(&summaries, Tz::UTC, &sink, &mut known).await;
        assert_eq!((written, error.is_some()), (0, true));
        assert!(known.is_empty(), "a failed write is not remembered");
        sink.refuse.lock().unwrap().clear();
        let (written, _) = publish_summaries(&summaries, Tz::UTC, &sink, &mut known).await;
        assert_eq!(written, 2);
    }
}
