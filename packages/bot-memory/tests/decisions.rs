//! Optional fact curation keeps the existing updater's evidence, chronology and trust rules.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use flow_like_bot_memory::compaction::CompactionConfig;
use flow_like_bot_memory::engine::MemoryEngine;
use flow_like_bot_memory::facts::{
    DecidedBy, Evidence, Fact, FactKind, UpdateAction, integrate_facts,
};
use flow_like_bot_memory::host::{DecisionLlm, RoutedLlm, SwitchLlm};
use flow_like_bot_memory::llm::{LlmRequest, LlmTask, MemoryLlm};
use flow_like_bot_memory::model::FactSource;
use flow_like_bot_memory::store::MemoryStores;
use flow_like_bot_memory::testing::{HashEmbedder, StubLlm, Transcript};
use flow_like_bot_memory::text::content_hash;
use flow_like_bot_memory::tools::{MemorySaveArgs, WriteGate, WriteOutcome, memory_save};
use flow_like_model_provider::systemone::{
    SystemOneAnswer, SystemOneModelLogic, SystemOneQuestion, SystemOneRequest, SystemOneResponse,
};
use flow_like_types::{Cacheable, Result, anyhow};

type Answer = dyn Fn(&SystemOneRequest) -> Result<SystemOneResponse> + Send + Sync;

struct Classifier {
    answer: Box<Answer>,
    requests: Mutex<Vec<SystemOneRequest>>,
}

impl Classifier {
    fn new(
        answer: impl Fn(&SystemOneRequest) -> Result<SystemOneResponse> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            answer: Box::new(answer),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn choosing(choice: &'static str) -> Arc<Self> {
        Self::new(move |request| Ok(response(request, choice)))
    }
}

impl Cacheable for Classifier {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl SystemOneModelLogic for Classifier {
    async fn invoke(&self, request: &SystemOneRequest) -> Result<SystemOneResponse> {
        self.requests.lock().unwrap().push(request.clone());
        (self.answer)(request)
    }
}

fn response(request: &SystemOneRequest, choice: &str) -> SystemOneResponse {
    let (id, SystemOneQuestion::Choice { criteria, .. }) = request.questions.iter().next().unwrap()
    else {
        panic!("expected a choice");
    };
    SystemOneResponse {
        model: "decision-test".into(),
        answers: BTreeMap::from([(
            id.clone(),
            SystemOneAnswer::Choice {
                choice: choice.into(),
                probabilities: criteria
                    .keys()
                    .map(|key| (key.clone(), if key == choice { 1.0 } else { 0.0 }))
                    .collect(),
                confidence: 1.0,
            },
        )]),
        usage: None,
        extra: BTreeMap::new(),
    }
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-01T12:00:00Z")
        .unwrap()
        .into()
}

fn fact(id: &str, text: &str, seq: u64) -> Fact {
    let at = now() + Duration::minutes(seq as i64);
    let mut fact = Fact {
        id: id.into(),
        text: text.into(),
        kind: FactKind::Knowledge,
        subject: "user".into(),
        valid_from: None,
        valid_to: None,
        recorded_at: at,
        expired_at: None,
        geojson: None,
        time: None,
        expires_at: None,
        evidence: vec![Evidence {
            seq,
            message_id: format!("m{seq}"),
            speaker: None,
            part: None,
            quote: text.into(),
            at,
            partial: false,
        }],
        segment_id: None,
        source: FactSource::UserStated,
        confidence: 0.9,
        superseded_by: None,
        content_hash: String::new(),
    };
    fact.content_hash = content_hash(&fact.subject, &fact.text);
    fact
}

async fn stores(dir: &tempfile::TempDir) -> MemoryStores {
    MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new("hash", 64)))
        .await
        .unwrap()
}

#[tokio::test]
async fn curation_can_discard_the_first_mined_candidate_without_completion() {
    let dir = tempfile::tempdir().unwrap();
    let stores = stores(&dir).await;
    let completion = Arc::new(StubLlm::new());
    let classifier = Classifier::choosing("noop");
    let llm = DecisionLlm::new(completion.clone(), classifier.clone());
    let candidate = fact("f_transient", "The user said hello.", 1);
    let report = integrate_facts(&stores.facts, &llm, vec![candidate.clone()], Tz::UTC, now())
        .await
        .unwrap();
    assert_eq!(report.decisions[0].action, UpdateAction::Noop);
    assert_eq!(report.decisions[0].decided_by, DecidedBy::DecisionModel);
    assert!(stores.facts.get(&candidate.id).await.unwrap().is_none());
    assert!(completion.requests().is_empty());
    let requests = classifier.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].state["existing"], serde_json::json!([]));
}

#[tokio::test]
async fn invalid_or_unavailable_decisions_fall_back_to_the_completion_updater() {
    for unavailable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let stores = stores(&dir).await;
        let existing = fact("f_existing", "The user's city is Berlin.", 1);
        stores
            .facts
            .insert(std::slice::from_ref(&existing))
            .await
            .unwrap();
        let completion = Arc::new(StubLlm::new().with(LlmTask::FactUpdate, |_| {
            Ok("{\"action\":\"supersede\",\"target_id\":\"f_existing\"}".into())
        }));
        let classifier = Classifier::new(move |request| {
            if unavailable {
                Err(anyhow!("offline"))
            } else {
                Ok(response(request, "supersede:f_unoffered"))
            }
        });
        let llm = DecisionLlm::new(completion.clone(), classifier);
        let candidate = fact("f_candidate", "The user's city is Paris.", 2);
        let report = integrate_facts(&stores.facts, &llm, vec![candidate.clone()], Tz::UTC, now())
            .await
            .unwrap();
        assert_eq!(report.decisions[0].decided_by, DecidedBy::Model);
        assert_eq!(
            report.decisions[0].action,
            UpdateAction::Supersede {
                target: existing.id.clone()
            }
        );
        assert_eq!(completion.requests().len(), 1);
        assert_eq!(
            stores
                .facts
                .get(&existing.id)
                .await
                .unwrap()
                .unwrap()
                .superseded_by,
            Some(candidate.id)
        );
    }
}

#[tokio::test]
async fn decision_cannot_replace_more_trusted_or_newer_evidence() {
    for older in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let stores = stores(&dir).await;
        let existing = fact("f_existing", "The user's city is Berlin.", 3);
        stores
            .facts
            .insert(std::slice::from_ref(&existing))
            .await
            .unwrap();
        let completion = Arc::new(StubLlm::new());
        let llm = DecisionLlm::new(
            completion.clone(),
            Classifier::choosing("supersede:f_existing"),
        );
        let mut candidate = fact(
            "f_candidate",
            "The user's city is Paris.",
            if older { 1 } else { 4 },
        );
        if !older {
            candidate.source = FactSource::ToolUntrusted;
        }
        let report = integrate_facts(&stores.facts, &llm, vec![candidate.clone()], Tz::UTC, now())
            .await
            .unwrap();
        assert_eq!(
            report.decisions[0].decided_by,
            if older {
                DecidedBy::TimeGuard
            } else {
                DecidedBy::TrustGuard
            }
        );
        assert!(
            stores
                .facts
                .get(&existing.id)
                .await
                .unwrap()
                .unwrap()
                .is_active()
        );
        let stored = stores.facts.get(&candidate.id).await.unwrap().unwrap();
        assert_eq!(stored.evidence, candidate.evidence);
        assert_eq!(stored.superseded_by.is_some(), older);
        assert!(completion.requests().is_empty());
    }
}

#[tokio::test]
async fn exact_duplicates_merge_evidence_without_inference() {
    let dir = tempfile::tempdir().unwrap();
    let stores = stores(&dir).await;
    let existing = fact("f_existing", "The user's city is Berlin.", 1);
    stores
        .facts
        .insert(std::slice::from_ref(&existing))
        .await
        .unwrap();
    let classifier = Classifier::choosing("noop");
    let llm = DecisionLlm::new(Arc::new(StubLlm::new()), classifier.clone());
    let candidate = fact("f_again", &existing.text, 2);
    let report = integrate_facts(&stores.facts, &llm, vec![candidate], Tz::UTC, now())
        .await
        .unwrap();
    assert_eq!(report.decisions[0].decided_by, DecidedBy::Hash);
    assert_eq!(
        stores
            .facts
            .get(&existing.id)
            .await
            .unwrap()
            .unwrap()
            .evidence_seqs(),
        [1, 2]
    );
    assert!(classifier.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_memory_save_cannot_be_dismissed_by_either_model() {
    let dir = tempfile::tempdir().unwrap();
    let stores = stores(&dir).await;
    stores
        .facts
        .insert(&[fact("f_existing", "The user's city is Berlin.", 1)])
        .await
        .unwrap();
    let completion = Arc::new(StubLlm::new().with(LlmTask::FactUpdate, |_| {
        Ok("{\"action\":\"noop\",\"target_id\":null}".into())
    }));
    let classifier = Classifier::choosing("noop");
    let llm = Arc::new(DecisionLlm::new(completion.clone(), classifier.clone()));
    let engine = MemoryEngine::new(stores, llm, CompactionConfig::default(), Tz::UTC);
    let mut transcript = Transcript::new(now());
    let seq = transcript.user("Remember that I am allergic to peanuts.");
    let evidence = transcript.history.get(seq).unwrap();
    let result = memory_save(
        &engine,
        &MemorySaveArgs {
            geojson: None,
            time: None,
            ttl_seconds: None,

            text: "The user is allergic to peanuts.".into(),
            evidence_seq: Some(seq),
            kind: Some("profile".into()),
            subject: Some("user".into()),
            valid_from: None,
            valid_to: None,
        },
        &evidence,
        WriteGate::default(),
        now(),
    )
    .await
    .unwrap();
    let WriteOutcome::Done((saved, action)) = result else {
        panic!("save was refused");
    };
    assert_eq!(action, UpdateAction::Add);
    assert_eq!(
        engine.stores().facts.get(&saved.id).await.unwrap(),
        Some(saved)
    );
    assert_eq!(completion.requests().len(), 1);
    let requests = classifier.requests.lock().unwrap();
    let SystemOneQuestion::Choice { criteria, .. } = &requests[0].questions["fact_update"] else {
        panic!("expected choice");
    };
    assert!(!criteria.contains_key("noop"));
}

#[tokio::test]
async fn refreshed_models_replace_curation_and_removing_the_pin_restores_default_rules() {
    let dir = tempfile::tempdir().unwrap();
    let stores = stores(&dir).await;
    let completion = Arc::new(StubLlm::new());
    let first = Classifier::choosing("noop");
    let second = Classifier::choosing("add");
    let routed = |classifier: Arc<Classifier>| -> Arc<dyn MemoryLlm> {
        Arc::new(RoutedLlm::new(completion.clone()).route(
            LlmTask::FactUpdate,
            Arc::new(DecisionLlm::new(completion.clone(), classifier)),
        ))
    };
    let llm = SwitchLlm::new(routed(first.clone()));
    let candidate = fact("f_candidate", "The user's city is Paris.", 1);
    integrate_facts(&stores.facts, &llm, vec![candidate.clone()], Tz::UTC, now())
        .await
        .unwrap();
    llm.set(routed(second.clone()));
    integrate_facts(&stores.facts, &llm, vec![candidate.clone()], Tz::UTC, now())
        .await
        .unwrap();
    assert!(stores.facts.get(&candidate.id).await.unwrap().is_some());
    llm.set(completion.clone());
    let mut next = fact("f_next", "Project Atlas is due on Friday.", 2);
    next.subject = "project atlas".into();
    next.content_hash = content_hash(&next.subject, &next.text);
    let report = integrate_facts(&stores.facts, &llm, vec![next], Tz::UTC, now())
        .await
        .unwrap();
    assert_eq!(report.decisions[0].decided_by, DecidedBy::NoNeighbours);
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    assert_eq!(second.requests.lock().unwrap().len(), 1);
    assert!(completion.requests().is_empty());
}

#[tokio::test]
async fn summaries_and_extraction_use_completion_even_with_a_decision_model() {
    let completion = Arc::new(StubLlm::new());
    let classifier = Classifier::choosing("noop");
    let llm = DecisionLlm::new(completion.clone(), classifier.clone());
    for task in [LlmTask::SegmentSummary, LlmTask::FactExtraction] {
        llm.complete(LlmRequest {
            task,
            system: String::new(),
            prompt: String::new(),
            json_schema: None,
            max_output_tokens: 128,
        })
        .await
        .unwrap();
    }
    assert_eq!(completion.requests().len(), 2);
    assert!(classifier.requests.lock().unwrap().is_empty());
}
