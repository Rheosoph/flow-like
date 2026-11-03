use super::*;
use crate::entity::{sea_orm_active_enums::BitType, user_bit};
use sea_orm::{
    ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, Schema, sea_query::Table,
};
use serde_json::json;

fn image_bit() -> Bit {
    Bit {
        id: "official-image".into(),
        bit_type: BitTypes::ImageGeneration,
        parameters: json!({"provider":{"provider_name":"hosted:openrouter","model_id":"google/image-model",
            "params":{"tier":"FREE"}},"pricing":{"request_micro_usd":100_000}}),
        ..Default::default()
    }
}

#[test]
fn catalog_requires_an_image_provider_without_embedded_credentials_or_files() {
    let bit = image_bit();
    assert!(catalog_provider(&bit).is_ok());
    let mut invalid = bit.clone();
    invalid.bit_type = BitTypes::Llm;
    assert!(catalog_provider(&invalid).is_err());
    for provider in [
        "custom:openrouter",
        "hosted:openai",
        "Premium",
        "openrouter",
    ] {
        let mut invalid = bit.clone();
        invalid.parameters["provider"]["provider_name"] = json!(provider);
        assert!(catalog_provider(&invalid).is_err());
    }
    for field in ["endpoint", "api_key", "headers", "extra_body"] {
        let mut invalid = bit.clone();
        invalid.parameters["provider"]["params"][field] = json!("injected");
        assert!(catalog_provider(&invalid).is_err());
    }
    for model in [
        "",
        "../other",
        "model?query=true",
        "model#fragment",
        "https://other.invalid",
    ] {
        let mut invalid = bit.clone();
        invalid.parameters["provider"]["model_id"] = json!(model);
        assert!(catalog_provider(&invalid).is_err());
    }
    let mut invalid = bit.clone();
    invalid.dependencies.push("weight".into());
    assert!(catalog_provider(&invalid).is_err());
    let mut invalid = bit.clone();
    invalid.size = Some(100);
    assert!(catalog_provider(&invalid).is_err());
    let mut invalid = bit.clone();
    invalid.parameters["provider"]["params"]["tier"] = json!("  ");
    assert!(catalog_provider(&invalid).is_err());
    let mut invalid = bit;
    invalid.parameters["provider"]["api_surface"] = json!("Responses");
    assert!(catalog_provider(&invalid).is_err());
}

#[test]
fn image_price_is_explicit_and_has_no_token_tariff() {
    let mut bit = image_bit();
    let rate = crate::bit_pricing::hosted_image_rate(&bit).unwrap();
    assert_eq!(rate.provider_cost(100, 100), 100_000);
    for value in [
        json!(null),
        json!({}),
        json!({"request_micro_usd":-1}),
        json!({"request_micro_usd":0.1}),
        json!({"request_micro_usd":"10"}),
        json!({"request_micro_usd":10,"input_micro_usd_per_million_tokens":1}),
    ] {
        bit.parameters["pricing"] = value;
        assert!(crate::bit_pricing::hosted_image_rate(&bit).is_err());
    }
    bit.parameters["pricing"] = json!({"request_micro_usd":0});
    assert_eq!(
        crate::bit_pricing::hosted_image_rate(&bit)
            .unwrap()
            .request_micro_usd,
        0
    );
}

#[tokio::test]
async fn private_bits_never_authorize_server_credentials() {
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
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    let catalog = image_bit();
    let now = chrono::Utc::now().fixed_offset();
    user_bit::Entity::insert(user_bit::ActiveModel {
        id: Set(catalog.id.clone()),
        user_id: Set("caller".into()),
        r#type: Set(BitType::ImageGeneration),
        parameters: Set(Some(catalog.parameters.clone())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec(&db)
    .await
    .unwrap();
    assert!(lookup_catalog(&db, &catalog.id).await.is_err());
    bit::Entity::insert(bit::ActiveModel {
        id: Set(catalog.id.clone()),
        hub: Set("catalog.example".into()),
        r#type: Set(BitType::ImageGeneration),
        parameters: Set(Some(catalog.parameters.clone())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec(&db)
    .await
    .unwrap();
    assert!(catalog_provider(&lookup_catalog(&db, &catalog.id).await.unwrap()).is_ok());
    bit::Entity::delete_by_id(&catalog.id)
        .exec(&db)
        .await
        .unwrap();
    assert!(lookup_catalog(&db, &catalog.id).await.is_err());
}
