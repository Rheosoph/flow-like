//! The memory tab's calls: list a bot's facts with their evidence, forget one, reset.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use flow_like_bot_memory::facts::{Evidence, Fact, FactKind};
use flow_like_bot_memory::host::FactAdmin;
use flow_like_bot_memory::model::{FactSource, Seq};
use flow_like_bot_memory::store::MemoryStores;
use flow_like_bot_memory::testing::HashEmbedder;
use flow_like_bot_memory::text::content_hash;
use tempfile::TempDir;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
        .unwrap()
        .into()
}

fn fact(id: &str, text: &str, seq: Seq) -> Fact {
    Fact {
        id: id.into(),
        text: text.into(),
        kind: FactKind::Profile,
        subject: "user".into(),
        valid_from: None,
        valid_to: None,
        recorded_at: now(),
        expired_at: None,
        evidence: vec![Evidence {
            seq,
            message_id: format!("msg_{seq}"),
            part: None,
            quote: text.into(),
            at: now(),
            partial: false,
        }],
        segment_id: None,
        source: FactSource::UserStated,
        confidence: 0.8,
        superseded_by: None,
        content_hash: content_hash("user", text),
    }
}

async fn stores(dir: &TempDir) -> MemoryStores {
    MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new("hash", 16)))
        .await
        .unwrap()
}

#[tokio::test]
async fn lists_facts_with_their_evidence_and_forgets_one_by_prefix() {
    let dir = TempDir::new().unwrap();
    let seeded = stores(&dir).await;
    seeded
        .facts
        .insert(&[
            fact("f_tea", "Prefers green tea", 12),
            fact("f_city", "Lives in Hamburg", 40),
        ])
        .await
        .unwrap();
    let admin = FactAdmin::over(stores(&dir).await);

    let listed = admin.list(false).await.unwrap();
    assert_eq!(listed.len(), 2);
    let tea = listed.iter().find(|fact| fact.id == "f_tea").unwrap();
    assert_eq!(tea.evidence[0].seq, 12);
    assert_eq!(tea.evidence[0].quote, "Prefers green tea");

    let forgotten = admin.forget("f_te").await.unwrap();
    assert_eq!(forgotten.id, "f_tea");
    let left = admin.list(false).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, "f_city");
}

#[tokio::test]
async fn forgetting_an_unknown_fact_is_an_error_and_reset_empties_the_memory() {
    let dir = TempDir::new().unwrap();
    stores(&dir)
        .await
        .facts
        .insert(&[fact("f_tea", "Prefers green tea", 12)])
        .await
        .unwrap();
    let admin = FactAdmin::over(stores(&dir).await);

    assert!(admin.forget("f_nope").await.is_err());
    assert_eq!(admin.list(false).await.unwrap().len(), 1);

    admin.reset().await.unwrap();
    assert!(admin.list(false).await.unwrap().is_empty());
}
