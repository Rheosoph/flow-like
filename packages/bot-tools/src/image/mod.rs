mod backend;
#[cfg(feature = "image")]
mod catalog;

use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::create_id;
use flow_like_types::data_classification::DataClassification;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::files::{guess_mime, handle_for};
use crate::layout::IMAGES_DIR;
use crate::outcome::{ToolImage, ToolOutput};
use crate::spec::{Approval, ToolSpec, schema_of};

pub use backend::{
    GeneratedImage, ImageBackend, ImageBudget, ImageJob, ImageModelInfo, ImageUnavailable,
    MAX_IMAGES_PER_CALL, MAX_IMAGES_PER_TURN,
};
#[cfg(feature = "image")]
pub use catalog::{CatalogImages, hosted_access, resolve_bit};

pub const IMAGE_GENERATE: &str = "image_generate";

pub const NO_IMAGE_MODEL_MESSAGE: &str = "This bot has no image model. Ask the owner to pick one in the bot's Brain settings (Image model).";
const MAX_PROMPT_CHARS: usize = 4000;
const MAX_STYLE_CHARS: usize = 200;
const MAX_NEGATIVE_CHARS: usize = 1000;
const MAX_SHOWN_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// Prompts above this label never go to a provider outside the platform.
const EXTERNAL_CEILING: DataClassification = DataClassification::Internal;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum ImageSize {
    #[default]
    #[serde(rename = "1024x1024")]
    Square,
    #[serde(rename = "1536x1024")]
    Landscape,
    #[serde(rename = "1024x1536")]
    Portrait,
}

impl ImageSize {
    fn as_str(self) -> &'static str {
        match self {
            Self::Square => "1024x1024",
            Self::Landscape => "1536x1024",
            Self::Portrait => "1024x1536",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageGenerateArgs {
    /// What the image shows. Be concrete: subject, setting, composition, light, medium. Up to 4000 characters.
    pub prompt: String,
    /// `1024x1024` (default), `1536x1024` (landscape) or `1024x1536` (portrait).
    #[serde(default)]
    pub size: Option<ImageSize>,
    /// How many variants, 1 to 4 (default 1). A turn can generate 8 images in total.
    #[serde(default)]
    pub count: Option<u32>,
    /// A look to apply, e.g. "watercolor", "photorealistic", "flat vector icon".
    #[serde(default)]
    pub style: Option<String>,
    /// What to keep out of the image. Not every model honours it.
    #[serde(default)]
    pub negative_prompt: Option<String>,
    /// A seed for repeatable results, where the model supports one.
    #[serde(default)]
    pub seed: Option<u64>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: IMAGE_GENERATE,
        description: "Generate up to 4 images from a text prompt with the bot's image model. The images are saved in the workspace under bot/images/, shown in the chat and returned as handles; you also see them. Generation takes a while and may cost money: at most 4 images per call and 8 per turn. The prompt is sent to the model's provider. Generated images are marked as AI-generated; say so when you share them.",
        capability: Some("image_generation"),
        module: "image",
        approval: Approval::None,
        approval_args: &[],
        mutating: true,
        input_schema: schema_of::<ImageGenerateArgs>(),
    }]
}

fn clean(value: Option<String>, max: usize, what: &str) -> ToolResult<Option<String>> {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return Ok(None);
    };
    if value.chars().count() > max {
        return Err(ToolError::invalid(format!(
            "The {what} is longer than {max} characters"
        )));
    }
    Ok((!value.is_empty()).then_some(value))
}

struct Checked {
    job: ImageJob,
}

fn check(args: ImageGenerateArgs) -> ToolResult<Checked> {
    let prompt = args.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(ToolError::invalid("The prompt is empty"));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(ToolError::invalid(format!(
            "The prompt is longer than {MAX_PROMPT_CHARS} characters"
        )));
    }
    let count = args.count.unwrap_or(1);
    if !(1..=MAX_IMAGES_PER_CALL).contains(&count) {
        return Err(ToolError::invalid(format!(
            "count must be between 1 and {MAX_IMAGES_PER_CALL}"
        )));
    }
    let style = clean(args.style, MAX_STYLE_CHARS, "style")?;
    let negative_prompt = clean(args.negative_prompt, MAX_NEGATIVE_CHARS, "negative prompt")?;
    let prompt = match style {
        Some(style) => format!("{prompt}\n\nStyle: {style}"),
        None => prompt,
    };
    Ok(Checked {
        job: ImageJob {
            prompt,
            negative_prompt,
            count,
            size: args.size.unwrap_or_default().as_str().to_string(),
            seed: args.seed,
        },
    })
}

fn extension_of(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn unavailable(reason: ImageUnavailable) -> ToolError {
    match reason {
        ImageUnavailable::NotOnThisHost => {
            ToolError::new(code::NOT_YET, "This host cannot generate images")
        }
        ImageUnavailable::NoModel => ToolError::new(code::NO_IMAGE_MODEL, NO_IMAGE_MODEL_MESSAGE),
        ImageUnavailable::Unusable(why) => ToolError::new(
            code::FAILED,
            format!("The image model cannot be used: {why}"),
        ),
    }
}

fn ensure_may_send(env: &dyn ToolEnv, model: &ImageModelInfo) -> ToolResult<()> {
    if model.external && env.scope().classification > EXTERNAL_CEILING {
        return Err(ToolError::refused(format!(
            "This bot is classified {}; its prompts may not leave the platform for '{}'. Pick a local image model or lower the bot's classification.",
            env.scope().classification.as_str(),
            model.name
        )));
    }
    Ok(())
}

pub async fn image_generate(env: &dyn ToolEnv, args: ImageGenerateArgs) -> ToolResult<ToolOutput> {
    let Checked { job } = check(args)?;
    let workspace = env.scope().workspace.clone().ok_or_else(|| {
        ToolError::new(
            code::NO_WORKSPACE,
            "The bot has no workspace to save images in",
        )
    })?;
    let backend = env.images().await.map_err(unavailable)?;
    let model = backend.model();
    ensure_may_send(env, &model)?;
    let budget = backend.budget();
    let wanted = job.count;
    budget.reserve(wanted).map_err(|left| {
        ToolError::new(
            code::RATE_LIMITED,
            format!(
                "A turn can generate at most {MAX_IMAGES_PER_TURN} images and {left} are left; ask for fewer or answer with what you have"
            ),
        )
    })?;
    let generated = match backend.generate(job.clone()).await {
        Ok(images) => images,
        Err(error) => {
            budget.release(wanted);
            return Err(ToolError::new(
                code::FAILED,
                format!("Image generation with '{}' failed: {error:#}", model.name),
            ));
        }
    };
    let generated: Vec<GeneratedImage> = generated.into_iter().take(wanted as usize).collect();
    budget.release(wanted.saturating_sub(generated.len() as u32));
    if generated.is_empty() {
        return Err(ToolError::new(
            code::FAILED,
            format!("The model '{}' returned no images", model.name),
        ));
    }

    let run = create_id().chars().take(8).collect::<String>();
    let mut output = ToolOutput::new(serde_json::Value::Null);
    let mut files = Vec::new();
    let mut saved: Vec<WorkspaceHandle> = Vec::new();
    let mut hidden = 0usize;
    for (index, image) in generated.into_iter().enumerate() {
        let path = format!(
            "{IMAGES_DIR}/img-{run}-{}.{}",
            index + 1,
            extension_of(&image.mime)
        );
        env.write_workspace(&path, image.bytes.clone(), &image.mime)
            .await
            .map_err(|error| {
                ToolError::new(code::FAILED, format!("Saving '{path}' failed: {error:#}"))
            })?;
        let handle = handle_for(&workspace, &path, &image.bytes, &image.mime);
        let shown = image.bytes.len() <= MAX_SHOWN_IMAGE_BYTES
            && matches!(
                guess_mime(&path).as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            );
        if shown {
            output.images.push(ToolImage {
                label: path.clone(),
                mime: image.mime.clone(),
                data: image.bytes.clone(),
            });
        } else {
            hidden += 1;
        }
        files.push(json!({ "file": handle, "metadata": image.metadata }));
        saved.push(handle);
    }
    if hidden > 0 {
        output.warnings.push(format!(
            "{hidden} image(s) were too large to show you; they are saved as files"
        ));
    }
    output.result = json!({
        "images": files,
        "model": model.name,
        "size": job.size,
        "ai_generated": true,
        "left_this_turn": budget.left(),
    });
    output.saved = saved;
    Ok(output)
}

#[cfg(test)]
mod tests;
