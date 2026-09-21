use chrono::Utc;
use sea_orm::ConnectionTrait;
use serde_json::Value;

use super::{ApiError, sql};

pub async fn enqueue<C: ConnectionTrait>(
    db: &C,
    key: &str,
    effect: &str,
    source_type: &str,
    source_id: &str,
    payload: Value,
) -> Result<(), ApiError> {
    let id = blake3::hash(key.as_bytes()).to_hex().to_string();
    let now = Utc::now().timestamp_millis();
    db.execute_raw(sql(r#"INSERT INTO "PaymentOutbox" (id,"sourceType","sourceId",effect,payload,"nextAttemptAt","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$6,$6) ON CONFLICT DO NOTHING"#,
        vec![id.clone().into(),source_type.into(),source_id.into(),effect.into(),payload.clone().into(),now.into()])).await?;
    let row = db
        .query_one_raw(sql(
            r#"SELECT "sourceType","sourceId",effect,payload FROM "PaymentOutbox" WHERE id=$1"#,
            vec![id.into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if row.try_get::<String>("", "sourceType")? != source_type
        || row.try_get::<String>("", "sourceId")? != source_id
        || row.try_get::<String>("", "effect")? != effect
        || row.try_get::<Value>("", "payload")? != payload
    {
        return Err(ApiError::internal(
            "Payment outbox effect identity was reused for different work",
        ));
    }
    Ok(())
}
