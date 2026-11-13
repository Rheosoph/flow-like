//! The sign-in of a device bot's own-key image model, for one call. The device's claim holds the
//! model without its provider `params`; while a turn of the bot runs, the bot's runtime token
//! fetches them here for each `image_generate` call, and the device keeps them in memory for
//! that call only. Every hand-out is logged.

use super::{model::BotSettings, owned_bot};
use crate::{
    bot_host::worker, entity::bot, error::ApiError, middleware::jwt::AppUser,
    routes::ai::copilot::master_flow_like_state, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like::bit::Bit;
use flow_like_bot_tools::image::{
    ImageUnavailable, is_own_key_image_bit, resolve_bit, take_provider_params,
};
use flow_like_types::bot::{BotCapability, BotLocation};
use serde::Serialize;
use utoipa::ToSchema;

pub(crate) const IMAGE_MODEL_NO_SIGN_IN: &str = "BOT_IMAGE_MODEL_NO_SIGN_IN";
pub(crate) const IMAGE_MODEL_UNAVAILABLE: &str = "BOT_IMAGE_MODEL_UNAVAILABLE";

#[derive(Debug, Serialize, ToSchema)]
pub struct ImageModelCredential {
    /// The image model's Bit id.
    pub bit_id: String,
    /// The provider `params` of the owner's image model (its key, endpoint and options).
    #[schema(value_type = Object)]
    pub params: serde_json::Value,
}

/// The image model of a bot placed on a device with image generation switched on.
pub(crate) fn device_image_model(row: &bot::Model) -> Result<(String, BotSettings), ApiError> {
    let settings = BotSettings::from_row(row);
    if settings.location != BotLocation::Device {
        return Err(ApiError::forbidden(format!(
            "Bot '{}' does not run on a device, so its image model's sign-in stays on the server",
            row.id
        )));
    }
    if !settings
        .capabilities
        .contains(&BotCapability::ImageGeneration)
    {
        return Err(ApiError::forbidden(format!(
            "Bot '{}' doesn't have image generation switched on",
            row.id
        )));
    }
    let id = settings
        .image_model
        .clone()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ApiError::not_found(format!("Bot '{}' has no image model", row.id)))?;
    Ok((id, settings))
}

/// The provider `params` of an own-key image model; a hosted or local model has none to hand out.
pub(crate) fn own_key_params(mut bit: Bit) -> Result<serde_json::Value, ApiError> {
    if !is_own_key_image_bit(&bit) {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            IMAGE_MODEL_NO_SIGN_IN,
            format!(
                "The image model '{}' is hosted or runs on its owner's machine; it has no sign-in to hand out",
                bit.id
            ),
        ));
    }
    Ok(take_provider_params(&mut bit).unwrap_or_else(|| serde_json::json!({})))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/image-model/credential",
    tag = "Bots",
    description = "The sign-in of the bot's own-key image model for one image generation on its device: the provider settings of the owner's model, key included. Needs the bot's own runtime token, only works while its turn runs, and only for a bot placed on a device.",
    params(("bot_id" = String, Path, description = "The bot's id")),
    responses(
        (status = 200, description = "The image model's provider settings for this call", body = ImageModelCredential),
        (status = 401, description = "Not a runtime token of this bot, or its turn ended"),
        (status = 403, description = "The caller is not the bot's runtime token, the bot is not on a device, or image generation is off"),
        (status = 404, description = "No such bot, or it has no image model"),
        (status = 409, description = "BOT_IMAGE_MODEL_NO_SIGN_IN: the model is hosted or local; BOT_IMAGE_MODEL_UNAVAILABLE: the model cannot be found in the owner's profile")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/image-model/credential", skip(state, user))]
pub async fn bot_image_credential(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<ImageModelCredential>, ApiError> {
    if !matches!(user, AppUser::Bot(_)) {
        return Err(ApiError::forbidden(
            "Only the bot's own runtime token receives its image model's sign-in",
        ));
    }
    let owner = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let row = owned_bot(&state.db, &owner, &bot_id).await?;
    let (image_model, settings) = device_image_model(&row)?;
    let (profile, _) = worker::load_profile(&state, &owner, settings.profile_id.as_deref()).await?;
    let fls = master_flow_like_state(&state).await?;
    let bit = resolve_bit(&profile, &fls, Some(&image_model))
        .await
        .map_err(|error| {
            let why = match error {
                ImageUnavailable::Unusable(why) => why,
                other => format!("{other:?}"),
            };
            ApiError::coded(
                StatusCode::CONFLICT,
                IMAGE_MODEL_UNAVAILABLE,
                format!("The image model '{image_model}' cannot be used: {why}"),
            )
        })?;
    let provider = bit.parameters["provider"]["provider_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let bit_id = bit.id.clone();
    let params = own_key_params(bit)?;
    tracing::info!(
        bot_id = %bot_id,
        bit_id = %bit_id,
        provider = %provider,
        "[bots] handed the image model's sign-in to the bot's device turn"
    );
    Ok(Json(ImageModelCredential { bit_id, params }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::bit::BitTypes;
    use serde_json::json;

    fn image(provider_name: &str) -> Bit {
        Bit {
            id: "img".into(),
            bit_type: BitTypes::ImageGeneration,
            parameters: json!({ "provider": {
                "provider_name": provider_name, "params": { "api_key": "owner-key" },
            }}),
            ..Default::default()
        }
    }

    #[test]
    fn only_an_own_key_model_hands_out_its_sign_in() {
        assert_eq!(
            own_key_params(image("openai")).unwrap()["api_key"],
            "owner-key"
        );
        for provider in [
            "hosted:openrouter",
            flow_like::flow_like_model_provider::stablediffusion::PROVIDER_NAME,
        ] {
            let refused = own_key_params(image(provider)).unwrap_err();
            assert_eq!(refused.status(), StatusCode::CONFLICT, "{provider}");
            assert_eq!(refused.public_code(), IMAGE_MODEL_NO_SIGN_IN);
        }
    }

    #[tokio::test]
    async fn only_a_device_bot_with_image_generation_and_a_model_qualifies() {
        use crate::routes::inbox::fixture::{database, insert_bot};
        use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
        let db = database().await;
        let row = |id: &'static str| {
            let db = &db;
            async move { bot::Entity::find_by_id(id).one(db).await.unwrap().unwrap() }
        };
        let configure = |row: bot::Model, image: Option<&str>, capable: bool| {
            let db = &db;
            let image = image.map(str::to_string);
            async move {
                let mut active: bot::ActiveModel = row.into();
                active.image_model = Set(image);
                let capabilities: Vec<String> = if capable {
                    vec!["image_generation".into()]
                } else {
                    Vec::new()
                };
                active.capabilities = Set(capabilities.into());
                active.update(db).await.unwrap()
            }
        };
        insert_bot(&db, "on-device", "user-a", "DEVICE").await;
        insert_bot(&db, "on-server", "user-a", "SERVER").await;
        let device = configure(row("on-device").await, Some("img"), true).await;
        assert_eq!(device_image_model(&device).unwrap().0, "img");
        let server = configure(row("on-server").await, Some("img"), true).await;
        assert_eq!(
            device_image_model(&server).unwrap_err().status(),
            StatusCode::FORBIDDEN
        );
        let off = configure(device.clone(), Some("img"), false).await;
        assert_eq!(
            device_image_model(&off).unwrap_err().status(),
            StatusCode::FORBIDDEN
        );
        let none = configure(device, None, true).await;
        assert_eq!(
            device_image_model(&none).unwrap_err().status(),
            StatusCode::NOT_FOUND
        );
    }
}
