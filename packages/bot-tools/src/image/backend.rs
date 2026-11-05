use std::sync::Mutex;

use async_trait::async_trait;
use flow_like_types::Bytes;
use serde_json::Value;

pub const MAX_IMAGES_PER_CALL: u32 = 4;
pub const MAX_IMAGES_PER_TURN: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageJob {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub count: u32,
    /// `<width>x<height>`.
    pub size: String,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub bytes: Bytes,
    pub mime: String,
    /// Provider, model and the AI-generated mark.
    pub metadata: Value,
}

/// The model a backend generates with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageModelInfo {
    pub name: String,
    /// Whether the prompt leaves the platform for a provider that is not the owner's machine.
    pub external: bool,
}

/// Why a host cannot generate an image for this bot right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageUnavailable {
    /// The host offers no image generation.
    NotOnThisHost,
    /// The bot has no image model and the host has no default one.
    NoModel,
    /// The model is set but cannot be used (missing Bit, refused provider, missing key).
    Unusable(String),
}

/// The images one turn may still generate. A host builds one per turn and shares it with every
/// backend it hands out, so the cap holds across calls.
#[derive(Debug)]
pub struct ImageBudget {
    used: Mutex<u32>,
    per_turn: u32,
}

impl Default for ImageBudget {
    fn default() -> Self {
        Self::new(MAX_IMAGES_PER_TURN)
    }
}

impl ImageBudget {
    pub fn new(per_turn: u32) -> Self {
        Self {
            used: Mutex::new(0),
            per_turn,
        }
    }

    fn used(&self) -> std::sync::MutexGuard<'_, u32> {
        self.used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn left(&self) -> u32 {
        self.per_turn.saturating_sub(*self.used())
    }

    /// Takes `count` images off the turn's allowance, or reports how many are left.
    pub fn reserve(&self, count: u32) -> Result<(), u32> {
        let mut used = self.used();
        match used
            .checked_add(count)
            .filter(|total| *total <= self.per_turn)
        {
            Some(total) => {
                *used = total;
                Ok(())
            }
            None => Err(self.per_turn.saturating_sub(*used)),
        }
    }

    pub fn release(&self, count: u32) {
        let mut used = self.used();
        *used = used.saturating_sub(count);
    }
}

/// The image model of one bot on one host. Hosts build it from the bot's `image_model` or their
/// default image-capable Bit; the provider call, keys and egress rules stay on the host.
#[async_trait]
pub trait ImageBackend: Send + Sync {
    fn model(&self) -> ImageModelInfo;

    fn budget(&self) -> &ImageBudget;

    async fn generate(&self, job: ImageJob) -> flow_like_types::Result<Vec<GeneratedImage>>;
}
