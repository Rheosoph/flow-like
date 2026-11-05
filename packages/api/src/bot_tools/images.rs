use std::sync::Arc;

use async_trait::async_trait;
use flow_like::flow::execution::ExecutionEnvironment;
use flow_like::models::llm::ModelUsageContext;
use flow_like::profile::Profile;
use flow_like::state::FlowLikeState;
use flow_like_bot_tools::image::{
    CatalogImages, GeneratedImage, ImageBackend, ImageBudget, ImageJob, ImageModelInfo,
    ImageUnavailable, hosted_access, resolve_bit,
};
use flow_like_bot_tools_host::ImageSource;
use flow_like_types::tokio::sync::OnceCell;

/// The bot's image model for one server turn. The profile the API built for the turn holds the
/// owner's private Bits with their keys, so the call to the provider happens here and no key
/// travels to an executor.
pub(crate) struct ApiImages {
    profile: Arc<Profile>,
    state: Arc<FlowLikeState>,
    image_model: Option<String>,
    token: Option<String>,
    workspace: String,
    turn_id: String,
    budget: Arc<ImageBudget>,
    backend: OnceCell<Arc<dyn ImageBackend>>,
}

impl ApiImages {
    pub(crate) fn new(
        profile: Arc<Profile>,
        state: Arc<FlowLikeState>,
        image_model: Option<String>,
        token: Option<String>,
        workspace: String,
        turn_id: String,
    ) -> Self {
        Self {
            profile,
            state,
            image_model,
            token,
            workspace,
            turn_id,
            budget: Arc::new(ImageBudget::default()),
            backend: OnceCell::new(),
        }
    }

    async fn build(&self) -> Result<Arc<dyn ImageBackend>, ImageUnavailable> {
        let bit = resolve_bit(&self.profile, &self.state, self.image_model.as_deref()).await?;
        let usage = ModelUsageContext {
            app_id: Some(self.workspace.clone()),
            run_id: Some(self.turn_id.clone()),
            api_base_url: None,
        };
        let hosted = hosted_access(&self.state, self.token.clone(), Some(&usage));
        let backend = CatalogImages::resolve(
            bit,
            &self.profile,
            &self.state,
            ExecutionEnvironment::server_default(),
            Some(hosted),
            self.budget.clone(),
        )
        .await?;
        Ok(Arc::new(Logged {
            inner: backend,
            turn_id: self.turn_id.clone(),
        }))
    }
}

#[async_trait]
impl ImageSource for ApiImages {
    async fn images(&self) -> Result<Arc<dyn ImageBackend>, ImageUnavailable> {
        self.backend.get_or_try_init(|| self.build()).await.cloned()
    }
}

struct Logged {
    inner: CatalogImages,
    turn_id: String,
}

#[async_trait]
impl ImageBackend for Logged {
    fn model(&self) -> ImageModelInfo {
        self.inner.model()
    }

    fn budget(&self) -> &ImageBudget {
        self.inner.budget()
    }

    async fn generate(&self, job: ImageJob) -> flow_like_types::Result<Vec<GeneratedImage>> {
        let started = std::time::Instant::now();
        let requested = job.count;
        let images = self.inner.generate(job).await?;
        tracing::info!(
            turn_id = %self.turn_id,
            model = %self.inner.model().name,
            requested,
            images = images.len(),
            duration_ms = started.elapsed().as_millis() as u64,
            "[bot_tools] image generation finished"
        );
        Ok(images)
    }
}
