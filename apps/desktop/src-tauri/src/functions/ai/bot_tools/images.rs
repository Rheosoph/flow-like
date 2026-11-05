//! `image_generate` on this device: the bot's image model resolved from the run's profile, with
//! every provider including local stable-diffusion (desktop) and remote providers only on phones.

use std::{
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use dashmap::DashMap;
use flow_like::{flow::execution::ExecutionEnvironment, models::llm::ModelUsageContext};
use flow_like_bot_tools::image::{
    CatalogImages, ImageBackend, ImageBudget, ImageUnavailable, hosted_access, resolve_bit,
};
use flow_like_types::tokio::sync::OnceCell;

use crate::functions::ai::native_bot_tools::BotToolRuntime;

const IDLE_TURN: Duration = Duration::from_secs(15 * 60);

static BUDGETS: LazyLock<DashMap<String, (Arc<ImageBudget>, Instant)>> =
    LazyLock::new(DashMap::new);

const ENVIRONMENT: ExecutionEnvironment = if cfg!(any(target_os = "ios", target_os = "android")) {
    ExecutionEnvironment::Mobile
} else {
    ExecutionEnvironment::Desktop
};

/// One allowance per turn: a turn's tool calls share it though each call builds its own env.
fn turn_budget(runtime: &BotToolRuntime, conversation_id: Option<&str>) -> Arc<ImageBudget> {
    let now = Instant::now();
    BUDGETS.retain(|_, (_, used)| now.duration_since(*used) < IDLE_TURN);
    let turn = runtime
        .turn_id
        .as_deref()
        .or(runtime.run_id.as_deref())
        .or(conversation_id)
        .unwrap_or_default();
    let key = format!("{}\u{1f}{turn}", runtime.context.bot_id);
    let mut entry = BUDGETS
        .entry(key)
        .or_insert_with(|| (Arc::new(ImageBudget::default()), now));
    entry.1 = now;
    entry.0.clone()
}

pub(crate) struct DesktopImages {
    runtime: Arc<BotToolRuntime>,
    budget: Arc<ImageBudget>,
    backend: OnceCell<Arc<dyn ImageBackend>>,
}

impl DesktopImages {
    pub(crate) fn new(runtime: Arc<BotToolRuntime>, conversation_id: Option<&str>) -> Self {
        let budget = turn_budget(&runtime, conversation_id);
        Self {
            runtime,
            budget,
            backend: OnceCell::new(),
        }
    }

    pub(crate) async fn backend(&self) -> Result<Arc<dyn ImageBackend>, ImageUnavailable> {
        self.backend.get_or_try_init(|| self.build()).await.cloned()
    }

    async fn build(&self) -> Result<Arc<dyn ImageBackend>, ImageUnavailable> {
        let runtime = &self.runtime;
        let profile = runtime.profile.as_ref().ok_or_else(|| {
            ImageUnavailable::Unusable("this run has no profile to resolve models from".into())
        })?;
        let bit = resolve_bit(
            profile,
            &runtime.state,
            runtime.context.image_model.as_deref(),
        )
        .await?;
        let usage = ModelUsageContext {
            app_id: runtime.context.workspace_app_id.clone(),
            run_id: runtime.run_id.clone(),
            api_base_url: None,
        };
        let hosted = hosted_access(&runtime.state, runtime.token.clone(), Some(&usage));
        let images = CatalogImages::resolve(
            bit,
            profile,
            &runtime.state,
            ENVIRONMENT,
            Some(hosted),
            self.budget.clone(),
        )
        .await?;
        Ok(Arc::new(images))
    }
}
