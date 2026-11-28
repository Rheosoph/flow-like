//! Entities with facts as edges: one entity for every name the conversation used, updates
//! that compare facts across those names, recall and `about` searches that follow entities,
//! the maintenance backfill and the owner's merge and forget.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use flow_like_bot_memory::compaction::CompactionConfig;
use flow_like_bot_memory::engine::MemoryEngine;
use flow_like_bot_memory::entities::{EntityKind, OWNER_ENTITY};
use flow_like_bot_memory::facts::{Evidence, Fact, FactKind, integrate_facts};
use flow_like_bot_memory::llm::{LlmRequest, LlmTask};
use flow_like_bot_memory::model::FactSource;
use flow_like_bot_memory::prompt::ModelBudget;
use flow_like_bot_memory::store::{FactFilter, MemoryStores};
use flow_like_bot_memory::testing::{HashEmbedder, SpanLine, StubLlm, Transcript, span_lines};
use flow_like_bot_memory::tools::{
    MemoryForgetArgs, MemorySaveArgs, MemorySearchArgs, WriteGate, WriteOutcome, memory_forget,
    memory_save, memory_search,
};
use flow_like_types::Result;
use regex::Regex;
use serde_json::{Value, json};
use tempfile::TempDir;

fn start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-01T09:00:00Z")
        .unwrap()
        .into()
}

async fn engine(dir: &TempDir, llm: StubLlm) -> MemoryEngine {
    let stores = MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new("hash", 16)))
        .await
        .unwrap();
    MemoryEngine::new(stores, Arc::new(llm), CompactionConfig::default(), Tz::UTC)
}

fn fact_answer(seq: u64, quote: &str, text: &str, subject: &str, object: &str) -> Value {
    json!({
        "text": text,
        "kind": "knowledge",
        "subject": subject,
        "object": object,
        "relation": "lives in",
        "valid_from": null,
        "valid_to": null,
        "time": null,
        "ttl_seconds": null,
        "geojson": null,
        "evidence": [{ "pointer": format!("#{seq}"), "quote": quote }],
        "source": "user_stated",
        "confidence": 0.9
    })
}

/// The extraction a model would give for the two sister messages.
fn extraction(request: &LlmRequest) -> Result<String> {
    let mut facts = Vec::new();
    let mut entities = Vec::new();
    for line in span_lines(&request.prompt, "Messages to extract from:") {
        let SpanLine::Message { seq, role, text } = line else {
            continue;
        };
        if role != "user" {
            continue;
        }
        if text.contains("My sister lives in Paris") {
            facts.push(fact_answer(
                seq,
                "My sister lives in Paris",
                "The user's sister lives in Paris.",
                "user.sister",
                "paris",
            ));
            entities.push(json!({ "name": "Paris", "kind": "place", "aliases": [] }));
        }
        if text.contains("Mia moved to Lyon") {
            facts.push(fact_answer(
                seq,
                "Mia moved to Lyon",
                "Mia lives in Lyon.",
                "mia",
                "lyon",
            ));
            entities
                .push(json!({ "name": "Mia", "kind": "person", "aliases": ["the user's sister"] }));
            entities.push(json!({ "name": "Lyon", "kind": "place", "aliases": [] }));
        }
    }
    Ok(json!({ "entities": entities, "facts": facts }).to_string())
}

/// Supersedes the Paris fact when the Lyon fact is compared with it.
fn update(request: &LlmRequest) -> Result<String> {
    let id = Regex::new(r"\[(f_[a-z0-9]+)\] The user's sister lives in Paris")
        .unwrap()
        .captures(&request.prompt)
        .map(|caps| caps[1].to_string());
    Ok(match id {
        Some(id) if request.prompt.contains("Lyon") => {
            json!({ "action": "supersede", "target_id": id })
        }
        _ => json!({ "action": "add", "target_id": null }),
    }
    .to_string())
}

#[tokio::test]
async fn every_name_of_one_person_is_one_entity_and_updates_compare_across_names() {
    let dir = tempfile::tempdir().unwrap();
    let llm = StubLlm::new()
        .with(LlmTask::FactExtraction, extraction)
        .with(LlmTask::FactUpdate, update);
    let engine = engine(&dir, llm).await;
    let mut transcript = Transcript::new(start());
    transcript.user("My sister lives in Paris.");
    let first = transcript.bot("Lovely city.");
    engine
        .mine_idle(&transcript.history, first, transcript.now())
        .await
        .unwrap();
    transcript.advance(Duration::days(3));
    transcript.user("My sister Mia moved to Lyon last week.");
    let second = transcript.bot("Noted.");
    engine
        .mine_idle(&transcript.history, second, transcript.now())
        .await
        .unwrap();

    let stores = engine.stores();
    let entities = stores.entities.all().await.unwrap();
    let mia = entities
        .iter()
        .find(|entity| entity.name == "Mia")
        .unwrap_or_else(|| panic!("no Mia: {entities:?}"));
    assert_eq!(mia.kind, EntityKind::Person);
    for alias in ["mia", "user.sister"] {
        assert!(mia.aliases.contains(&alias.to_string()), "{mia:?}");
    }
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::Person)
            .count(),
        1,
        "{entities:?}"
    );

    let facts = stores
        .facts
        .all(&FactFilter {
            include_superseded: true,
            ..FactFilter::default()
        })
        .await
        .unwrap();
    let paris = facts
        .iter()
        .find(|fact| fact.text.contains("Paris"))
        .unwrap();
    let lyon = facts
        .iter()
        .find(|fact| fact.text.contains("Lyon"))
        .unwrap();
    assert_eq!(paris.subject_id.as_deref(), Some(mia.id.as_str()));
    assert_eq!(lyon.subject_id.as_deref(), Some(mia.id.as_str()));
    assert_eq!(lyon.relation.as_deref(), Some("lives in"));
    assert_eq!(paris.superseded_by.as_deref(), Some(lyon.id.as_str()));
    let place = |id: &Option<String>| {
        entities
            .iter()
            .find(|entity| Some(&entity.id) == id.as_ref())
            .map(|entity| (entity.name.clone(), entity.kind))
    };
    assert_eq!(
        place(&lyon.object_id),
        Some(("Lyon".into(), EntityKind::Place))
    );
}

#[tokio::test]
async fn recall_and_about_searches_follow_the_entity_a_message_names() {
    let dir = tempfile::tempdir().unwrap();
    let llm = StubLlm::new()
        .with(LlmTask::FactExtraction, extraction)
        .with(LlmTask::FactUpdate, update);
    let engine = engine(&dir, llm).await;
    let mut transcript = Transcript::new(start());
    transcript.user("My sister Mia moved to Lyon last week.");
    let latest = transcript.bot("Noted.");
    engine
        .mine_idle(&transcript.history, latest, transcript.now())
        .await
        .unwrap();
    let now = transcript.now();
    for i in 0..12 {
        save(
            &engine,
            &mut transcript,
            &format!("The user's garden bed {i} has kale."),
            "user",
            None,
        )
        .await;
    }

    let budget = ModelBudget {
        facts_count: 2,
        ..ModelBudget::hosted(128_000, 8_000)
    };
    let recalled = engine
        .recall("Should I visit my sister soon?", &budget, now)
        .await
        .unwrap();
    assert!(
        recalled.iter().any(|fact| fact.text.contains("Lyon")),
        "{recalled:?}"
    );

    let about = memory_search(
        engine.stores(),
        &MemorySearchArgs {
            about: Some("my sister".into()),
            ..MemorySearchArgs::default()
        },
        now,
        Tz::UTC,
    )
    .await
    .unwrap();
    assert!(about.text.contains("Mia (person"), "{}", about.text);
    assert!(about.text.contains("Mia lives in Lyon."), "{}", about.text);
    assert!(about.text.contains("Connected"), "{}", about.text);
    assert!(about.text.contains("Lyon (place)"), "{}", about.text);
    assert!(!about.text.contains("kale"), "{}", about.text);

    let missing = memory_search(
        engine.stores(),
        &MemorySearchArgs {
            about: Some("Bertrand".into()),
            ..MemorySearchArgs::default()
        },
        now,
        Tz::UTC,
    )
    .await
    .unwrap();
    assert!(
        missing.text.starts_with("Nothing remembered"),
        "{}",
        missing.text
    );
}

async fn save(
    engine: &MemoryEngine,
    transcript: &mut Transcript,
    text: &str,
    subject: &str,
    object: Option<&str>,
) -> Fact {
    let seq = transcript.user(text);
    let evidence = transcript.history.get(seq).unwrap();
    let outcome = memory_save(
        engine,
        &MemorySaveArgs {
            text: text.to_string(),
            subject: Some(subject.to_string()),
            object: object.map(str::to_string),
            relation: object.map(|_| "works at".to_string()),
            ..MemorySaveArgs::default()
        },
        &evidence,
        WriteGate::default(),
        transcript.now(),
    )
    .await
    .unwrap();
    match outcome {
        WriteOutcome::Done((fact, _)) => fact,
        other => panic!("not saved: {other:?}"),
    }
}

#[tokio::test]
async fn saved_facts_link_their_subject_and_object() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, StubLlm::new()).await;
    let mut transcript = Transcript::new(start());
    let anna = save(
        &engine,
        &mut transcript,
        "Anna works at Acme.",
        "Anna",
        Some("Acme"),
    )
    .await;
    let own = save(
        &engine,
        &mut transcript,
        "The user likes green tea.",
        "user",
        None,
    )
    .await;
    assert_eq!(own.subject_id.as_deref(), Some(OWNER_ENTITY));
    let entities = engine.stores().entities.all().await.unwrap();
    let name = |id: &Option<String>| {
        entities
            .iter()
            .find(|entity| Some(&entity.id) == id.as_ref())
            .map(|entity| entity.name.clone())
    };
    assert_eq!(name(&anna.subject_id).as_deref(), Some("anna"));
    assert_eq!(name(&anna.object_id).as_deref(), Some("acme"));
    assert_eq!(anna.relation.as_deref(), Some("works at"));
}

#[tokio::test]
async fn maintenance_gives_old_facts_entities_and_drops_unused_ones() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, StubLlm::new()).await;
    let now = start();
    let old = |id: &str, text: &str, subject: &str| {
        let mut fact = Fact {
            id: id.into(),
            text: text.into(),
            kind: FactKind::Knowledge,
            subject: subject.into(),
            subject_id: None,
            object: None,
            object_id: None,
            relation: None,
            geojson: None,
            time: None,
            expires_at: None,
            valid_from: None,
            valid_to: None,
            recorded_at: now,
            expired_at: None,
            evidence: vec![Evidence {
                seq: 1,
                message_id: "m1".into(),
                speaker: None,
                part: None,
                quote: text.into(),
                at: now,
                partial: false,
            }],
            segment_id: None,
            source: FactSource::UserStated,
            confidence: 0.9,
            superseded_by: None,
            content_hash: String::new(),
        };
        fact.content_hash = flow_like_bot_memory::text::content_hash(subject, text);
        fact
    };
    integrate_facts(
        &engine.stores().facts,
        &StubLlm::new(),
        vec![
            old("f_one", "The user drinks tea.", "user"),
            old("f_two", "Project Atlas ships in May.", "project atlas"),
            old("f_three", "Project Atlas uses Rust.", "project atlas"),
        ],
        Tz::UTC,
        now,
    )
    .await
    .unwrap();

    let mut transcript = Transcript::new(now);
    let stray = save(&engine, &mut transcript, "Bob likes chess.", "bob", None).await;
    memory_forget(
        &engine,
        &MemoryForgetArgs { id: stray.id },
        WriteGate::default(),
        transcript.now(),
    )
    .await
    .unwrap();

    engine.optimize(transcript.now()).await.unwrap();
    let facts = engine
        .stores()
        .facts
        .all(&FactFilter::default())
        .await
        .unwrap();
    assert!(
        facts.iter().all(|fact| fact.subject_id.is_some()),
        "{facts:?}"
    );
    let atlas: Vec<_> = facts
        .iter()
        .filter(|fact| fact.subject == "project atlas")
        .map(|fact| fact.subject_id.clone())
        .collect();
    assert_eq!(atlas.len(), 2);
    assert_eq!(atlas[0], atlas[1]);
    let entities = engine.stores().entities.all().await.unwrap();
    assert!(entities.iter().any(|entity| entity.id == OWNER_ENTITY));
    assert!(
        !entities.iter().any(|entity| entity.name == "bob"),
        "an entity without facts is dropped: {entities:?}"
    );
}

#[tokio::test]
async fn merging_moves_every_fact_and_forgetting_takes_them_all() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, StubLlm::new()).await;
    let mut transcript = Transcript::new(start());
    let short = save(
        &engine,
        &mut transcript,
        "Acme pays on the 5th.",
        "acme",
        None,
    )
    .await;
    let long = save(
        &engine,
        &mut transcript,
        "Anna works at Acme GmbH.",
        "anna",
        Some("acme gmbh"),
    )
    .await;
    let acme = short.subject_id.clone().unwrap();
    let gmbh = long.object_id.clone().unwrap();
    assert_ne!(acme, gmbh);

    let merged = engine
        .merge_entity(&gmbh, &acme, transcript.now())
        .await
        .unwrap();
    assert_eq!(merged.id, acme);
    assert!(merged.aliases.contains(&"acme gmbh".to_string()));
    let moved = engine.stores().facts.get(&long.id).await.unwrap().unwrap();
    assert_eq!(moved.object_id.as_deref(), Some(acme.as_str()));
    assert!(
        engine
            .merge_entity(OWNER_ENTITY, &acme, transcript.now())
            .await
            .is_err()
    );

    let renamed = engine
        .update_entity(
            &acme,
            Some("ACME Corp"),
            Some(EntityKind::Organization),
            transcript.now(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "ACME Corp");
    assert!(renamed.aliases.contains(&"acme corp".to_string()));

    let forgotten = engine.forget_entity(&acme, transcript.now()).await.unwrap();
    assert_eq!(forgotten.len(), 2);
    assert!(
        engine
            .stores()
            .facts
            .all(&FactFilter {
                include_superseded: true,
                ..FactFilter::default()
            })
            .await
            .unwrap()
            .is_empty()
    );
    let left: Vec<String> = engine
        .stores()
        .entities
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|entity| entity.id)
        .collect();
    assert!(!left.contains(&acme) && !left.contains(&gmbh), "{left:?}");
    assert!(
        engine
            .forget_entity(OWNER_ENTITY, transcript.now())
            .await
            .is_err()
    );
}
