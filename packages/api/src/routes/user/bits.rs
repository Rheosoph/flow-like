use sea_orm::sea_query::ExprTrait;
use std::collections::HashMap;

use crate::{
    devices::management::admitted_device,
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
    http::StatusCode,
};
use flow_like::bit::{Bit, BitTypes, MLX_PROVIDER_NAME, Metadata};
use flow_like::models::device::DeviceModelTarget;
use flow_like_device_protocol::validate_management_id;
use flow_like_types::Value;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Provider params that hold credentials. They are stripped from the stored
/// `parameters` JSON and kept AES-encrypted in `secretsEncrypted`.
const SECRET_PARAM_KEYS: [&str; 4] = ["api_key", "service_account_json", "access_token", "headers"];

/// Provider name of local (downloadable) model bits.
const LOCAL_PROVIDER: &str = "Local";
const HUGGING_FACE_HOST: &str = "huggingface.co";

fn valid_huggingface_repo_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    if bytes.is_empty() || bytes.len() > 96 {
        return false;
    }
    if !bytes
        .first()
        .is_some_and(|value| value.is_ascii_alphanumeric())
        || !bytes
            .last()
            .is_some_and(|value| value.is_ascii_alphanumeric())
    {
        return false;
    }
    bytes
        .iter()
        .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'-'))
}

fn invalid_huggingface_gguf_url(label: &str, detail: &str) -> ApiError {
    ApiError::bad_request(format!(
        "{label} must be a public HTTPS huggingface.co resolve URL pinned to a full commit SHA: {detail}"
    ))
}

fn validate_huggingface_pinned_gguf_url(value: &str, label: &str) -> Result<(), ApiError> {
    if value.trim() != value {
        return Err(invalid_huggingface_gguf_url(
            label,
            "surrounding whitespace is not allowed",
        ));
    }
    let url = reqwest::Url::parse(value)
        .map_err(|_| invalid_huggingface_gguf_url(label, "invalid URL"))?;
    if url.scheme() != "https"
        || url.host_str() != Some(HUGGING_FACE_HOST)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_huggingface_gguf_url(
            label,
            "credentials, fragments, alternate origins, and non-HTTPS URLs are not allowed",
        ));
    }

    let mut query = url.query_pairs();
    match (query.next(), query.next()) {
        (None, None) => {}
        (Some((key, value)), None) if key == "download" && value == "true" => {}
        _ => {
            return Err(invalid_huggingface_gguf_url(
                label,
                "only the optional download=true query is allowed",
            ));
        }
    }

    let encoded_segments = url
        .path_segments()
        .ok_or_else(|| invalid_huggingface_gguf_url(label, "invalid path"))?
        .collect::<Vec<_>>();
    if encoded_segments.len() < 5
        || encoded_segments.iter().any(|segment| segment.is_empty())
        || encoded_segments[2] != "resolve"
    {
        return Err(invalid_huggingface_gguf_url(
            label,
            "expected /owner/repository/resolve/<commit-sha>/<file>.gguf",
        ));
    }

    let segments = encoded_segments
        .into_iter()
        .map(|segment| {
            let decoded = urlencoding::decode(segment).map_err(|_| {
                invalid_huggingface_gguf_url(label, "path contains invalid escaping")
            })?;
            if decoded.is_empty()
                || matches!(decoded.as_ref(), "." | "..")
                || decoded.contains('/')
                || decoded.contains('\\')
                || decoded.contains('\0')
            {
                return Err(invalid_huggingface_gguf_url(
                    label,
                    "path contains an unsafe component",
                ));
            }
            Ok(decoded.into_owned())
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    if !valid_huggingface_repo_component(&segments[0])
        || !valid_huggingface_repo_component(&segments[1])
        || !(40..=64).contains(&segments[3].len())
        || !segments[3].bytes().all(|value| value.is_ascii_hexdigit())
    {
        return Err(invalid_huggingface_gguf_url(
            label,
            "owner/repository is invalid or revision is not a full hexadecimal commit SHA",
        ));
    }
    if !segments[4..]
        .last()
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".gguf"))
    {
        return Err(invalid_huggingface_gguf_url(
            label,
            "target file must be GGUF",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpsertUserBitBody {
    /// The bit in core `Bit` shape. Secret provider params may be included in
    /// `parameters.provider.params` OR passed via `secrets`; either way they
    /// are stripped and stored encrypted.
    pub bit: Bit,
    /// Secret provider params (e.g. `api_key`). On update, omitted keys keep
    /// their previously stored values.
    #[serde(default)]
    pub secrets: Option<HashMap<String, Value>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListUserBitsQuery {
    /// Include decrypted provider secrets in `parameters.provider.params`.
    /// Owner-only, used by the desktop app for local execution.
    #[serde(default)]
    pub include_secrets: bool,
    /// Include models hosted on your devices; apps that can't use them leave this off.
    #[serde(default)]
    pub device_models: bool,
}

/// Bits of the `device` provider reach only clients that ask for them: an app from before device
/// models treats one as a remote model, so Find Model could pick it and fail instead of falling back.
pub(crate) fn served_to_client(bit: &Bit, device_models: bool) -> bool {
    device_models || !bit.is_device_model()
}

#[utoipa::path(
    get,
    path = "/user/bits",
    tag = "user",
    params(
        ("include_secrets" = Option<bool>, Query, description = "Include decrypted provider secrets (for local execution on your own devices)"),
        ("device_models" = Option<bool>, Query, description = "Include models hosted on your devices; apps that can't use them leave this off")
    ),
    responses(
        (status = 200, description = "Your private custom model bits", body = Vec<Bit>),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /user/bits", skip_all)]
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
        .filter(|bit| served_to_client(bit, query.device_models))
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
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "A device model bit names a device you neither own nor hold a grant on")
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

    let bit_type = checked_user_bit(&state, &sub, &bit, body.secrets.as_ref()).await?;

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

    let previous_bit = existing
        .clone()
        .map(|model| user_bit_to_core(model, &state, false));
    bit.normalize_edited_user_local_artifact_identity(previous_bit.as_ref());

    let secrets_encrypted = if extracted_secrets.is_empty() {
        existing.as_ref().and_then(|e| e.secrets_encrypted.clone())
    } else {
        let previous_secrets = existing
            .as_ref()
            .and_then(|e| e.secrets_encrypted.as_deref())
            .map(|encrypted| {
                let decrypted =
                    decrypt_secret(encrypted, &state.encryption_key).ok_or_else(|| {
                        ApiError::bad_request("Could not read existing model credentials")
                    })?;
                flow_like_types::json::from_str::<HashMap<String, Value>>(&decrypted)
                    .map_err(|_| ApiError::bad_request("Could not read existing model credentials"))
            })
            .transpose()?
            .unwrap_or_default();
        let merged_secrets = merge_edited_secrets(previous_secrets, extracted_secrets);
        let json = flow_like_types::json::to_string(&merged_secrets)
            .map_err(|e| ApiError::bad_request(format!("Invalid secret params: {e}")))?;
        Some(encrypt_secret(&json, &state.encryption_key))
    };

    let now = chrono::Utc::now().fixed_offset();
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
        .filter(user_bit::Column::Id.is_in(wanted))
        .all(&state.db)
        .await?;

    Ok(models
        .into_iter()
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

/// The bit type, and the device a `device` bit names: those go through
/// [`validate_device_model_bit`], every other bit through [`validate_user_bit`].
fn validated_bit(bit: &Bit) -> Result<(BitType, Option<DeviceModelTarget>), ApiError> {
    let Some(target) = bit.device_model_target() else {
        return validate_user_bit(bit).map(|bit_type| (bit_type, None));
    };
    let target = target.map_err(|error| ApiError::bad_request(error.to_string()))?;
    validate_device_model_bit(bit, &target).map(|bit_type| (bit_type, Some(target)))
}

/// A device model bit also needs an account that may use its device, and carries no secrets.
async fn checked_user_bit(
    state: &AppState,
    sub: &str,
    bit: &Bit,
    secrets: Option<&HashMap<String, Value>>,
) -> Result<BitType, ApiError> {
    let (bit_type, device) = validated_bit(bit)?;
    let Some(target) = device else {
        return Ok(bit_type);
    };
    if secrets.is_some_and(|secrets| !secrets.is_empty()) {
        return Err(ApiError::bad_request(format!(
            "Device model bit {} carries no credentials; its device authorizes each call",
            bit.id
        )));
    }
    ensure_device_access(state, sub, &bit.id, &target.device_id).await?;
    Ok(bit_type)
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
        BitTypes::SystemOne => BitType::SystemOne,
        _ => {
            return Err(ApiError::bad_request(
                "Custom bits must be language, vision, or SystemOne decision models",
            ));
        }
    };

    let provider = bit.try_to_provider().ok_or_else(|| {
        ApiError::bad_request("Bit parameters must contain valid model parameters for their type")
    })?;

    let name = provider.provider_name.trim();
    let is_custom = name.to_ascii_lowercase().starts_with("custom:");
    let is_local = name == LOCAL_PROVIDER;
    let is_mlx = name.eq_ignore_ascii_case(MLX_PROVIDER_NAME);

    if bit.bit_type == BitTypes::SystemOne {
        if !is_local
            && !matches!(
                name.to_ascii_lowercase().as_str(),
                "custom:systemone" | "custom:typesafe" | "custom:openrouter"
            )
        {
            return Err(ApiError::bad_request(
                "SystemOne custom bits require Local, custom:systemone, custom:typesafe, or custom:openrouter",
            ));
        }
        if bit.try_to_context_length() == Some(0) {
            return Err(ApiError::bad_request(
                "SystemOne context length must be positive",
            ));
        }
        if !is_local {
            let param = |key: &str| {
                provider
                    .params
                    .as_ref()
                    .and_then(|params| params.get(key))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            };
            if provider
                .model_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .or_else(|| param("model_id"))
                .is_none()
            {
                return Err(ApiError::bad_request(
                    "SystemOne providers require a model ID",
                ));
            }
            let endpoint = param("endpoint");
            if name.eq_ignore_ascii_case("custom:systemone") && endpoint.is_none() {
                return Err(ApiError::bad_request(
                    "Custom SystemOne providers require an endpoint",
                ));
            }
            if let Some(endpoint) = endpoint {
                let valid = reqwest::Url::parse(endpoint).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                });
                if !valid {
                    return Err(ApiError::bad_request(
                        "SystemOne endpoints must be HTTP or HTTPS URLs without credentials, query, or fragment",
                    ));
                }
            }
        }
    }

    if !is_custom && !is_local && !is_mlx {
        return Err(ApiError::bad_request(
            "Custom bit provider must be 'custom:<provider>' (remote backend), 'Local' (GGUF), or 'MLX'",
        ));
    }

    if is_local {
        let download_link = bit
            .download_link
            .as_deref()
            .filter(|link| !link.trim().is_empty())
            .ok_or_else(|| ApiError::bad_request("Local model bits require a download link"))?;
        validate_huggingface_pinned_gguf_url(download_link, "Local model download_link")?;
        if bit
            .file_name
            .as_deref()
            .is_none_or(|file_name| file_name.trim().is_empty())
        {
            return Err(ApiError::bad_request(
                "Local model bits require a file_name",
            ));
        }
        if bit.size.is_none_or(|size| size == 0) {
            return Err(ApiError::bad_request(
                "Local model bits require a positive file size",
            ));
        }
    }

    if is_mlx {
        if bit.download_link.is_some()
            || bit.file_name.is_some()
            || bit.size.is_some_and(|size| size != 0)
        {
            return Err(ApiError::bad_request(
                "User-owned MLX model roots must be virtual (no download_link, file_name, or non-zero size)",
            ));
        }
        if !bit.dependencies.is_empty() {
            return Err(ApiError::bad_request(
                "User-owned MLX model roots must use parameters.huggingface instead of registry dependencies",
            ));
        }
        if provider.params.as_ref().is_some_and(|params| {
            params.contains_key("huggingface") || params.contains_key("assets")
        }) {
            return Err(ApiError::bad_request(
                "MLX source manifests belong at parameters.huggingface, not provider.params",
            ));
        }

        let assets = bit.inline_mlx_asset_bits().map_err(|error| {
            ApiError::bad_request(format!("Invalid user-owned MLX manifest: {error}"))
        })?;
        if assets.is_empty() {
            return Err(ApiError::bad_request(
                "User-owned MLX models require parameters.huggingface",
            ));
        }
    }

    // llama.cpp only sees images when it is started with `--mmproj`, so a local
    // vision model without a projector would advertise vision it cannot do.
    let has_decision_projector = bit.bit_type == BitTypes::SystemOne
        && provider
            .params
            .as_ref()
            .is_some_and(|params| params.contains_key("projection"));
    if is_local && (bit.bit_type == BitTypes::Vlm || has_decision_projector) {
        let projection = bit.projection_bit().ok_or_else(|| {
            ApiError::bad_request(
                "Local model projectors require provider.params.projection with a download_link, file_name, and positive size for the mmproj file",
            )
        })?;
        if projection.size.is_none_or(|size| size == 0) {
            return Err(ApiError::bad_request(
                "Local model projector size must be positive",
            ));
        }
        let projection_url = projection
            .download_link
            .as_deref()
            .expect("projection_bit requires a download link");
        validate_huggingface_pinned_gguf_url(projection_url, "Projector download_link")?;
    }

    Ok(bit_type)
}

/// A `device` bit names a language, vision, embedding, or decision model hosted on a Flow-Like device. Its
/// provider params hold the device and the model. It carries no file, link or credential;
/// whoever runs it unlocks the device on their own computer.
fn validate_device_model_bit(bit: &Bit, target: &DeviceModelTarget) -> Result<BitType, ApiError> {
    if validate_management_id(&target.device_id).is_err() {
        return Err(ApiError::bad_request(format!(
            "Device model bit {} names an invalid device id",
            bit.id
        )));
    }
    let bit_type = device_model_bit_type(bit)?;
    if carries_files(bit) {
        return Err(ApiError::bad_request(format!(
            "Device model bit {} is served by its device and carries no download_link, file_name, size or dependencies",
            bit.id
        )));
    }
    if let Some(key) = secret_param_key(bit) {
        return Err(ApiError::bad_request(format!(
            "Device model bit {} carries no credentials; remove provider.params.{key}",
            bit.id
        )));
    }
    Ok(bit_type)
}

fn device_model_bit_type(bit: &Bit) -> Result<BitType, ApiError> {
    match bit.bit_type {
        BitTypes::Llm => Ok(BitType::Llm),
        BitTypes::Vlm => Ok(BitType::Vlm),
        BitTypes::SystemOne => Ok(BitType::SystemOne),
        BitTypes::Embedding => Ok(BitType::Embedding),
        _ => Err(ApiError::bad_request(format!(
            "Device model bit {} must be a language, vision, embedding, or SystemOne bit",
            bit.id
        ))),
    }
}

fn carries_files(bit: &Bit) -> bool {
    bit.download_link.is_some()
        || bit.file_name.is_some()
        || bit.size.is_some_and(|size| size != 0)
        || !bit.dependencies.is_empty()
}

fn secret_param_key(bit: &Bit) -> Option<&str> {
    let params = bit.parameters.get("provider")?.get("params")?.as_object()?;
    SECRET_PARAM_KEYS
        .into_iter()
        .find(|key| params.contains_key(*key))
}

/// The rule for opening a tunnel to the device: the account owns it or holds a current grant in
/// its latest access rules.
async fn ensure_device_access(
    state: &AppState,
    sub: &str,
    bit_id: &str,
    device_id: &str,
) -> Result<(), ApiError> {
    admitted_device(state, sub, device_id)
        .await
        .map(|_| ())
        .map_err(|error| device_access_error(error, bit_id, device_id))
}

/// A device the account may not use and one that does not exist answer alike.
fn device_access_error(error: ApiError, bit_id: &str, device_id: &str) -> ApiError {
    match error.status() {
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => ApiError::forbidden(format!(
            "Device model bit {bit_id} names device {device_id}, which you neither own nor hold a grant on"
        )),
        _ => error,
    }
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

// Editing one credential keeps other credentials that the client did not receive.
fn merge_edited_secrets(
    mut previous: HashMap<String, Value>,
    edits: HashMap<String, Value>,
) -> HashMap<String, Value> {
    previous.extend(edits);
    previous
}

/// Merges decrypted secrets back into `parameters.provider.params`.
fn merge_secret_params(parameters: &mut Value, secrets: HashMap<String, Value>) {
    if secrets.is_empty() {
        return;
    }
    let Some(provider) = parameters.get_mut("provider") else {
        return;
    };
    if provider.get("params").is_none_or(|p| p.is_null())
        && let Some(provider) = provider.as_object_mut()
    {
        provider.insert(
            "params".to_string(),
            Value::Object(flow_like_types::json::Map::new()),
        );
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

    let mut bit = Bit {
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
        created: model.created_at.to_rfc3339(),
        updated: model.updated_at.to_rfc3339(),
        model_slug: None,
        model_evaluation: None,
    };
    // Migrate legacy `hash == id` rows on read as well as new writes. This is
    // intentionally computed from public source fields, so no database
    // migration or secret material is required.
    bit.normalize_user_local_artifact_identity();
    bit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_one_credential_preserves_the_others() {
        let previous = HashMap::from([
            ("api_key".into(), Value::String("old-key".into())),
            (
                "headers".into(),
                serde_json::json!({"x-project": "project-id"}),
            ),
        ]);
        let edits = HashMap::from([("api_key".into(), Value::String("new-key".into()))]);
        let merged = merge_edited_secrets(previous, edits);
        assert_eq!(merged["api_key"], "new-key");
        assert_eq!(
            merged["headers"],
            serde_json::json!({"x-project": "project-id"})
        );
    }

    fn model_classification() -> Value {
        flow_like_types::json::json!({
            "cost": 0.3,
            "speed": 0.3,
            "reasoning": 0.3,
            "creativity": 0.3,
            "factuality": 0.3,
            "function_calling": 0.3,
            "safety": 0.3,
            "openness": 0.3,
            "multilinguality": 0.3,
            "coding": 0.3,
        })
    }

    fn mlx_manifest(include_processor: bool) -> Value {
        let mut files = vec![
            flow_like_types::json::json!({"path": "config.json", "size": 100}),
            flow_like_types::json::json!({"path": "tokenizer.json", "size": 200}),
            flow_like_types::json::json!({"path": "tokenizer_config.json", "size": 300}),
            flow_like_types::json::json!({"path": "model.safetensors", "size": 4_000}),
        ];
        if include_processor {
            files
                .push(flow_like_types::json::json!({"path": "processor_config.json", "size": 400}));
        }
        flow_like_types::json::json!({
            "schema": 1,
            "repo_id": "owner/model",
            "revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "format": "mlx",
            "files": files,
        })
    }

    fn user_model(bit_type: BitTypes, provider_name: &str) -> Bit {
        Bit {
            bit_type,
            parameters: flow_like_types::json::json!({
                "context_length": 8192,
                "provider": {
                    "provider_name": provider_name,
                    "model_id": "owner/model",
                    "version": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "params": {},
                },
                "model_classification": model_classification(),
            }),
            size: Some(0),
            ..Bit::default()
        }
    }

    #[test]
    fn systemone_user_bits_accept_native_providers_and_reject_chat_or_mlx() {
        for provider in ["custom:systemone", "custom:typesafe", "custom:openrouter"] {
            let mut bit = user_model(BitTypes::SystemOne, provider);
            if provider == "custom:systemone" {
                bit.parameters["provider"]["params"]["endpoint"] =
                    flow_like_types::json::json!("http://localhost:8080/v1");
            }
            bit.parameters
                .as_object_mut()
                .unwrap()
                .remove("model_classification");
            assert_eq!(validate_user_bit(&bit).unwrap(), BitType::SystemOne);
        }
        for provider in [
            "custom:openai",
            "custom:anthropic",
            "MLX",
            "hosted:openrouter",
        ] {
            assert!(validate_user_bit(&user_model(BitTypes::SystemOne, provider)).is_err());
        }
        let mut bit = user_model(BitTypes::SystemOne, LOCAL_PROVIDER);
        bit.download_link = Some(pinned_gguf_url("laya.gguf"));
        bit.file_name = Some("laya.gguf".into());
        bit.size = Some(449_000_000);
        assert_eq!(validate_user_bit(&bit).unwrap(), BitType::SystemOne);
        bit.parameters["context_length"] = flow_like_types::json::json!(0);
        assert!(validate_user_bit(&bit).is_err());
    }

    #[test]
    fn systemone_custom_bits_require_a_model_and_valid_native_endpoint() {
        let mut bit = user_model(BitTypes::SystemOne, "custom:systemone");
        assert!(validate_user_bit(&bit).is_err());
        for endpoint in [
            "file:///tmp/server",
            "https://user:password@example.com/v1",
            "https://example.com/v1?key=value",
        ] {
            bit.parameters["provider"]["params"]["endpoint"] =
                flow_like_types::json::json!(endpoint);
            assert!(validate_user_bit(&bit).is_err());
        }
        bit.parameters["provider"]["params"]["endpoint"] =
            flow_like_types::json::json!("http://localhost:8080/v1");
        assert!(validate_user_bit(&bit).is_ok());
        bit.parameters["provider"]["model_id"] = flow_like_types::json::json!(" ");
        assert!(validate_user_bit(&bit).is_err());
    }

    #[test]
    fn systemone_optional_projectors_require_complete_pinned_artifacts() {
        let mut bit = user_model(BitTypes::SystemOne, LOCAL_PROVIDER);
        bit.download_link = Some(pinned_gguf_url("decision.gguf"));
        bit.file_name = Some("decision.gguf".into());
        bit.size = Some(1_000);
        assert!(validate_user_bit(&bit).is_ok());
        bit.parameters["provider"]["params"]["projection"] = flow_like_types::json::json!({
            "download_link": pinned_gguf_url("mmproj.gguf"),
            "file_name": "mmproj.gguf"
        });
        assert!(validate_user_bit(&bit).is_err());
        bit.parameters["provider"]["params"]["projection"]["size"] =
            flow_like_types::json::json!(500);
        assert!(validate_user_bit(&bit).is_ok());
        bit.parameters["provider"]["params"]["projection"]["download_link"] =
            flow_like_types::json::json!("https://example.com/mmproj.gguf");
        assert!(validate_user_bit(&bit).is_err());
    }

    fn add_huggingface_manifest(bit: &mut Bit, include_processor: bool) {
        bit.parameters
            .as_object_mut()
            .unwrap()
            .insert("huggingface".to_string(), mlx_manifest(include_processor));
    }

    fn pinned_gguf_url(file_name: &str) -> String {
        format!(
            "https://huggingface.co/owner/model/resolve/{}/{file_name}?download=true",
            "a".repeat(40)
        )
    }

    #[test]
    fn accepts_virtual_user_owned_mlx_llm_with_valid_manifest() {
        let mut bit = user_model(BitTypes::Llm, MLX_PROVIDER_NAME);
        add_huggingface_manifest(&mut bit, false);

        assert!(matches!(validate_user_bit(&bit), Ok(BitType::Llm)));
        assert_eq!(bit.inline_mlx_asset_bits().unwrap().len(), 4);
    }

    #[test]
    fn user_owned_mlx_requires_virtual_root_and_top_level_manifest() {
        let mut missing_manifest = user_model(BitTypes::Llm, MLX_PROVIDER_NAME);
        assert!(validate_user_bit(&missing_manifest).is_err());

        add_huggingface_manifest(&mut missing_manifest, false);
        missing_manifest.download_link = Some("https://example.com/model.safetensors".into());
        assert!(validate_user_bit(&missing_manifest).is_err());

        let mut misplaced_manifest = user_model(BitTypes::Llm, MLX_PROVIDER_NAME);
        misplaced_manifest
            .parameters
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .and_then(|provider| provider.get_mut("params"))
            .and_then(Value::as_object_mut)
            .unwrap()
            .insert("huggingface".to_string(), mlx_manifest(false));
        assert!(validate_user_bit(&misplaced_manifest).is_err());
    }

    #[test]
    fn user_owned_mlx_vlm_requires_processor_configuration() {
        let mut missing_processor = user_model(BitTypes::Vlm, MLX_PROVIDER_NAME);
        add_huggingface_manifest(&mut missing_processor, false);
        assert!(validate_user_bit(&missing_processor).is_err());

        add_huggingface_manifest(&mut missing_processor, true);
        assert!(matches!(
            validate_user_bit(&missing_processor),
            Ok(BitType::Vlm)
        ));
    }

    #[test]
    fn local_gguf_requires_pinned_root_and_complete_projection() {
        let mut llm = user_model(BitTypes::Llm, LOCAL_PROVIDER);
        assert!(validate_user_bit(&llm).is_err());
        llm.download_link = Some(pinned_gguf_url("model.gguf"));
        llm.file_name = Some("model.gguf".into());
        llm.size = Some(1_000);
        assert!(matches!(validate_user_bit(&llm), Ok(BitType::Llm)));

        let mut vlm = user_model(BitTypes::Vlm, LOCAL_PROVIDER);
        vlm.download_link = Some(pinned_gguf_url("model.gguf"));
        vlm.file_name = Some("model.gguf".into());
        vlm.size = Some(1_000);
        assert!(validate_user_bit(&vlm).is_err());

        vlm.parameters
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .and_then(|provider| provider.get_mut("params"))
            .and_then(Value::as_object_mut)
            .unwrap()
            .insert(
                "projection".to_string(),
                flow_like_types::json::json!({
                    "download_link": pinned_gguf_url("mmproj-F16.gguf"),
                    "file_name": "mmproj-F16.gguf",
                }),
            );
        assert!(validate_user_bit(&vlm).is_err());

        vlm.parameters["provider"]["params"]["projection"]["size"] =
            flow_like_types::json::json!(500);
        assert!(matches!(validate_user_bit(&vlm), Ok(BitType::Vlm)));
    }

    #[test]
    fn local_gguf_rejects_mutable_external_or_credentialed_urls() {
        for download_link in [
            "https://example.com/owner/model/resolve/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/model.gguf",
            "http://huggingface.co/owner/model/resolve/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/model.gguf",
            "https://user:secret@huggingface.co/owner/model/resolve/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/model.gguf",
            "https://huggingface.co/owner/model/resolve/main/model.gguf",
            "https://huggingface.co/owner/model/blob/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/model.gguf",
            "https://huggingface.co/owner/model/resolve/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/model.gguf?token=secret",
            "https://huggingface.co/owner/model/resolve/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/config.json",
        ] {
            let mut bit = user_model(BitTypes::Llm, LOCAL_PROVIDER);
            bit.download_link = Some(download_link.into());
            bit.file_name = Some("model.gguf".into());
            bit.size = Some(1_000);
            assert!(
                validate_user_bit(&bit).is_err(),
                "unexpectedly accepted {download_link}"
            );
        }
    }

    #[test]
    fn local_gguf_edit_refreshes_server_owned_source_identity() {
        let build = |revision: &str| {
            let mut bit = user_model(BitTypes::Llm, LOCAL_PROVIDER);
            bit.id = "stable-user-model".into();
            bit.hash = bit.id.clone();
            bit.download_link = Some(format!(
                "https://huggingface.co/owner/model/resolve/{revision}/model.gguf"
            ));
            bit.file_name = Some("model.gguf".into());
            bit.size = Some(1_000);
            bit
        };
        let mut first = build(&"a".repeat(40));
        let mut second = build(&"b".repeat(40));
        assert!(validate_user_bit(&first).is_ok());
        assert!(validate_user_bit(&second).is_ok());

        first.normalize_user_local_artifact_identity();
        second.hash = first.hash.clone();
        second.normalize_edited_user_local_artifact_identity(Some(&first));

        assert_eq!(first.id, second.id);
        assert_eq!(first.file_name, second.file_name);
        assert_eq!(first.size, second.size);
        assert_ne!(first.hash, second.hash);
        assert_ne!(first.dependency_tree_hash, second.dependency_tree_hash);
    }

    const DEVICE_ID: &str = "4f1c2a9e-0d3b-4c55-9a7e-2b8f6d1e0c3a";

    fn device_bit(bit_type: BitTypes, params: Value) -> Bit {
        let provider = flow_like_types::json::json!({
            "provider_name": "device",
            "model_id": "qwen3-8b",
            "version": null,
            "params": params,
        });
        let parameters = if bit_type == BitTypes::Embedding {
            flow_like_types::json::json!({
                "languages": [],
                "vector_length": 768,
                "input_length": 512,
                "prefix": {"query": "", "paragraph": ""},
                "pooling": "Mean",
                "provider": provider,
            })
        } else {
            flow_like_types::json::json!({
                "context_length": 8192,
                "provider": provider,
                "model_classification": model_classification(),
            })
        };
        Bit {
            id: "device-bit".into(),
            bit_type,
            parameters,
            ..Bit::default()
        }
    }

    fn device_params(kind: &str) -> Value {
        flow_like_types::json::json!({"device_id": DEVICE_ID, "model": "qwen3-8b", "kind": kind})
    }

    fn rejection(bit: &Bit) -> String {
        let error = validated_bit(bit).expect_err("the bit must be refused");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        error.public_message().unwrap_or_default().to_owned()
    }

    #[test]
    fn device_bits_are_accepted_for_language_vision_embedding_and_systemone() {
        for (bit_type, kind, expected) in [
            (BitTypes::Llm, "chat", BitType::Llm),
            (BitTypes::Vlm, "vision", BitType::Vlm),
            (BitTypes::Embedding, "embedding", BitType::Embedding),
            (BitTypes::SystemOne, "systemone", BitType::SystemOne),
        ] {
            let bit = device_bit(bit_type, device_params(kind));
            let (bit_type, device) = validated_bit(&bit).unwrap();
            assert_eq!(bit_type, expected);
            assert_eq!(device.unwrap().device_id, DEVICE_ID);
        }
        let without_kind = device_bit(
            BitTypes::Embedding,
            flow_like_types::json::json!({"device_id": DEVICE_ID, "model": "embed"}),
        );
        assert_eq!(validated_bit(&without_kind).unwrap().0, BitType::Embedding);

        let mut local = user_model(BitTypes::Llm, MLX_PROVIDER_NAME);
        add_huggingface_manifest(&mut local, false);
        assert!(matches!(validated_bit(&local), Ok((BitType::Llm, None))));
    }

    #[test]
    fn device_bits_name_a_device_and_a_model_and_nothing_else() {
        let unnamed = device_bit(
            BitTypes::Llm,
            flow_like_types::json::json!({"device_id": DEVICE_ID}),
        );
        assert!(rejection(&unnamed).contains("device-bit"));

        let mismatched = device_bit(BitTypes::Llm, device_params("embedding"));
        assert!(rejection(&mismatched).contains("device-bit"));

        let invalid_device = device_bit(
            BitTypes::Llm,
            flow_like_types::json::json!({"device_id": "../other", "model": "qwen3-8b"}),
        );
        assert!(rejection(&invalid_device).contains("invalid device id"));

        let mut with_file = device_bit(BitTypes::Llm, device_params("chat"));
        with_file.download_link = Some(pinned_gguf_url("model.gguf"));
        assert!(rejection(&with_file).contains("served by its device"));

        let with_secret = device_bit(
            BitTypes::Llm,
            flow_like_types::json::json!({
                "device_id": DEVICE_ID,
                "model": "qwen3-8b",
                "api_key": "sk-test",
            }),
        );
        assert!(rejection(&with_secret).contains("provider.params.api_key"));
    }

    #[test]
    fn device_bits_reach_only_clients_that_ask_for_them() {
        let asked = |uri: &str| {
            let uri = uri.parse().expect("a valid request URI");
            Query::<ListUserBitsQuery>::try_from_uri(&uri)
                .expect("a valid query")
                .0
                .device_models
        };
        assert!(!asked("/user/bits?include_secrets=true"));
        assert!(asked("/user/bits?include_secrets=true&device_models=true"));

        let device = device_bit(BitTypes::Llm, device_params("chat"));
        assert!(served_to_client(&device, true));
        assert!(!served_to_client(&device, false));

        let mut local = user_model(BitTypes::Llm, MLX_PROVIDER_NAME);
        add_huggingface_manifest(&mut local, false);
        assert!(served_to_client(&local, false));
    }

    #[test]
    fn other_embedding_providers_stay_refused() {
        let mut bit = device_bit(BitTypes::Embedding, device_params("embedding"));
        bit.parameters["provider"]["provider_name"] = Value::String(LOCAL_PROVIDER.into());
        assert!(rejection(&bit).contains("Custom bits must be language, vision, or SystemOne"));
    }

    #[test]
    fn devices_the_account_may_not_use_answer_like_missing_ones() {
        for status in [ApiError::FORBIDDEN, ApiError::NOT_FOUND] {
            let error = device_access_error(status, "device-bit", DEVICE_ID);
            assert_eq!(error.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                error.public_message(),
                Some(
                    "Device model bit device-bit names device 4f1c2a9e-0d3b-4c55-9a7e-2b8f6d1e0c3a, which you neither own nor hold a grant on"
                )
            );
        }
        let unavailable = device_access_error(
            ApiError::service_unavailable("Standalone device enrollment is not enabled"),
            "device-bit",
            DEVICE_ID,
        );
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
