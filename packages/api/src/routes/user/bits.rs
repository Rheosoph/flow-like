use std::collections::HashMap;

use crate::{
    entity::{sea_orm_active_enums::BitType, user_bit},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::user::ensure_user_exists,
    state::AppState,
    utils::crypto::{decrypt_secret, encrypt_secret},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::{Bit, BitTypes, Metadata};
use flow_like_types::Value;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Provider params that hold credentials. They are stripped from the stored
/// `parameters` JSON and kept AES-encrypted in `secretsEncrypted`.
const SECRET_PARAM_KEYS: [&str; 4] = ["api_key", "service_account_json", "access_token", "headers"];

/// Provider name of local (downloadable) model bits.
const LOCAL_PROVIDER: &str = "Local";

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpsertUserBitBody {
    /// The bit in core `Bit` shape. Secret provider params may be included in
    /// `parameters.provider.params` OR passed via `secrets`; either way they
    /// are stripped and stored encrypted.
    pub bit: Bit,
    /// Secret provider params (e.g. `api_key`). On update, omitting this keeps
    /// the previously stored secrets.
    #[serde(default)]
    pub secrets: Option<HashMap<String, Value>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListUserBitsQuery {
    /// Include decrypted provider secrets in `parameters.provider.params`.
    /// Owner-only, used by the desktop app for local execution.
    #[serde(default)]
    pub include_secrets: bool,
}

#[utoipa::path(
    get,
    path = "/user/bits",
    tag = "user",
    params(
        ("include_secrets" = Option<bool>, Query, description = "Include decrypted provider secrets (for local execution on your own devices)")
    ),
    responses(
        (status = 200, description = "Your private custom model bits", body = Vec<Bit>),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /user/bits", skip(state, user))]
pub async fn list_user_bits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListUserBitsQuery>,
) -> Result<Json<Vec<Bit>>, ApiError> {
    let sub = user.sub()?;

    let models = user_bit::Entity::find()
        .filter(user_bit::Column::UserId.eq(&sub))
        .all(&state.db)
        .await?;

    let bits = models
        .into_iter()
        .map(|model| user_bit_to_core(model, &state, query.include_secrets))
        .collect();

    Ok(Json(bits))
}

#[utoipa::path(
    put,
    path = "/user/bits/{bit_id}",
    tag = "user",
    params(("bit_id" = String, Path, description = "Identifier of the custom bit")),
    request_body = UpsertUserBitBody,
    responses(
        (status = 200, description = "Custom bit created or updated", body = Bit),
        (status = 400, description = "Invalid bit configuration"),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /user/bits/{bit_id}", skip(state, user, body))]
pub async fn upsert_user_bit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bit_id): Path<String>,
    Json(body): Json<UpsertUserBitBody>,
) -> Result<Json<Bit>, ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;

    validate_bit_id(&bit_id)?;
    let mut bit = body.bit;
    bit.id = bit_id.clone();

    let bit_type = validate_user_bit(&bit)?;

    let (public_parameters, mut extracted_secrets) = split_secret_params(bit.parameters.clone());
    if let Some(secrets) = body.secrets {
        for (key, value) in secrets {
            extracted_secrets.insert(key, value);
        }
    }

    let meta = validate_meta(&bit)?;

    let existing = user_bit::Entity::find_by_id(&bit_id)
        .filter(user_bit::Column::UserId.eq(&sub))
        .one(&state.db)
        .await?;

    let secrets_encrypted = if extracted_secrets.is_empty() {
        existing.as_ref().and_then(|e| e.secrets_encrypted.clone())
    } else {
        let json = flow_like_types::json::to_string(&extracted_secrets)
            .map_err(|e| ApiError::bad_request(format!("Invalid secret params: {e}")))?;
        Some(encrypt_secret(&json, &state.encryption_key))
    };

    let now = chrono::Utc::now().naive_utc();
    let model = user_bit::ActiveModel {
        id: Set(bit_id.clone()),
        user_id: Set(sub.clone()),
        r#type: Set(bit_type),
        repository: Set(bit.repository.clone()),
        download_link: Set(bit.download_link.clone()),
        file_name: Set(bit.file_name.clone()),
        hash: Set(if bit.hash.is_empty() {
            None
        } else {
            Some(bit.hash.clone())
        }),
        size: Set(bit.size.map(|s| s as i64)),
        parameters: Set(Some(public_parameters)),
        secrets_encrypted: Set(secrets_encrypted),
        meta: Set(Some(meta)),
        version: Set(bit.version.clone()),
        license: Set(bit.license.clone()),
        created_at: Set(existing.as_ref().map(|e| e.created_at).unwrap_or(now)),
        updated_at: Set(now),
    };

    let saved = if existing.is_some() {
        model.update(&state.db).await?
    } else {
        model.insert(&state.db).await?
    };

    Ok(Json(user_bit_to_core(saved, &state, false)))
}

#[utoipa::path(
    delete,
    path = "/user/bits/{bit_id}",
    tag = "user",
    params(("bit_id" = String, Path, description = "Identifier of the custom bit")),
    responses(
        (status = 200, description = "Custom bit deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Custom bit not found")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /user/bits/{bit_id}", skip(state, user))]
pub async fn delete_user_bit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bit_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;

    let result = user_bit::Entity::delete_many()
        .filter(
            user_bit::Column::Id
                .eq(&bit_id)
                .and(user_bit::Column::UserId.eq(&sub)),
        )
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    Ok(Json(()))
}

/// Loads a user's whole custom-bit library. This is the catalog view — every
/// bit the user has ever configured, so credentials never have to be entered
/// twice — NOT the set active in any one profile.
pub async fn load_custom_bits_for_user(
    state: &AppState,
    sub: &str,
    include_secrets: bool,
) -> Result<Vec<Bit>, ApiError> {
    let models = user_bit::Entity::find()
        .filter(user_bit::Column::UserId.eq(sub))
        .all(&state.db)
        .await?;

    Ok(models
        .into_iter()
        .map(|model| user_bit_to_core(model, state, include_secrets))
        .collect())
}

/// Loads the custom bits a specific profile has activated, hydrated for
/// execution. Membership rides on `Profile.bit_ids` exactly like public bits,
/// so one library can back several profiles with different model line-ups.
/// Secrets are decrypted only when `include_secrets` is set — call that
/// variant only where the result stays inside the trust boundary (copilot
/// request handling, execution dispatch).
pub async fn load_custom_bits_for_profile(
    state: &AppState,
    sub: &str,
    profile_bit_ids: &[String],
    include_secrets: bool,
) -> Result<Vec<Bit>, ApiError> {
    let wanted: std::collections::HashSet<&str> = profile_bit_ids
        .iter()
        .map(|reference| {
            reference
                .rsplit_once(':')
                .map_or(reference.as_str(), |(_, id)| id)
        })
        .collect();

    if wanted.is_empty() {
        return Ok(vec![]);
    }

    let models = user_bit::Entity::find()
        .filter(user_bit::Column::UserId.eq(sub))
        .all(&state.db)
        .await?;

    Ok(models
        .into_iter()
        .filter(|model| wanted.contains(model.id.as_str()))
        .map(|model| user_bit_to_core(model, state, include_secrets))
        .collect())
}

fn validate_bit_id(bit_id: &str) -> Result<(), ApiError> {
    if bit_id.is_empty()
        || bit_id.len() > 64
        || !bit_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request(
            "Bit id must be alphanumeric (plus - and _) and at most 64 characters",
        ));
    }
    Ok(())
}

/// A user bit must be a well-formed LLM/VLM bit with a provider the local
/// model factory can instantiate from per-bit params. Standard provider names
/// ("openai", "azure", …) are rejected because they would resolve against the
/// server's own env credentials; "hosted[:*]" is rejected because the factory
/// overwrites its api_key with the caller's JWT.
fn validate_user_bit(bit: &Bit) -> Result<BitType, ApiError> {
    let bit_type = match bit.bit_type {
        BitTypes::Llm => BitType::Llm,
        BitTypes::Vlm => BitType::Vlm,
        _ => {
            return Err(ApiError::bad_request(
                "Only LLM and VLM custom bits are supported right now",
            ));
        }
    };

    let provider = bit.try_to_provider().ok_or_else(|| {
        ApiError::bad_request(
            "Bit parameters must contain valid LLM/VLM parameters (context_length, provider, model_classification)",
        )
    })?;

    let name = provider.provider_name.trim();
    let is_custom = name.to_ascii_lowercase().starts_with("custom:");
    let is_local = name == LOCAL_PROVIDER;

    if !is_custom && !is_local {
        return Err(ApiError::bad_request(
            "Custom bit provider must be 'custom:<provider>' (remote backend) or 'Local' (downloadable model)",
        ));
    }

    if is_local && bit.download_link.is_none() {
        return Err(ApiError::bad_request(
            "Local model bits require a download link",
        ));
    }

    Ok(bit_type)
}

fn validate_meta(bit: &Bit) -> Result<Value, ApiError> {
    if !bit.meta.contains_key("en") {
        return Err(ApiError::bad_request(
            "Custom bits require English ('en') metadata with a name",
        ));
    }
    flow_like_types::json::to_value(&bit.meta)
        .map_err(|e| ApiError::bad_request(format!("Invalid bit metadata: {e}")))
}

/// Removes secret keys from `parameters.provider.params`, returning the
/// scrubbed parameters and the extracted secrets.
fn split_secret_params(parameters: Value) -> (Value, HashMap<String, Value>) {
    let mut parameters = parameters;
    let mut secrets = HashMap::new();

    if let Some(params) = parameters
        .get_mut("provider")
        .and_then(|p| p.get_mut("params"))
        .and_then(|p| p.as_object_mut())
    {
        for key in SECRET_PARAM_KEYS {
            if let Some(value) = params.remove(key) {
                secrets.insert(key.to_string(), value);
            }
        }
    }

    (parameters, secrets)
}

/// Merges decrypted secrets back into `parameters.provider.params`.
fn merge_secret_params(parameters: &mut Value, secrets: HashMap<String, Value>) {
    if secrets.is_empty() {
        return;
    }
    let Some(provider) = parameters.get_mut("provider") else {
        return;
    };
    if provider.get("params").is_none_or(|p| p.is_null()) {
        if let Some(provider) = provider.as_object_mut() {
            provider.insert(
                "params".to_string(),
                Value::Object(flow_like_types::json::Map::new()),
            );
        }
    }
    if let Some(params) = provider.get_mut("params").and_then(|p| p.as_object_mut()) {
        for (key, value) in secrets {
            params.insert(key, value);
        }
    }
}

pub(crate) fn user_bit_to_core(
    model: user_bit::Model,
    state: &AppState,
    include_secrets: bool,
) -> Bit {
    let mut parameters = model.parameters.unwrap_or_default();

    if include_secrets
        && let Some(encrypted) = model.secrets_encrypted.as_deref()
        && let Some(decrypted) = decrypt_secret(encrypted, &state.encryption_key)
        && let Ok(secrets) = flow_like_types::json::from_str::<HashMap<String, Value>>(&decrypted)
    {
        merge_secret_params(&mut parameters, secrets);
    }

    let meta = model
        .meta
        .and_then(|meta| flow_like_types::json::from_value::<HashMap<String, Metadata>>(meta).ok())
        .unwrap_or_default();

    Bit {
        id: model.id.clone(),
        bit_type: model.r#type.into(),
        meta,
        authors: vec![],
        repository: model.repository,
        download_link: model.download_link,
        file_name: model.file_name,
        hash: model.hash.unwrap_or_else(|| model.id.clone()),
        size: model.size.map(|s| s as u64),
        hub: state.platform_config.domain.clone(),
        parameters,
        version: model.version,
        license: model.license,
        dependencies: vec![],
        dependency_tree_hash: model.id,
        created: model.created_at.and_utc().to_rfc3339(),
        updated: model.updated_at.and_utc().to_rfc3339(),
        model_slug: None,
        model_evaluation: None,
    }
}
