use crate::entity::{app, user};
use aws_lambda_events::event::s3::S3EventRecord;
use aws_lambda_events::event::sqs::SqsEvent;
use aws_lambda_events::s3::S3Event;
use aws_lambda_events::sqs::SqsBatchResponse;
use aws_sdk_dynamodb::Client as DynamoClient;
use lambda_runtime::{tracing, Error, LambdaEvent};
use sea_orm::prelude::*;
use sea_orm::sea_query::Expr;
use sea_orm::{DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

fn decode(key: &str) -> Result<String, Error> {
    let key = key.replace("+", " ");
    urlencoding::decode(&key)
        .map_err(|e| Error::from(format!("Failed to decode key: {}", e)))
        .map(|decoded| decoded.into_owned())
}

pub(crate) async fn function_handler(
    event: LambdaEvent<SqsEvent>,
    dynamo: DynamoClient,
    db: DatabaseConnection,
) -> Result<SqsBatchResponse, Error> {
    let mut batch_item_failures = Vec::new();

    for record in event.payload.records.iter() {
        let body = &record.body;
        let s3_event = body
            .as_ref()
            .ok_or_else(|| Error::from("Record body is missing"))?;

        let s3_event: S3Event = serde_json::from_str(s3_event)
            .map_err(|e| Error::from(format!("Failed to parse SQS message: {}", e)))?;

        let mut successful = true;

        // Process each S3 record in the event
        for s3_record in s3_event.records {
            if let Err(err) = process_s3_event(&s3_record, &dynamo, &db).await {
                tracing::error!("Error processing S3 event: {}", err);
                successful = false;
            }
        }

        if !successful {
            // If processing failed, add to batch item failures
            let message_id = record.message_id.clone().unwrap_or_default();
            tracing::error!("Failed to process S3 event for message ID: {}", &message_id);
            batch_item_failures.push(aws_lambda_events::sqs::BatchItemFailure {
                item_identifier: message_id,
            });
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}

async fn process_s3_event(
    s3_record: &S3EventRecord,
    dynamo: &DynamoClient,
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let event_name = s3_record
        .event_name
        .as_ref()
        .ok_or_else(|| Error::from("Event name is missing"))?;
    let key = s3_record
        .s3
        .object
        .key
        .as_ref()
        .ok_or_else(|| Error::from("Object key is missing"))?;

    let key = decode(key)
        .map_err(|e| Error::from(format!("Failed to decode URL-encoded key {}: {}", key, e)))?;

    let (user_id, app_id) = parse_key_identity(&key)
        .map_err(|e| Error::from(format!("Failed to parse key identity: {}", e)))?;

    let is_deletion = event_name.starts_with("ObjectRemoved");

    let delta = if is_deletion {
        let old_value = delete_dynamo(dynamo, &app_id, &key).await?;
        if old_value == 0 {
            tracing::warn!("No previous size found for key: {}", key);
        }
        -old_value
    } else {
        let size = s3_record
            .s3
            .object
            .size
            .ok_or_else(|| Error::from("Object size is missing"))?;

        let etag = s3_record
            .s3
            .object
            .e_tag
            .as_ref()
            .ok_or_else(|| Error::from("ETag is missing"))?;

        let old_value =
            upsert_dynamo(dynamo, &app_id, &key, user_id.as_deref(), size, etag).await?;
        if old_value == 0 {
            tracing::warn!("No previous size found for key: {}", key);
        }
        size - old_value
    };

    // Update Postgres totals
    if let Err(err) = update_postgres_usage(db, user_id.as_deref(), &app_id, delta).await {
        tracing::error!("Failed to update Postgres usage for key {}: {}", key, err);
        // NOTE: We intentionally do NOT delete the S3 object here.
        // Doing so would trigger another S3 event notification, causing Lambda recursion.
        // Instead, we only revert the DynamoDB state and let the SQS retry mechanism
        // handle the failure. The S3 object remains, and will be reconciled on retry.
        delete_dynamo(dynamo, &app_id, &key).await?;
        return Err(err);
    }
    Ok(())
}

async fn delete_dynamo(dynamo: &DynamoClient, app_id: &str, key: &str) -> Result<i64, Error> {
    let table_name = std::env::var("FILES_TABLE_NAME")
        .map_err(|_| Error::from("FILES_TABLE_NAME environment variable not set"))?;

    let item = dynamo
        .delete_item()
        .table_name(table_name)
        .key(
            "pk",
            aws_sdk_dynamodb::types::AttributeValue::S(app_id.to_string()),
        )
        .key(
            "sk",
            aws_sdk_dynamodb::types::AttributeValue::S(key.to_string()),
        )
        .return_values(aws_sdk_dynamodb::types::ReturnValue::AllOld)
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to delete item from DynamoDB: {}", e)))?;

    if let Some(old_item) = item.attributes {
        if let Some(size_attr) = old_item.get("size") {
            if let Ok(size_str) = size_attr.as_s() {
                return Ok(size_str.parse::<i64>().unwrap_or(0));
            }
        }
    }

    Ok(0)
}

async fn upsert_dynamo(
    dynamo: &DynamoClient,
    app_id: &str,
    key: &str,
    user_id: Option<&str>,
    size: i64,
    etag: &str,
) -> Result<i64, Error> {
    let table_name = std::env::var("FILES_TABLE_NAME")
        .map_err(|_| Error::from("FILES_TABLE_NAME environment variable not set"))?;

    let old_item = dynamo
        .put_item()
        .table_name(table_name)
        .item(
            "pk",
            aws_sdk_dynamodb::types::AttributeValue::S(app_id.to_string()),
        )
        .item(
            "sk",
            aws_sdk_dynamodb::types::AttributeValue::S(key.to_string()),
        )
        .item(
            "size",
            aws_sdk_dynamodb::types::AttributeValue::S(size.to_string()),
        )
        .item(
            "user_id",
            aws_sdk_dynamodb::types::AttributeValue::S(user_id.unwrap_or_default().to_string()),
        )
        .item(
            "etag",
            aws_sdk_dynamodb::types::AttributeValue::S(etag.to_string()),
        )
        .item(
            "updated_at",
            aws_sdk_dynamodb::types::AttributeValue::N(chrono::Utc::now().timestamp().to_string()),
        )
        .return_values(aws_sdk_dynamodb::types::ReturnValue::AllOld)
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to upsert item in DynamoDB: {}", e)))?;

    if let Some(old_item) = old_item.attributes {
        if let Some(size_attr) = old_item.get("size") {
            if let Ok(size_str) = size_attr.as_s() {
                return Ok(size_str.parse::<i64>().unwrap_or(0));
            }
        }
    }

    Ok(0)
}

async fn update_postgres_usage(
    db: &DatabaseConnection,
    user_id: Option<&str>,
    app_id: &str,
    delta: i64,
) -> Result<(), Error> {
    let txn = db
        .begin()
        .await
        .map_err(|e| Error::from(format!("Failed to start transaction: {}", e)))?;

    let update = app::Entity::update_many()
        .col_expr(
            app::Column::TotalSize,
            Expr::col(app::Column::TotalSize).add(delta),
        )
        .filter(app::Column::Id.eq(app_id))
        .exec_with_returning(&txn)
        .await
        .map_err(|e| Error::from(format!("Failed to update usage: {}", e)))?;

    if update.is_empty() {
        return Err(Error::from("Failed to update app usage, app not found"));
    }

    if let Some(user_id) = user_id {
        let update_user = user::Entity::update_many()
            .col_expr(
                user::Column::TotalSize,
                Expr::col(user::Column::TotalSize).add(delta),
            )
            .filter(user::Column::Id.eq(user_id))
            .exec_with_returning(&txn)
            .await
            .map_err(|e| Error::from(format!("Failed to update user usage: {}", e)))?;

        if update_user.is_empty() {
            return Err(Error::from("Failed to update user usage, user not found"));
        }
    }

    txn.commit()
        .await
        .map_err(|e| Error::from(format!("Failed to commit transaction: {}", e)))?;

    Ok(())
}

fn parse_key_identity(key: &str) -> Result<(Option<String>, String), Error> {
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() < 2 {
        return Err("Invalid key format".into());
    }
    if parts[0] == "apps" {
        Ok((None, parts[1].to_string()))
    } else if parts[0] == "users" {
        if parts.len() < 4 {
            return Err("Invalid key format for user".into());
        }
        Ok((Some(parts[1].to_string()), parts[3].to_string()))
    } else {
        Err("Invalid key format".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_key_decoding_1() {
        let key = "media/image+%281%29.jpg";

        let decoded_key = decode(key).unwrap();

        assert_eq!(decoded_key, "media/image (1).jpg");
    }

    #[tokio::test]
    async fn test_key_decoding_2() {
        let key = "media/image%2B1.jpg";

        let decoded_key = decode(key).unwrap();

        assert_eq!(decoded_key, "media/image+1.jpg");
    }

    #[tokio::test]
    async fn test_key_decoding_3() {
        let key = "media/image+%281%29+copy%2B1.jpg";

        let decoded_key = decode(key).unwrap();

        assert_eq!(decoded_key, "media/image (1) copy+1.jpg");
    }
}
