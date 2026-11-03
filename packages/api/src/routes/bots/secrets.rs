//! A bot's secret store. Write-only for people: they list names and metadata, set or replace a
//! value and delete it. No route returns a value, and values never reach a log.

use super::{lock_bot, owned_bot, secret_cipher};
use crate::{
    audit, entity::bot_secret, error::ApiError, middleware::jwt::AppUser,
    routes::profile::deserialize_nullable, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::Value;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const NAME_MAX_LEN: usize = 64;
const MAX_SECRETS_PER_BOT: u64 = 50;
const VALUE_MAX_BYTES: usize = 16 * 1024;
const SEALED_MAX_BYTES: usize = 32 * 1024;
const DESCRIPTION_MAX_CHARS: usize = 2_000;
const BOT_SECRET_LIMIT_REACHED: &str = "BOT_SECRET_LIMIT_REACHED";

/// How a secret is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BotSecretEncryption {
    /// Encrypted by the server under a key only this bot's secrets use.
    Server,
    /// Sealed to the bot's own key; only the bot's host can open it.
    Sealed,
}

impl BotSecretEncryption {
    fn column(self) -> &'static str {
        match self {
            Self::Server => "SERVER",
            Self::Sealed => "SEALED",
        }
    }

    fn from_column(value: &str) -> Option<Self> {
        [Self::Server, Self::Sealed]
            .into_iter()
            .find(|encryption| encryption.column().eq_ignore_ascii_case(value))
    }
}

/// A secret's metadata. No response ever carries its value.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotSecretInfo {
    pub name: String,
    pub encryption: BotSecretEncryption,
    pub description: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
    /// When the bot last read the secret.
    pub last_used_at: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotSecretList {
    /// Sorted by name.
    pub secrets: Vec<BotSecretInfo>,
}

/// Exactly one of `value` or `sealed`.
#[derive(Deserialize, ToSchema)]
pub struct UpsertBotSecretRequest {
    /// The secret, at most 16 KiB. Encrypted before it is stored and never returned.
    pub value: Option<String>,
    /// An envelope sealed to the bot's key: an object with a numeric `v` and an `enc` naming
    /// the encryption, at most 32 KiB.
    #[schema(value_type = Option<Object>)]
    pub sealed: Option<Value>,
    /// Omitted keeps the current description, `null` clears it. At most 2,000 characters.
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
}

type SecretColumns = (
    String,
    String,
    Option<String>,
    DateTime<FixedOffset>,
    DateTime<FixedOffset>,
    Option<DateTime<FixedOffset>>,
);

fn secret_info(
    (name, encryption, description, created_at, updated_at, last_used_at): SecretColumns,
) -> Option<BotSecretInfo> {
    let Some(encryption) = BotSecretEncryption::from_column(&encryption) else {
        tracing::warn!(%name, %encryption, "Skipping bot secret with an unknown encryption");
        return None;
    };
    Some(BotSecretInfo {
        name,
        encryption,
        description,
        created_at,
        updated_at,
        last_used_at,
    })
}

fn stored_info(row: bot_secret::Model) -> Result<BotSecretInfo, ApiError> {
    let (bot_id, name) = (row.bot_id, row.name.clone());
    secret_info((
        row.name,
        row.encryption,
        row.description,
        row.created_at,
        row.updated_at,
        row.last_used_at,
    ))
    .ok_or_else(|| {
        ApiError::internal(format!(
            "Bot secret '{name}' of bot '{bot_id}' was stored with an unknown encryption"
        ))
    })
}

/// One validated write: the ciphertext and, unless omitted, the new description.
#[derive(Clone)]
struct SecretWrite {
    bot_id: String,
    name: String,
    encryption: BotSecretEncryption,
    ciphertext: String,
    description: Option<Option<String>>,
}

async fn replace_secret(
    txn: &DatabaseTransaction,
    existing: bot_secret::Model,
    write: SecretWrite,
) -> Result<bot_secret::Model, DbErr> {
    let mut row: bot_secret::ActiveModel = existing.into();
    row.encryption = Set(write.encryption.column().to_string());
    row.ciphertext = Set(write.ciphertext);
    if let Some(description) = write.description {
        row.description = Set(description);
    }
    row.updated_at = Set(chrono::Utc::now().fixed_offset());
    row.update(txn).await
}

async fn insert_secret(
    txn: &DatabaseTransaction,
    write: SecretWrite,
) -> Result<bot_secret::Model, ApiError> {
    let count = bot_secret::Entity::find()
        .filter(bot_secret::Column::BotId.eq(&write.bot_id))
        .count(txn)
        .await?;
    if count >= MAX_SECRETS_PER_BOT {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_SECRET_LIMIT_REACHED,
            format!(
                "Bot '{}' already has {MAX_SECRETS_PER_BOT} secrets; delete one before adding '{}'",
                write.bot_id, write.name
            ),
        ));
    }
    let now = chrono::Utc::now().fixed_offset();
    let row = bot_secret::ActiveModel {
        bot_id: Set(write.bot_id),
        name: Set(write.name),
        encryption: Set(write.encryption.column().to_string()),
        ciphertext: Set(write.ciphertext),
        description: Set(write.description.flatten()),
        created_at: Set(now),
        updated_at: Set(now),
        last_used_at: Set(None),
    };
    Ok(row.insert(txn).await?)
}

/// Stores or replaces the secret under the bot's lock; true when it is new.
async fn store_secret(
    txn: &DatabaseTransaction,
    owner_id: &str,
    write: SecretWrite,
) -> Result<(bot_secret::Model, bool), ApiError> {
    lock_bot(txn, &write.bot_id).await?;
    owned_bot(txn, owner_id, &write.bot_id).await?;
    let existing = bot_secret::Entity::find_by_id((write.bot_id.clone(), write.name.clone()))
        .one(txn)
        .await?;
    match existing {
        Some(existing) => Ok((replace_secret(txn, existing, write).await?, false)),
        None => Ok((insert_secret(txn, write).await?, true)),
    }
}

/// Metadata of a bot's secrets; the ciphertexts are never selected.
async fn secret_infos<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Vec<BotSecretInfo>, DbErr> {
    let rows = bot_secret::Entity::find()
        .select_only()
        .columns([
            bot_secret::Column::Name,
            bot_secret::Column::Encryption,
            bot_secret::Column::Description,
            bot_secret::Column::CreatedAt,
            bot_secret::Column::UpdatedAt,
            bot_secret::Column::LastUsedAt,
        ])
        .filter(bot_secret::Column::BotId.eq(bot_id))
        .order_by_asc(bot_secret::Column::Name)
        .into_tuple::<SecretColumns>()
        .all(db)
        .await?;
    Ok(rows.into_iter().filter_map(secret_info).collect())
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    let mut bytes = name.bytes();
    let valid = name.len() <= NAME_MAX_LEN
        && bytes.next().is_some_and(|first| first.is_ascii_uppercase())
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    if valid {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "Secret name '{name}' must start with A-Z and continue with up to 63 of A-Z, 0-9 or _"
    )))
}

fn secret_description(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return Ok(None);
    };
    let chars = value.chars().count();
    if chars > DESCRIPTION_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "Secret description has {chars} characters; at most {DESCRIPTION_MAX_CHARS} are allowed"
        )));
    }
    Ok(Some(value).filter(|value| !value.is_empty()))
}

fn sealed_envelope(name: &str, envelope: &Value) -> Result<String, ApiError> {
    let Value::Object(fields) = envelope else {
        return Err(ApiError::bad_request(format!(
            "Sealed secret '{name}' must be a JSON object with 'v' and 'enc'"
        )));
    };
    if !fields.get("v").is_some_and(Value::is_number) {
        return Err(ApiError::bad_request(format!(
            "Sealed secret '{name}' needs a numeric envelope version 'v'"
        )));
    }
    if !fields
        .get("enc")
        .and_then(Value::as_str)
        .is_some_and(|enc| !enc.is_empty() && enc != "none")
    {
        return Err(ApiError::bad_request(format!(
            "Sealed secret '{name}' needs 'enc' naming the encryption it was sealed with"
        )));
    }
    let text = serde_json::to_string(envelope).map_err(|error| {
        ApiError::bad_request(format!("Sealed secret '{name}' is not JSON: {error}"))
    })?;
    if text.len() > SEALED_MAX_BYTES {
        return Err(ApiError::bad_request(format!(
            "Sealed secret '{name}' is {} bytes; at most {SEALED_MAX_BYTES} are allowed",
            text.len()
        )));
    }
    Ok(text)
}

/// What to store for one request: the server ciphertext of `value`, or the sealed envelope.
fn stored_form(
    master: &[u8; 32],
    bot_id: &str,
    name: &str,
    value: Option<String>,
    sealed: Option<Value>,
) -> Result<(BotSecretEncryption, String), ApiError> {
    match (value, sealed) {
        (Some(value), None) => {
            if value.len() > VALUE_MAX_BYTES {
                return Err(ApiError::bad_request(format!(
                    "Secret '{name}' is {} bytes; at most {VALUE_MAX_BYTES} are allowed",
                    value.len()
                )));
            }
            let ciphertext = secret_cipher::encrypt(master, bot_id, name, &value)?;
            Ok((BotSecretEncryption::Server, ciphertext))
        }
        (None, Some(envelope)) => Ok((
            BotSecretEncryption::Sealed,
            sealed_envelope(name, &envelope)?,
        )),
        _ => Err(ApiError::bad_request(format!(
            "Send exactly one of 'value' or 'sealed' for secret '{name}'"
        ))),
    }
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/secrets",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "Names and metadata of the bot's secrets; never their values", body = BotSecretList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/secrets", skip(state, user))]
pub async fn list_bot_secrets(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotSecretList>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    Ok(Json(BotSecretList {
        secrets: secret_infos(&state.db, &bot_id).await?,
    }))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/secrets/{name}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("name" = String, Path, description = "Secret name: A-Z first, then up to 63 of A-Z, 0-9 or _")
    ),
    request_body = UpsertBotSecretRequest,
    responses(
        (status = 200, description = "The secret was stored or replaced; its metadata, without the value", body = BotSecretInfo),
        (status = 400, description = "The name, value, sealed envelope or description is invalid, or both or neither of value and sealed were sent"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_SECRET_LIMIT_REACHED: the bot already has 50 secrets")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/secrets/{name}", skip_all)]
pub async fn upsert_bot_secret(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, name)): Path<(String, String)>,
    Json(body): Json<UpsertBotSecretRequest>,
) -> Result<Json<BotSecretInfo>, ApiError> {
    let sub = user.sub()?;
    validate_name(&name)?;
    let description = body.description.map(secret_description).transpose()?;
    let (encryption, ciphertext) = stored_form(
        &state.encryption_key,
        &bot_id,
        &name,
        body.value,
        body.sealed,
    )?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let write = SecretWrite {
        bot_id: bot_id.clone(),
        name,
        encryption,
        ciphertext,
        description,
    };
    let (stored, created) = state
        .transaction(|txn| {
            let (sub, write) = (sub.clone(), write.clone());
            Box::pin(async move { store_secret(txn, &sub, write).await })
        })
        .await?;
    let info = stored_info(stored)?;
    audit!(
        state,
        user,
        "bot.secret.set",
        "Bot",
        bot_id,
        serde_json::json!({
            "name": info.name,
            "encryption": info.encryption,
            "created": created,
        })
    );
    Ok(Json(info))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/secrets/{name}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("name" = String, Path, description = "Secret name")
    ),
    responses(
        (status = 204, description = "The secret is gone (or never existed)"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/secrets/{name}", skip(state, user))]
pub async fn delete_bot_secret(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let removed = bot_secret::Entity::delete_many()
        .filter(bot_secret::Column::BotId.eq(&bot_id))
        .filter(bot_secret::Column::Name.eq(&name))
        .exec(&state.db)
        .await?;
    if removed.rows_affected > 0 {
        audit!(
            state,
            user,
            "bot.secret.delete",
            "Bot",
            bot_id,
            serde_json::json!({ "name": name })
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KEY: [u8; 32] = [3; 32];

    #[test]
    fn names_are_environment_style() {
        for valid in ["A", "API_KEY", "X9_", &format!("A{}", "B".repeat(63))] {
            assert!(validate_name(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "api_key",
            "_KEY",
            "9KEY",
            "API-KEY",
            "ÄPI",
            &format!("A{}", "B".repeat(64)),
        ] {
            assert!(validate_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn exactly_one_of_value_or_sealed() {
        let sealed = json!({ "v": 1, "enc": "hpke-x25519-sha256-chacha20poly1305", "ct": "AAAA" });
        assert!(stored_form(&KEY, "bot_a", "K", None, None).is_err());
        assert!(stored_form(&KEY, "bot_a", "K", Some("x".into()), Some(sealed.clone())).is_err());

        let (encryption, stored) =
            stored_form(&KEY, "bot_a", "K", Some("sk-123".into()), None).unwrap();
        assert_eq!(encryption, BotSecretEncryption::Server);
        assert!(stored.starts_with("v1:") && !stored.contains("sk-123"));
        assert_eq!(
            secret_cipher::decrypt(&KEY, "bot_a", "K", &stored).unwrap(),
            "sk-123"
        );

        let (encryption, stored) =
            stored_form(&KEY, "bot_a", "K", None, Some(sealed.clone())).unwrap();
        assert_eq!(encryption, BotSecretEncryption::Sealed);
        assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), sealed);
    }

    #[test]
    fn values_and_envelopes_are_bounded() {
        let at_limit = "x".repeat(VALUE_MAX_BYTES);
        assert!(stored_form(&KEY, "bot_a", "K", Some(at_limit), None).is_ok());
        let error = stored_form(
            &KEY,
            "bot_a",
            "K",
            Some("x".repeat(VALUE_MAX_BYTES + 1)),
            None,
        )
        .unwrap_err();
        assert!(error.public_message().unwrap().contains("bytes"));

        let huge = json!({ "v": 1, "enc": "hpke", "ct": "A".repeat(SEALED_MAX_BYTES) });
        assert!(stored_form(&KEY, "bot_a", "K", None, Some(huge)).is_err());
    }

    #[test]
    fn a_sealed_envelope_names_its_version_and_encryption() {
        for bad in [
            json!("ciphertext"),
            json!({ "enc": "hpke" }),
            json!({ "v": "1", "enc": "hpke" }),
            json!({ "v": 1 }),
            json!({ "v": 1, "enc": "" }),
            json!({ "v": 1, "enc": "none", "body": {} }),
        ] {
            assert!(sealed_envelope("K", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn descriptions_are_trimmed_and_bounded() {
        assert_eq!(secret_description(None).unwrap(), None);
        assert_eq!(secret_description(Some("  ".into())).unwrap(), None);
        assert_eq!(
            secret_description(Some(" OpenAI ".into()))
                .unwrap()
                .as_deref(),
            Some("OpenAI")
        );
        assert!(secret_description(Some("d".repeat(DESCRIPTION_MAX_CHARS + 1))).is_err());
    }

    #[test]
    fn metadata_never_carries_a_value_and_reads_stored_encryptions() {
        let now = chrono::Utc::now().fixed_offset();
        let row = |encryption: &str| {
            secret_info(("API_KEY".into(), encryption.into(), None, now, now, None))
        };
        let server = row("SERVER").unwrap();
        assert_eq!(server.encryption, BotSecretEncryption::Server);
        assert_eq!(
            row("SEALED").unwrap().encryption,
            BotSecretEncryption::Sealed
        );
        assert!(row("PLAIN").is_none());

        let wire = serde_json::to_value(server).unwrap();
        let mut keys: Vec<&str> = wire
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "created_at",
                "description",
                "encryption",
                "last_used_at",
                "name",
                "updated_at"
            ]
        );
        assert_eq!(wire["encryption"], "server");
    }

    #[test]
    fn upsert_body_tells_omitted_from_null_descriptions() {
        let omitted: UpsertBotSecretRequest =
            serde_json::from_value(json!({ "value": "x" })).unwrap();
        assert!(omitted.description.is_none());
        let cleared: UpsertBotSecretRequest =
            serde_json::from_value(json!({ "value": "x", "description": null })).unwrap();
        assert_eq!(cleared.description, Some(None));
    }
}
