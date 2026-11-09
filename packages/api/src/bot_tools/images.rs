use std::sync::Arc;
use std::sync::Mutex;

use flow_like::profile::Profile;
use flow_like::state::FlowLikeState;
use flow_like_bot_tools::image::{
    ImageUnavailable, MAX_IMAGES_PER_TURN, NO_IMAGE_MODEL_MESSAGE, resolve_bit,
};
use flow_like_bot_tools_host::JobImage;
use serde_json::Value;

/// The trusted half of `image_generate`: the API resolves the bot's image Bit from the turn's
/// profile, which holds the owner's private Bits with their keys, and hands it to the one
/// executor job that generates. The executor makes the provider call and writes the files; the
/// Bit is never logged, returned or stored. The turn's image allowance lives here because every
/// call is its own job.
pub(crate) struct ImageBits {
    profile: Arc<Profile>,
    state: Arc<FlowLikeState>,
    image_model: Option<String>,
    used: Mutex<u32>,
}

impl ImageBits {
    pub(crate) fn new(
        profile: Arc<Profile>,
        state: Arc<FlowLikeState>,
        image_model: Option<String>,
    ) -> Self {
        Self {
            profile,
            state,
            image_model,
            used: Mutex::new(0),
        }
    }

    fn used(&self) -> std::sync::MutexGuard<'_, u32> {
        self.used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn left(&self) -> u32 {
        MAX_IMAGES_PER_TURN.saturating_sub(*self.used())
    }

    /// The Bit and allowance for one job, or what to tell the model when there is none.
    pub(crate) async fn job_image(&self) -> Result<JobImage, String> {
        let bit = resolve_bit(&self.profile, &self.state, self.image_model.as_deref())
            .await
            .map_err(|reason| match reason {
                ImageUnavailable::NoModel => NO_IMAGE_MODEL_MESSAGE.to_string(),
                ImageUnavailable::NotOnThisHost => "This host cannot generate images".to_string(),
                ImageUnavailable::Unusable(why) => format!("The image model cannot be used: {why}"),
            })?;
        let bit = serde_json::to_value(bit).map_err(|error| error.to_string())?;
        Ok(JobImage {
            bit,
            left: self.left(),
        })
    }

    /// Counts the images a finished job saved against the turn.
    pub(crate) fn record(&self, output: &str) {
        let saved = serde_json::from_str::<Value>(output)
            .ok()
            .filter(|value| value["status"] == "ok")
            .and_then(|value| value["saved"].as_array().map(Vec::len))
            .unwrap_or(0);
        let mut used = self.used();
        *used = used.saturating_add(saved as u32);
    }
}
