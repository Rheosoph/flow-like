use sea_orm::sea_query::ExprTrait;
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
use flow_like::bit::{Bit, BitTypes, MLX_PROVIDER_NAME, Metadata};
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

    let previous_bit = existing
        .clone()
        .map(|model| user_bit_to_core(model, &state, false));
    bit.normalize_edited_user_local_artifact_identity(previous_bit.as_ref());

    let secrets_encrypted = if extracted_secrets.is_empty() {
        existing.as_ref().and_then(|e| e.secrets_encrypted.clone())
    } else {
        let json = flow_like_types::json::to_string(&extracted_secrets)
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
    let is_mlx = name.eq_ignore_ascii_case(MLX_PROVIDER_NAME);

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
    if is_local && bit.bit_type == BitTypes::Vlm {
        let projection = bit.projection_bit().ok_or_else(|| {
            ApiError::bad_request(
                "Local vision models require a projector: set provider.params.projection with a download_link, file_name, and positive size for the mmproj file",
            )
        })?;
        if projection.size.is_none_or(|size| size == 0) {
            return Err(ApiError::bad_request(
                "Local vision model projector size must be positive",
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
}
