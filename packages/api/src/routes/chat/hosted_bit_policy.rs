use crate::{entity::bit, error::ApiError};
use flow_like::{bit::Bit, flow_like_model_provider::provider::ModelApiSurface};
use sea_orm::{DatabaseConnection, EntityTrait};

pub(super) async fn authorize_systemone(
    db: &DatabaseConnection,
    bit_id: &str,
    model_id: &str,
    provider: &super::relay::HostedProvider,
    body: &serde_json::Value,
) -> Result<(), ApiError> {
    let bit = bit::Entity::find_by_id(bit_id)
        .one(db)
        .await?
        .map(Bit::from);
    validate_systemone_catalog(bit.as_ref(), model_id, provider)?;
    super::systemone::validate_payload(body)?;
    if body.get("model").and_then(serde_json::Value::as_str) != Some(model_id) {
        return Err(ApiError::bad_request(
            "System One request model must match the official Bit",
        ));
    }
    Ok(())
}

fn validate_systemone_catalog(
    bit: Option<&Bit>,
    model_id: &str,
    provider: &super::relay::HostedProvider,
) -> Result<(), ApiError> {
    let bit = bit
        .filter(|bit| bit.bit_type == flow_like::bit::BitTypes::SystemOne)
        .ok_or_else(|| ApiError::forbidden("System One requires an official SystemOne Bit"))?;
    let approved = bit
        .try_to_provider()
        .ok_or_else(|| ApiError::forbidden("System One Bit has no provider"))?;
    if super::relay::HostedProvider::from_provider_name(&approved.provider_name).as_ref()
        != Some(provider)
        || !super::relay::supports_surface(provider, ModelApiSurface::SystemOne)
        || approved
            .model_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty() || id != model_id)
    {
        return Err(ApiError::forbidden(
            "The official Bit no longer authorizes this System One request",
        ));
    }
    Ok(())
}

/// Server IAM credentials are available only to the administrator's Bit catalog.
/// UserBit rows and caller-supplied Bit metadata never establish this authority.
pub(super) async fn authorize_bedrock_iam(
    db: &DatabaseConnection,
    bit_id: &str,
    model_id: &str,
    surface: ModelApiSurface,
) -> Result<(), ApiError> {
    let catalog_bit = bit::Entity::find_by_id(bit_id)
        .one(db)
        .await?
        .map(Bit::from);
    validate_catalog_bit(catalog_bit.as_ref(), model_id, surface)
}

pub(super) fn validate_bedrock_iam_body(
    body: &serde_json::Value,
    model_id: &str,
) -> Result<(), ApiError> {
    let body = body
        .as_object()
        .ok_or_else(|| ApiError::bad_request("Expected a request object"))?;
    if body.get("model").and_then(serde_json::Value::as_str) != Some(model_id) {
        return Err(ApiError::bad_request(
            "Bedrock IAM request model must match the official Bit",
        ));
    }
    // Transport and routing belong to the server. Inspect only top-level fields;
    // the same names inside conversation content or tool schemas are ordinary data.
    if [
        "extra_body",
        "extra_headers",
        "extra_query",
        "endpoint",
        "base_url",
        "api_key",
        "authorization",
        "headers",
        "provider",
        "modelId",
        "inferenceProfileArn",
        "region",
        "models",
        "route",
    ]
    .iter()
    .any(|field| body.contains_key(*field))
    {
        return Err(ApiError::bad_request(
            "Bedrock IAM requests cannot override provider routing or authentication",
        ));
    }
    Ok(())
}

fn validate_catalog_bit(
    catalog_bit: Option<&Bit>,
    model_id: &str,
    surface: ModelApiSurface,
) -> Result<(), ApiError> {
    let provider = catalog_bit
        .and_then(Bit::try_to_provider)
        .ok_or_else(|| ApiError::forbidden("Bedrock IAM requires an official hosted model Bit"))?;
    if !provider
        .provider_name
        .trim()
        .eq_ignore_ascii_case("hosted:bedrock")
        || provider.api_surface_or_default() != surface
        || provider.model_id.as_deref().is_none_or(|approved_model| {
            approved_model.trim().is_empty() || approved_model != model_id
        })
    {
        return Err(ApiError::forbidden(
            "The official Bit no longer authorizes this Bedrock IAM request",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{sea_orm_active_enums::BitType, user_bit};
    use axum::http::StatusCode;
    use flow_like::bit::{BitModelClassification, BitTypes};
    use sea_orm::{
        ActiveValue::Set, ColumnTrait, ConnectOptions, ConnectionTrait, Database, QueryFilter,
        Schema, sea_query::Table,
    };
    use serde_json::json;

    const MODEL: &str = "openai.gpt-oss-120b-1:0";

    fn catalog_bit() -> Bit {
        Bit {
            id: "official-bedrock".into(),
            bit_type: BitTypes::Llm,
            parameters: json!({
                "context_length": 32_768,
                "model_classification": BitModelClassification::default(),
                "provider": {
                    "provider_name": "hosted:bedrock",
                    "model_id": MODEL,
                },
            }),
            ..Default::default()
        }
    }

    fn assert_forbidden(result: Result<(), ApiError>) {
        assert_eq!(result.unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(bit::Entity),
            schema.create_table_from_entity(user_bit::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            // The catalog boundary needs these entities' real columns, without
            // foreign keys into the rest of the application database.
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db
    }

    #[tokio::test]
    async fn only_the_current_official_catalog_row_authorizes_iam() {
        let db = database().await;
        let mut catalog = catalog_bit();
        let id = catalog.id.clone();
        let chat = ModelApiSurface::ChatCompletions;
        assert_forbidden(authorize_bedrock_iam(&db, &id, MODEL, chat).await);

        let now = chrono::Utc::now().fixed_offset();
        // Even a forged hosted provider in UserBit cannot grant the server role.
        user_bit::Entity::insert(user_bit::ActiveModel {
            id: Set(id.clone()),
            user_id: Set("caller".into()),
            r#type: Set(BitType::Llm),
            parameters: Set(Some(catalog.parameters.clone())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        assert_forbidden(authorize_bedrock_iam(&db, &id, MODEL, chat).await);

        bit::Entity::insert(bit::ActiveModel {
            id: Set(id.clone()),
            hub: Set("catalog.example".into()),
            r#type: Set(BitType::Llm),
            parameters: Set(Some(catalog.parameters.clone())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        authorize_bedrock_iam(&db, &id, MODEL, chat).await.unwrap();

        let mut previous_model = MODEL;
        let mut previous_surface = chat;
        for (field, value) in [
            ("model_id", json!("replacement-model")),
            ("api_surface", json!(ModelApiSurface::Responses)),
            ("provider_name", json!("custom:bedrock")),
        ] {
            catalog.parameters["provider"][field] = value;
            bit::Entity::update_many()
                .set(bit::ActiveModel {
                    parameters: Set(Some(catalog.parameters.clone())),
                    ..Default::default()
                })
                .filter(bit::Column::Id.eq(&id))
                .exec(&db)
                .await
                .unwrap();
            assert_forbidden(
                authorize_bedrock_iam(&db, &id, previous_model, previous_surface).await,
            );
            let current_surface = if field == "model_id" {
                chat
            } else {
                ModelApiSurface::Responses
            };
            let current =
                authorize_bedrock_iam(&db, &id, "replacement-model", current_surface).await;
            if field == "provider_name" {
                assert_forbidden(current);
            } else {
                current.unwrap();
            }
            previous_model = "replacement-model";
            previous_surface = current_surface;
        }

        bit::Entity::update_many()
            .set(bit::ActiveModel {
                parameters: Set(Some(catalog_bit().parameters)),
                ..Default::default()
            })
            .filter(bit::Column::Id.eq(&id))
            .exec(&db)
            .await
            .unwrap();
        authorize_bedrock_iam(&db, &id, MODEL, chat).await.unwrap();
        bit::Entity::delete_by_id(&id).exec(&db).await.unwrap();
        assert_forbidden(authorize_bedrock_iam(&db, &id, MODEL, chat).await);
        assert!(
            user_bit::Entity::find_by_id(&id)
                .one(&db)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn systemone_credentials_require_matching_official_decision_bit() {
        use super::super::relay::HostedProvider;
        let mut bit = catalog_bit();
        bit.bit_type = BitTypes::SystemOne;
        bit.parameters["provider"]["provider_name"] = json!("hosted:openrouter");
        assert!(validate_systemone_catalog(Some(&bit), MODEL, &HostedProvider::OpenRouter).is_ok());
        assert_forbidden(validate_systemone_catalog(
            None,
            MODEL,
            &HostedProvider::OpenRouter,
        ));
        assert_forbidden(validate_systemone_catalog(
            Some(&bit),
            "other",
            &HostedProvider::OpenRouter,
        ));
        assert_forbidden(validate_systemone_catalog(
            Some(&bit),
            MODEL,
            &HostedProvider::TypeSafe,
        ));
        bit.bit_type = BitTypes::Llm;
        assert_forbidden(validate_systemone_catalog(
            Some(&bit),
            MODEL,
            &HostedProvider::OpenRouter,
        ));
    }

    #[test]
    fn authorizes_the_catalog_model_and_declared_surface() {
        for bit_type in [BitTypes::Llm, BitTypes::Vlm] {
            for surface in [ModelApiSurface::ChatCompletions, ModelApiSurface::Responses] {
                let mut bit = catalog_bit();
                bit.bit_type = bit_type.clone();
                bit.parameters["provider"]["provider_name"] = json!(" Hosted:Bedrock ");
                bit.parameters["provider"]["api_surface"] = json!(surface);
                validate_catalog_bit(Some(&bit), MODEL, surface).unwrap();
            }
        }
    }

    #[test]
    fn missing_or_non_hosted_catalog_models_cannot_use_iam() {
        let surface = ModelApiSurface::ChatCompletions;
        assert_forbidden(validate_catalog_bit(None, MODEL, surface));
        for name in [
            "custom:bedrock",
            "bedrock",
            "hosted:openrouter",
            "hosted",
            "hosted:bedrock:other",
        ] {
            let mut bit = catalog_bit();
            bit.parameters["provider"]["provider_name"] = json!(name);
            assert_forbidden(validate_catalog_bit(Some(&bit), MODEL, surface));
        }
    }

    #[test]
    fn changed_catalog_routing_revokes_queued_requests() {
        let mut bit = catalog_bit();
        assert_forbidden(validate_catalog_bit(
            Some(&bit),
            "unapproved-model",
            ModelApiSurface::ChatCompletions,
        ));
        bit.parameters["provider"]["api_surface"] = json!("Responses");
        assert_forbidden(validate_catalog_bit(
            Some(&bit),
            MODEL,
            ModelApiSurface::ChatCompletions,
        ));
    }

    #[test]
    fn explicit_catalog_model_is_required() {
        for model in [json!(null), json!(""), json!(" \t ")] {
            let mut bit = catalog_bit();
            bit.parameters["provider"]["model_id"] = model;
            assert_forbidden(validate_catalog_bit(
                Some(&bit),
                &bit.id,
                ModelApiSurface::ChatCompletions,
            ));
        }
        let mut bit = catalog_bit();
        bit.parameters = json!({"provider": {"provider_name": "hosted:bedrock"}});
        assert_forbidden(validate_catalog_bit(
            Some(&bit),
            MODEL,
            ModelApiSurface::ChatCompletions,
        ));
    }

    #[test]
    fn iam_body_keeps_conversation_and_tool_data() {
        let body = json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "Explain extra_headers and region"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "provider",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "api_key": {"type": "string"},
                            "region": {"type": "string"}
                        }
                    }
                }
            }],
            "temperature": 0.5,
            "max_completion_tokens": 1024,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        validate_bedrock_iam_body(&body, MODEL).unwrap();
    }

    #[test]
    fn iam_body_cannot_change_the_authorized_model() {
        for body in [
            json!(null),
            json!({}),
            json!({"model": null}),
            json!({"model": "unapproved-model"}),
            json!({"model": [MODEL]}),
        ] {
            assert_eq!(
                validate_bedrock_iam_body(&body, MODEL)
                    .unwrap_err()
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn iam_body_rejects_transport_and_routing_overrides() {
        for field in [
            "extra_body",
            "extra_headers",
            "extra_query",
            "endpoint",
            "base_url",
            "api_key",
            "authorization",
            "headers",
            "provider",
            "modelId",
            "inferenceProfileArn",
            "region",
            "models",
            "route",
        ] {
            let mut body = json!({"model": MODEL, "messages": []});
            body[field] = json!({"model": "unapproved-model"});
            assert_eq!(
                validate_bedrock_iam_body(&body, MODEL)
                    .unwrap_err()
                    .status(),
                StatusCode::BAD_REQUEST,
                "field {field} must be rejected"
            );
        }
    }
}
