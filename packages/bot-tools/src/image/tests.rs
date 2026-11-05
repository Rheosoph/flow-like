use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use flow_like_types::Bytes;
use flow_like_types::data_classification::DataClassification;
use serde_json::{Value, json};

use super::*;
use crate::outcome::ToolOutcome;
use crate::run;
use crate::testing::{FakeEnv, WORKSPACE};

struct FakeImages {
    budget: ImageBudget,
    external: bool,
    returns: Option<u32>,
    fail: bool,
    calls: AtomicU32,
    jobs: std::sync::Mutex<Vec<ImageJob>>,
}

impl FakeImages {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            budget: ImageBudget::default(),
            external: true,
            returns: None,
            fail: false,
            calls: AtomicU32::new(0),
            jobs: Default::default(),
        })
    }

    fn with(external: bool, returns: Option<u32>, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            budget: ImageBudget::default(),
            external,
            returns,
            fail,
            calls: AtomicU32::new(0),
            jobs: Default::default(),
        })
    }
}

#[async_trait]
impl ImageBackend for FakeImages {
    fn model(&self) -> ImageModelInfo {
        ImageModelInfo {
            name: "fake-image-1".into(),
            external: self.external,
        }
    }

    fn budget(&self) -> &ImageBudget {
        &self.budget
    }

    async fn generate(&self, job: ImageJob) -> flow_like_types::Result<Vec<GeneratedImage>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.jobs.lock().expect("jobs").push(job.clone());
        if self.fail {
            return Err(flow_like_types::anyhow!("provider said no"));
        }
        let count = self.returns.unwrap_or(job.count);
        Ok((0..count)
            .map(|n| GeneratedImage {
                bytes: Bytes::from(vec![n as u8; 64]),
                mime: "image/png".into(),
                metadata: json!({ "ai_generated": true }),
            })
            .collect())
    }
}

async fn env_with(images: Arc<FakeImages>) -> FakeEnv {
    let env = FakeEnv::new().await;
    *env.images.lock().expect("images") = Ok(images);
    env
}

async fn generate(env: &FakeEnv, args: Value) -> ToolOutcome {
    run(IMAGE_GENERATE, args, env).await
}

fn code_of(outcome: &ToolOutcome) -> &'static str {
    assert!(
        !outcome.is_ok(),
        "expected an error, got {:?}",
        outcome.result
    );
    outcome.error.as_ref().expect("error").code
}

#[tokio::test]
async fn images_land_in_the_workspace_and_reach_the_model() {
    let images = FakeImages::new();
    let env = env_with(images.clone()).await;
    let outcome = generate(
        &env,
        json!({ "prompt": "a lighthouse", "count": 2, "size": "1536x1024", "style": "watercolor", "seed": 3 }),
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.saved.len(), 2);
    assert_eq!(outcome.images.len(), 2);
    for handle in &outcome.saved {
        assert_eq!(handle.app_id, WORKSPACE);
        assert!(
            handle.path.starts_with("bot/images/img-"),
            "{}",
            handle.path
        );
        assert!(handle.path.ends_with(".png"));
        assert!(
            env.files
                .lock()
                .expect("files")
                .contains_key(&(WORKSPACE.to_string(), handle.path.clone()))
        );
    }
    assert_ne!(outcome.saved[0].path, outcome.saved[1].path);
    assert_eq!(outcome.result["ai_generated"], true);
    assert_eq!(outcome.result["left_this_turn"], 6);
    let job = images.jobs.lock().expect("jobs")[0].clone();
    assert_eq!(job.size, "1536x1024");
    assert_eq!(job.seed, Some(3));
    assert!(job.prompt.contains("Style: watercolor"));
}

#[tokio::test]
async fn every_call_gets_new_names() {
    let env = env_with(FakeImages::new()).await;
    let first = generate(&env, json!({ "prompt": "a" })).await;
    let second = generate(&env, json!({ "prompt": "a" })).await;
    assert_ne!(first.saved[0].path, second.saved[0].path);
}

#[tokio::test]
async fn bad_arguments_are_refused_before_a_model_is_asked() {
    let images = FakeImages::new();
    let env = env_with(images.clone()).await;
    for args in [
        json!({ "prompt": "   " }),
        json!({ "prompt": "x", "count": 0 }),
        json!({ "prompt": "x", "count": 5 }),
        json!({ "prompt": "x".repeat(MAX_PROMPT_CHARS + 1) }),
        json!({ "prompt": "x", "style": "s".repeat(MAX_STYLE_CHARS + 1) }),
        json!({ "prompt": "x", "size": "640x480" }),
        json!({ "prompt": "x", "count": "many" }),
        json!({}),
    ] {
        let outcome = generate(&env, args.clone()).await;
        assert_eq!(code_of(&outcome), code::INVALID_ARGS, "{args}");
    }
    assert_eq!(images.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_turn_generates_at_most_eight_images() {
    let images = FakeImages::new();
    let env = env_with(images.clone()).await;
    assert!(
        generate(&env, json!({ "prompt": "a", "count": 4 }))
            .await
            .is_ok()
    );
    assert!(
        generate(&env, json!({ "prompt": "a", "count": 4 }))
            .await
            .is_ok()
    );
    let third = generate(&env, json!({ "prompt": "a" })).await;
    assert_eq!(code_of(&third), code::RATE_LIMITED);
    assert!(third.error.expect("error").message.contains("0 are left"));
    assert_eq!(images.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_call_that_would_pass_the_turn_cap_is_refused_whole() {
    let images = FakeImages::new();
    let env = env_with(images.clone()).await;
    assert!(
        generate(&env, json!({ "prompt": "a", "count": 4 }))
            .await
            .is_ok()
    );
    assert!(
        generate(&env, json!({ "prompt": "a", "count": 3 }))
            .await
            .is_ok()
    );
    let over = generate(&env, json!({ "prompt": "a", "count": 2 })).await;
    assert_eq!(code_of(&over), code::RATE_LIMITED);
    assert!(over.error.expect("error").message.contains("1 are left"));
    assert!(generate(&env, json!({ "prompt": "a" })).await.is_ok());
}

#[tokio::test]
async fn failed_and_short_generations_give_the_allowance_back() {
    let failing = FakeImages::with(true, None, true);
    let env = env_with(failing.clone()).await;
    let outcome = generate(&env, json!({ "prompt": "a", "count": 4 })).await;
    assert_eq!(code_of(&outcome), code::FAILED);
    assert!(
        outcome
            .error
            .expect("error")
            .message
            .contains("provider said no")
    );
    assert_eq!(failing.budget.left(), MAX_IMAGES_PER_TURN);

    let short = FakeImages::with(true, Some(1), false);
    let env = env_with(short.clone()).await;
    let outcome = generate(&env, json!({ "prompt": "a", "count": 4 })).await;
    assert!(outcome.is_ok());
    assert_eq!(outcome.saved.len(), 1);
    assert_eq!(short.budget.left(), MAX_IMAGES_PER_TURN - 1);

    let none = FakeImages::with(true, Some(0), false);
    let env = env_with(none.clone()).await;
    assert_eq!(
        code_of(&generate(&env, json!({ "prompt": "a" })).await),
        code::FAILED
    );
    assert_eq!(none.budget.left(), MAX_IMAGES_PER_TURN);
}

#[tokio::test]
async fn a_bot_without_an_image_model_is_told_where_to_pick_one() {
    let env = FakeEnv::new().await;
    let outcome = generate(&env, json!({ "prompt": "a" })).await;
    assert_eq!(code_of(&outcome), code::NO_IMAGE_MODEL);
    assert!(
        outcome
            .error
            .expect("error")
            .message
            .contains("Brain settings")
    );
}

#[tokio::test]
async fn a_host_without_images_and_an_unusable_model_say_so() {
    let env = FakeEnv::new().await;
    *env.images.lock().expect("images") = Err(ImageUnavailable::NotOnThisHost);
    assert_eq!(
        code_of(&generate(&env, json!({ "prompt": "a" })).await),
        code::NOT_YET
    );
    *env.images.lock().expect("images") = Err(ImageUnavailable::Unusable("no key".into()));
    let outcome = generate(&env, json!({ "prompt": "a" })).await;
    assert_eq!(code_of(&outcome), code::FAILED);
    assert!(outcome.error.expect("error").message.contains("no key"));
}

#[tokio::test]
async fn a_bot_without_a_workspace_cannot_save_images() {
    let mut env = env_with(FakeImages::new()).await;
    env.scope.workspace = None;
    assert_eq!(
        code_of(&generate(&env, json!({ "prompt": "a" })).await),
        code::NO_WORKSPACE
    );
}

#[tokio::test]
async fn prompts_of_a_confidential_bot_stay_off_external_providers() {
    let external = FakeImages::with(true, None, false);
    let mut env = env_with(external.clone()).await;
    env.scope.classification = DataClassification::Confidential;
    let refused = generate(&env, json!({ "prompt": "a" })).await;
    assert_eq!(code_of(&refused), code::SCOPE_REFUSED);
    assert_eq!(external.calls.load(Ordering::SeqCst), 0);
    assert_eq!(external.budget.left(), MAX_IMAGES_PER_TURN);

    let local = FakeImages::with(false, None, false);
    let mut env = env_with(local).await;
    env.scope.classification = DataClassification::Restricted;
    assert!(generate(&env, json!({ "prompt": "a" })).await.is_ok());

    let mut env = env_with(FakeImages::with(true, None, false)).await;
    env.scope.classification = DataClassification::Internal;
    assert!(generate(&env, json!({ "prompt": "a" })).await.is_ok());
}

#[test]
fn the_spec_is_offered_with_the_capability_and_the_three_sizes() {
    let spec = crate::spec(IMAGE_GENERATE).expect("spec");
    assert_eq!(spec.capability, Some("image_generation"));
    assert!(spec.mutating);
    let schema = spec.input_schema.to_string();
    for size in ["1024x1024", "1536x1024", "1024x1536"] {
        assert!(schema.contains(size), "{size} missing from {schema}");
    }
}

#[test]
fn the_budget_reserves_atomically_and_never_underflows() {
    let budget = ImageBudget::new(3);
    assert_eq!(budget.reserve(2), Ok(()));
    assert_eq!(budget.reserve(2), Err(1));
    budget.release(10);
    assert_eq!(budget.left(), 3);
}
