use super::*;
use flow_like_types::Value;

#[test]
fn project_history_keeps_operator_dataset_exclusions() {
    use super::super::auto_training_tables::DatasetSelection;
    let mut selected = DatasetSelection {
        excluded_ids: vec!["operator".into()],
        excluded_groups: vec!["operator-group".into()],
        ineligible_test_ids: vec!["old-training".into()],
        ineligible_test_groups: vec!["old-group".into()],
    };
    merge_dataset_exclusions(
        &mut selected,
        LearningDatasetExclusions {
            sealed_audit_sample_ids: vec!["audit".into(), "operator".into()],
            sealed_audit_group_ids: vec!["audit-group".into()],
            previously_trained_sample_ids: vec!["trained".into()],
            previously_trained_group_ids: vec!["trained-group".into()],
        },
    );
    assert_eq!(selected.excluded_ids, ["audit", "operator"]);
    assert_eq!(selected.excluded_groups, ["audit-group", "operator-group"]);
    assert_eq!(selected.ineligible_test_ids, ["old-training", "trained"]);
    assert_eq!(
        selected.ineligible_test_groups,
        ["old-group", "trained-group"]
    );
}

#[test]
fn rolling_temporal_boundaries_advance_without_changing_embargo() {
    use super::super::auto_training_tables::TableSplitPolicy;
    let rolling = RollingTimeSplit {
        validation_duration_ms: 200,
        test_duration_ms: 100,
    };
    let mut split = TableSplitPolicy::Time {
        train_end_ms: 0,
        validation_end_ms: 0,
        embargo_ms: 25,
    };
    for (at_ms, expected_train, expected_validation) in [(1000, 700, 900), (1500, 1200, 1400)] {
        rolling.apply(&mut split, at_ms).unwrap();
        let TableSplitPolicy::Time {
            train_end_ms,
            validation_end_ms,
            embargo_ms,
        } = split
        else {
            panic!("temporal split changed kind")
        };
        assert_eq!(
            (train_end_ms, validation_end_ms, embargo_ms),
            (expected_train, expected_validation, 25)
        );
    }
    assert!(rolling.apply(&mut split, i64::MIN).is_err());
    let mut groups = TableSplitPolicy::Group {
        train_fraction: 0.6,
        validation_fraction: 0.2,
        seed: 42,
    };
    assert!(rolling.apply(&mut groups, 1000).is_err());
    assert!(
        RollingTimeSplit {
            validation_duration_ms: 0,
            test_duration_ms: 100
        }
        .apply(&mut split, 1000)
        .is_err()
    );
}

#[tokio::test]
async fn learning_project_reopens_source_retrains_and_promotes_on_fresh_audit_groups() {
    run_learning_project(CanaryReviewScenario::Unchanged).await;
}

#[tokio::test]
async fn corrected_canary_review_rejects_cached_predictions_and_retains_champion() {
    run_learning_project(CanaryReviewScenario::CorrectedAutomatic).await;
}

#[tokio::test]
async fn manual_promotion_preserves_corrected_canary_rejection() {
    run_learning_project(CanaryReviewScenario::CorrectedManual).await;
}

enum CanaryReviewScenario {
    Unchanged,
    CorrectedAutomatic,
    CorrectedManual,
}

async fn run_learning_project(scenario: CanaryReviewScenario) {
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{
                LogLevel, Run, context::ExecutionContextCache, internal_node::InternalNode,
            },
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState, FlowLikeStores},
        utils::http::HTTPClient,
    };
    use flow_like_catalog_core::{CachedDB, NodeDBConnection};
    use flow_like_storage::{
        databases::vector::{
            VectorStore, buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
        },
        files::store::{FlowLikeStore, local_store::LocalObjectStore},
        object_store::path::Path,
    };
    use flow_like_types::sync::{Mutex, RwLock};
    use std::sync::{Arc, Weak};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(std::env::temp_dir().join(format!(
        "flow-like-learning-project-{}",
        uuid::Uuid::new_v4()
    )));
    std::fs::create_dir_all(&directory.0).unwrap();
    let db_location = directory.0.join("database").to_string_lossy().into_owned();
    let mut config = FlowLikeConfig::new();
    let reopen = db_location.clone();
    config.register_build_project_database(Arc::new(move |_| {
        flow_like_storage::lancedb::connect(&reopen)
    }));
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));
    let logic: Arc<dyn NodeLogic> = Arc::new(CreateLearningProjectNode);
    let node = Arc::new(InternalNode::new(
        logic.get_node(),
        AHashMap::new(),
        logic,
        AHashMap::new(),
    ));
    let mut nodes = AHashMap::new();
    nodes.insert(node.node_id().to_string(), node.clone());
    let run: Weak<Mutex<Run>> = Weak::new();
    let mut context = ExecutionContext::new(
        Arc::new(nodes),
        &run,
        &state,
        &node,
        &Arc::new(Mutex::new(AHashMap::new())),
        &Arc::new(RwLock::new(AHashMap::new())),
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
        None,
    )
    .await;
    let mut stores = FlowLikeStores::default();
    stores.app_storage_store = Some(FlowLikeStore::Local(Arc::new(
        LocalObjectStore::new(directory.0.clone()).unwrap(),
    )));
    context.execution_cache = Some(ExecutionContextCache {
        stores,
        app_id: "learning-test".into(),
        model_usage_app_id: None,
        board_dir: Path::from("board"),
        board_id: "board".into(),
        node_id: node.shared_node_id(),
        sub: "tester".into(),
        shadow: false,
    });
    let connection = flow_like_storage::lancedb::connect(&db_location)
        .execute()
        .await
        .unwrap();
    let mut store = LanceDBVectorStore::from_connection(connection, "measurements".into()).await;
    let rows = |range: std::ops::Range<usize>| -> Vec<Value> {
        range.map(|i|json!({"id":i.to_string(),"group":(i/2).to_string(),"time":i as i64+1,"measurement":(i%2) as f64*100.+i as f64/1000.,"target":if i%2==0{"pass"}else{"fail"}})).collect()
    };
    store.insert(rows(0..240)).await.unwrap();
    let source = NodeDBConnection {
        cache_key: "learning-source".into(),
    };
    context.cache.write().await.insert(
        source.cache_key.clone(),
        Arc::new(CachedDB {
            db: Arc::new(RwLock::new(BufferedVectorStore::new(store, 0))),
        }),
    );
    let recipe = super::super::auto_training_legacy::LegacyRecipe {
        algorithm: super::super::auto_training_legacy::LegacyAlgorithm::GaussianNaiveBayes,
        labels: vec!["pass".into(), "fail".into()],
    };
    let request:CreateLearningProjectRequest=flow_like_types::json::from_value(json!({
        "training":{
            "dataset":{"source":source,"stream_id":"quality","spec":{"id":"quality","description":"Predict measured quality","task":"sensor_classification","labels":["pass","fail"],"input_shape":[1],"prediction_horizon_ms":null,"minimum_examples":2,"minimum_examples_per_class":1},
                "mapping":{"row_id":"id","group_id":"group","timestamp_ms":"time","features":{"kind":"tabular","options":{"numeric_columns":["measurement"],"categorical_columns":[]}},"target":{"kind":"column","column":"target"},"provenance":{"kind":"measured","source":"gauge"}},
                "split":{"kind":"group","train_fraction":0.6,"validation_fraction":0.2,"seed":42}},
            "compute":{"backend":"cpu"},"search":{"maximum_candidates":1},"budget":{"maximum_trials":1},
            "feature_plan":{"pipeline":{"steps":[{"kind":"derive_columns","columns":[{"name":"energy","expression":{"kind":"unary","op":"square","value":{"kind":"column","name":"measurement"}}}]}]},"numeric_columns":["measurement","energy"],"categorical_columns":[],"standardize":true},
            "initial_candidates":[{"engine":"legacy_classical","recipe":recipe,"compute":{"backend":"cpu"}}],
            "goals":{"task":{"kind":"classification"},"primary_metric":"accuracy","direction":"maximize","minimum_audited_samples":4,"bounds":[{"name":"accuracy","minimum":0.9,"maximum":null}]}
        },"policy":{"minimum_new_reviews":1,"minimum_audit_samples":4,"cooldown_ms":0,"canary_fraction":0.5},"automatic_promotion":true
    })).unwrap();
    let project = create(&mut context, request).await.unwrap();
    let id = project.project.id;
    let prepared = prepare_learning_cycle(&mut context, &id).await.unwrap();
    let experiment_id = prepared.experiment.unwrap().experiment_id;
    super::super::auto_training::run_next_auto_training(&context, &experiment_id)
        .await
        .unwrap();
    let exhausted = super::super::auto_training::run_next_auto_training(&context, &experiment_id)
        .await
        .unwrap();
    assert_eq!(
        exhausted.status,
        flow_like_ml_runtime::ExperimentStatus::BudgetExhausted
    );
    let pending = project_repository(&context, &id)
        .unwrap()
        .get_experiment(&experiment_id)
        .unwrap();
    assert!(pending.best_trial_id.is_some());
    assert!(pending.final_evaluation_id.is_none());
    let first = step(
        &mut context,
        LearningProjectId {
            project_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(first.project.state, LearningState::Active);
    assert!(first.comparison.as_ref().unwrap().eligible);
    let champion = first.project.champion_artifact_id.clone().unwrap();
    let original_audit = first.cycles[0].audit_group_ids.clone();
    let cached = source.load(&mut context).await.unwrap();
    cached
        .upsert_from(&context, rows(240..480), "id".into())
        .await
        .unwrap();
    cached.ensure_flushed().await.unwrap();
    let repo = project_repository(&context, &id).unwrap();
    repo.record_learning_observation(
        &id,
        &LearningObservation {
            sample_id: "240".into(),
            group_id: "120".into(),
            captured_at_ms: 241,
            student: None,
            teacher: None,
            embedding: vec![],
            slices: Default::default(),
            sample: json!({}),
        },
    )
    .unwrap();
    repo.record_learning_review(
        &id,
        &LearningReview {
            sample_id: "240".into(),
            annotation: json!({"kind":"class","class_id":0}),
            source: flow_like_ml_runtime::LabelSource::ObservedOutcome,
            reviewer: "gauge".into(),
            available_at_ms: flow_like_ml_runtime::now_ms(),
            revision: 1,
            outcome: None,
        },
    )
    .unwrap();
    context.cache.write().await.clear();
    let second = step(
        &mut context,
        LearningProjectId {
            project_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(second.project.state, LearningState::Canary);
    assert_eq!(second.project.usage.cycles_finished, 2);
    assert!(
        second.cycles[1]
            .audit_group_ids
            .iter()
            .all(|group| !original_audit.contains(group))
    );
    assert!(second.comparison.as_ref().unwrap().champion_metrics["accuracy"] >= 0.9);
    let candidate = second.project.candidate_artifact_id.clone().unwrap();
    for (i, mut row) in (480..488).zip(rows(480..488)) {
        let at = flow_like_ml_runtime::now_ms();
        row["time"] = json!(at);
        repo.record_learning_observation(
            &id,
            &LearningObservation {
                sample_id: i.to_string(),
                group_id: (i / 2).to_string(),
                captured_at_ms: at,
                student: Some(flow_like_ml_runtime::learning::LearningPrediction {
                    artifact_id: Some(candidate.clone()),
                    label: Some(if i % 2 == 0 { "pass" } else { "fail" }.into()),
                    confidence: 0.95,
                    probabilities: vec![],
                }),
                teacher: None,
                embedding: vec![],
                slices: Default::default(),
                sample: row,
            },
        )
        .unwrap();
        repo.record_learning_review(
            &id,
            &LearningReview {
                sample_id: i.to_string(),
                annotation: json!({"kind":"class","class_id":i%2}),
                source: flow_like_ml_runtime::LabelSource::ObservedOutcome,
                reviewer: "gauge".into(),
                available_at_ms: at,
                revision: 1,
                outcome: None,
            },
        )
        .unwrap();
    }
    if !matches!(scenario, CanaryReviewScenario::Unchanged) {
        let project = repo.get_learning_project(&id).unwrap();
        inference::audit_canary(&repo, &project).unwrap();
        let predictions = json!(repo.predictions(&candidate).unwrap());
        let cohort = repo
            .learning_canary_sample_ids(&id, flow_like_ml_runtime::now_ms())
            .unwrap();
        assert!(!cohort.is_empty());
        let mut corrected = repo
            .learning_training_reviews(&id)
            .unwrap()
            .into_iter()
            .find(|review| review.sample_id == cohort[0])
            .unwrap();
        corrected.revision += 1;
        corrected.available_at_ms = flow_like_ml_runtime::now_ms();
        corrected.annotation["class_id"] =
            json!(1 - corrected.annotation["class_id"].as_u64().unwrap());
        repo.record_learning_review(&id, &corrected).unwrap();

        let rejected = match scenario {
            CanaryReviewScenario::CorrectedManual => promote(
                &mut context,
                LearningProjectId {
                    project_id: id.clone(),
                },
            )
            .await
            .unwrap(),
            _ => finish_learning_cycle(&mut context, &id).await.unwrap(),
        };
        assert_eq!(rejected.project.state, LearningState::Active);
        assert_eq!(rejected.project.champion_artifact_id, Some(champion));
        assert!(rejected.project.candidate_artifact_id.is_none());
        let comparison = rejected.comparison.unwrap();
        assert!(!comparison.eligible);
        assert!(comparison.reasons.contains(&"canary_review_changed".into()));
        assert_eq!(json!(repo.predictions(&candidate).unwrap()), predictions);
        return;
    }

    let promoted = step(
        &mut context,
        LearningProjectId {
            project_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(promoted.project.state, LearningState::Active);
    assert_eq!(promoted.project.champion_artifact_id, Some(candidate));
    let deployment = repo
        .get_deployment(&second.project.request.deployment_id)
        .unwrap();
    assert_eq!(deployment.previous_artifact_id, Some(champion.clone()));
    let rolled = rollback(
        &mut context,
        LearningProjectId {
            project_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(rolled.project.champion_artifact_id, Some(champion));
    let paused = pause(
        &mut context,
        LearningProjectId {
            project_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(paused.project.state, LearningState::Paused);
    assert!(matches!(
        repo.learning_route(&id, "part-1").unwrap(),
        LearningRoute::Teacher
    ));
    assert_eq!(
        resume(&mut context, LearningProjectId { project_id: id })
            .await
            .unwrap()
            .project
            .state,
        LearningState::Active
    );
}
