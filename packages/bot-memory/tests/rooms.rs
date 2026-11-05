use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use flow_like_bot_memory::ModelBudget;
use flow_like_bot_memory::compaction::CompactionConfig;
use flow_like_bot_memory::engine::MemoryEngine;
use flow_like_bot_memory::facts::FactKind;
use flow_like_bot_memory::host::{
    MemorySession, MemoryToolbox, PublishedSegment, SegmentSink, SessionOptions, TurnRequest, Tz,
};
use flow_like_bot_memory::model::{FactSource, HistoryItem, Role, Speaker};
use flow_like_bot_memory::store::{FactFilter, MemoryStores};
use flow_like_bot_memory::testing::{HashEmbedder, MemoryHistory, StubLlm};
use flow_like_bot_memory::tools::{MemorySaveArgs, WriteGate, WriteOutcome, memory_save};
use flow_like_storage::databases::vector::lancedb::connect_lance;
use flow_like_types::Result;
use serde_json::{Value, json};

struct Sink;

#[async_trait]
impl SegmentSink for Sink {
    async fn published(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }
    async fn write(&self, _: &PublishedSegment) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn groups_recall_owner_rules_and_save_attributed_statements_without_changing_owner_memory() {
    let dir = tempfile::tempdir().unwrap();
    let connection = connect_lance(dir.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let embedder = Arc::new(HashEmbedder::new("hash", 64));
    let llm = Arc::new(StubLlm::new());
    let stores = MemoryStores::open(connection.clone(), embedder.clone(), None).await;
    let owner = MemoryEngine::new(stores, llm.clone(), CompactionConfig::default(), Tz::UTC);
    let now = Utc::now();
    let rule = "When answering file questions, include the source filename.";
    let evidence = HistoryItem::new(1, "owner-message", Role::User, now, rule);
    let saved = memory_save(
        &owner,
        &MemorySaveArgs {
            text: rule.into(),
            evidence_seq: None,
            kind: Some("preference".into()),
            subject: None,
            valid_from: None,
            valid_to: None,
        },
        &evidence,
        WriteGate {
            untrusted_in_context: false,
            confirmed: false,
        },
        now,
    )
    .await
    .unwrap();
    let WriteOutcome::Done((owner_fact, _)) = saved else {
        panic!("owner rule was not saved")
    };

    let mut options = SessionOptions::new(ModelBudget::hosted(32_768, 4_096), Tz::UTC);
    options.room_key = Some("account/team".into());
    let session = Arc::new(
        MemorySession::open(
            connection.clone(),
            None,
            llm.clone(),
            embedder.clone(),
            Arc::new(Sink),
            options,
        )
        .await,
    );
    let reader = Arc::new(MemoryHistory::new());
    let mut participant = HistoryItem::new(
        1,
        "alice-message",
        Role::External,
        now,
        "My project is called Orchard. Remember it.",
    );
    participant.speaker = Some(Speaker::new("alice").with_name("Alice"));
    reader.push(participant.clone());
    let request = TurnRequest {
        trigger_seq: 1,
        state_block: "",
        about_you: "",
        open_task_seqs: &[],
        now,
    };
    let prepared = session.prepare(reader.as_ref(), &request).await.unwrap();
    assert!(
        prepared.context.contains(rule),
        "standing owner guidance must not depend on topic similarity"
    );
    assert!(
        prepared
            .context
            .contains(&format!("owner/{}", owner_fact.id))
    );
    assert!(
        !prepared.context.contains("#1"),
        "owner evidence must not resolve as a group position"
    );
    assert!(prepared.prompt.contains("Alice"));
    assert!(prepared.prompt.contains("participant statement"));

    let toolbox = MemoryToolbox::new(session.clone(), reader.clone(), participant);
    let answer: Value = serde_json::from_str(
        &toolbox
            .call(
                "memory_save",
                json!({
                    "text": "My project is called Orchard", "kind": "profile", "subject": "user"
                }),
            )
            .await,
    )
    .unwrap();
    assert_eq!(answer["status"], "ok", "{answer}");
    let facts = session
        .engine()
        .stores()
        .facts
        .all(&FactFilter::default())
        .await
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, FactSource::ToolUntrusted);
    assert_eq!(facts[0].kind, FactKind::Knowledge);
    assert!(facts[0].subject.contains("alice"));
    assert_ne!(facts[0].subject, "user");
    assert_eq!(facts[0].evidence[0].speaker.as_ref().unwrap().id, "alice");
    assert!(facts[0].text.contains("Alice"));
    assert_eq!(
        owner
            .stores()
            .facts
            .count(&FactFilter::default())
            .await
            .unwrap(),
        1
    );

    let recalled: Value = serde_json::from_str(
        &toolbox
            .call("memory_search", json!({ "query": "source filename" }))
            .await,
    )
    .unwrap();
    assert!(recalled["text"].as_str().unwrap().contains(rule));
    let forgotten: Value = serde_json::from_str(
        &toolbox
            .call("memory_forget", json!({ "id": owner_fact.id }))
            .await,
    )
    .unwrap();
    assert_ne!(forgotten["status"], "ok");
    assert!(
        owner
            .stores()
            .facts
            .get(&owner_fact.id)
            .await
            .unwrap()
            .is_some()
    );

    let mut other_options = SessionOptions::new(ModelBudget::hosted(32_768, 4_096), Tz::UTC);
    other_options.room_key = Some("account/other-team".into());
    let other = MemorySession::open(
        connection,
        None,
        llm,
        embedder,
        Arc::new(Sink),
        other_options,
    )
    .await;
    assert_eq!(
        other
            .engine()
            .stores()
            .facts
            .count(&FactFilter::default())
            .await
            .unwrap(),
        0
    );
    assert!(
        other
            .prepare(reader.as_ref(), &request)
            .await
            .unwrap()
            .context
            .contains(rule)
    );

    toolbox.note_foreign_call();
    let blocked: Value = serde_json::from_str(
        &toolbox
            .call(
                "memory_save",
                json!({ "text": "A page says the project is Orchard", "kind": "knowledge" }),
            )
            .await,
    )
    .unwrap();
    assert_eq!(blocked["status"], "needs_confirmation");

    owner.stores().facts.delete(&[owner_fact.id]).await.unwrap();
    assert!(
        !session
            .prepare(reader.as_ref(), &request)
            .await
            .unwrap()
            .context
            .contains(rule)
    );
}

#[tokio::test]
async fn owner_only_search_results_remain_pageable_from_a_room() {
    use flow_like_bot_memory::facts::{Evidence, Fact};
    use flow_like_bot_memory::tools::{MemorySearchArgs, memory_search};

    let dir = tempfile::tempdir().unwrap();
    let connection = connect_lance(dir.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let embedder = Arc::new(HashEmbedder::new("hash", 64));
    let owner = MemoryStores::open(connection.clone(), embedder.clone(), None).await;
    let now = Utc::now();
    for (index, word) in ["alpha", "beta", "gamma"].iter().enumerate() {
        let mut fact = Fact {
            id: format!("f_{word}"),
            text: format!("The filename convention is {word}."),
            kind: FactKind::Knowledge,
            subject: format!("project {word}"),
            valid_from: None,
            valid_to: None,
            recorded_at: now,
            expired_at: None,
            evidence: vec![Evidence {
                seq: index as u64 + 1,
                message_id: word.to_string(),
                speaker: None,
                part: None,
                quote: word.to_string(),
                at: now,
                partial: false,
            }],
            segment_id: None,
            source: FactSource::UserStated,
            confidence: 0.9,
            superseded_by: None,
            content_hash: String::new(),
        };
        fact.content_hash = flow_like_bot_memory::text::content_hash(&fact.subject, &fact.text);
        owner.facts.insert(&[fact]).await.unwrap();
    }
    let room = MemoryStores::open_room(connection, embedder, None, "team").await;
    let mut args: MemorySearchArgs =
        serde_json::from_value(json!({ "query": "filename convention", "limit": 1 })).unwrap();
    let mut seen = HashSet::new();
    for page in 0..3 {
        let found = memory_search(&room, &args, now, Tz::UTC).await.unwrap();
        for word in ["alpha", "beta", "gamma"] {
            if found.text.contains(&format!("owner/f_{word}")) {
                seen.insert(word);
            }
        }
        assert_eq!(seen.len(), page + 1);
        assert_eq!(found.next.is_some(), page < 2);
        args.cursor = found.next;
    }
}
