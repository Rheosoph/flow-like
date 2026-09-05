use crate::accounting::{self, Observation};
use aws_lambda_events::{
    s3::{S3Event, S3EventRecord},
    sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent},
};
use aws_sdk_dynamodb::{types::AttributeValue, Client as DynamoClient};
use aws_sdk_s3::Client as S3Client;
use flow_like_db::DbDialect;
use lambda_runtime::{tracing, Error, LambdaEvent};
use sea_orm::DatabaseConnection;

#[derive(Clone, Debug)]
pub struct LegacyBaseline {
    pub table: String,
    pub bucket: String,
}

impl LegacyBaseline {
    pub fn from_env() -> Result<Option<Self>, String> {
        let table = std::env::var("FILES_TABLE_NAME")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let Some(table) = table else {
            return Ok(None);
        };
        let bucket = std::env::var("FILES_LEGACY_BUCKET_NAME").ok().filter(|s| !s.trim().is_empty())
            .ok_or("FILES_LEGACY_BUCKET_NAME is required with FILES_TABLE_NAME: legacy accounting keys did not include a bucket")?;
        Ok(Some(Self { table, bucket }))
    }
}

fn decode(key: &str) -> Result<String, Error> {
    let key = key.replace('+', " ");
    Ok(urlencoding::decode(&key)?.into_owned())
}

pub(crate) async fn function_handler(
    event: LambdaEvent<SqsEvent>,
    dynamo: DynamoClient,
    s3: S3Client,
    legacy: Option<LegacyBaseline>,
    db: DatabaseConnection,
    dialect: DbDialect,
) -> Result<SqsBatchResponse, Error> {
    let mut batch_item_failures = Vec::new();
    for record in event.payload.records {
        let result = async {
            let body = record.body.as_ref().ok_or("Record body is missing")?;
            let event: S3Event = serde_json::from_str(body)?;
            for s3_record in event.records {
                process_s3_event(&s3_record, &dynamo, &s3, legacy.as_ref(), &db, dialect).await?;
            }
            Ok::<(), Error>(())
        }
        .await;
        if let Err(error) = result {
            tracing::error!(%error, message_id = ?record.message_id, "file accounting event failed");
            batch_item_failures.push(BatchItemFailure {
                item_identifier: record.message_id.unwrap_or_default(),
            });
        }
    }
    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}

async fn process_s3_event(
    event: &S3EventRecord,
    dynamo: &DynamoClient,
    s3: &S3Client,
    legacy: Option<&LegacyBaseline>,
    db: &DatabaseConnection,
    dialect: DbDialect,
) -> Result<(), Error> {
    let name = event.event_name.as_deref().ok_or("Event name is missing")?;
    if !name.starts_with("ObjectCreated:") && !name.starts_with("ObjectRemoved:") {
        return Ok(());
    }
    let bucket = event
        .s3
        .bucket
        .name
        .as_deref()
        .ok_or("Object bucket is missing")?;
    let key = decode(
        event
            .s3
            .object
            .key
            .as_deref()
            .ok_or("Object key is missing")?,
    )?;
    let (user_id, app_id) = parse_key_identity(&key)?;
    let sequencer = accounting::normalize_sequencer(
        event
            .s3
            .object
            .sequencer
            .as_deref()
            .ok_or("Object sequencer is missing")?,
    )?;

    let mut observation = Observation {
        bucket: bucket.to_owned(),
        key,
        app_id,
        user_id,
        sequencer,
        legacy_size: 0,
    };
    if legacy.is_some() && !observation.already_accounted(db).await? {
        observation.legacy_size = read_legacy_size(
            dynamo,
            legacy,
            bucket,
            &observation.app_id,
            &observation.key,
        )
        .await?;
    }
    let bucket = bucket.to_owned();
    let key = observation.key.clone();
    // Sample after acquiring the SQL object write intent, and sample again after an OCC retry.
    // The call is read-only and bounded so this transaction cannot wait indefinitely on S3.
    let s3 = s3.clone();
    accounting::apply_current(db, dialect, observation, move || {
        let s3 = s3.clone();
        let bucket = bucket.clone();
        let key = key.clone();
        async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                s3.head_object().bucket(&bucket).key(&key).send(),
            )
            .await
            .map_err(|_| sea_orm::DbErr::Custom("S3 HEAD timed out".into()))?;
            match result {
                Ok(object) => object.content_length().ok_or_else(|| {
                    sea_orm::DbErr::Custom("S3 HEAD response has no content length".into())
                }),
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|error| error.is_not_found()) =>
                {
                    Ok(0)
                }
                Err(error) => Err(sea_orm::DbErr::Custom(format!("S3 HEAD failed: {error}"))),
            }
        }
    })
    .await?;
    Ok(())
}

async fn read_legacy_size(
    dynamo: &DynamoClient,
    baseline: Option<&LegacyBaseline>,
    bucket: &str,
    app_id: &str,
    key: &str,
) -> Result<i64, Error> {
    let Some(baseline) = baseline.filter(|baseline| baseline.bucket == bucket) else {
        return Ok(0);
    };
    let result = dynamo
        .get_item()
        .table_name(&baseline.table)
        .key("pk", AttributeValue::S(app_id.to_owned()))
        .key("sk", AttributeValue::S(key.to_owned()))
        .consistent_read(true)
        .send()
        .await?;
    let Some(item) = result.item else {
        return Ok(0);
    };
    let size = match item.get("size") {
        Some(AttributeValue::S(value) | AttributeValue::N(value)) => value.parse::<i64>()?,
        _ => return Err("legacy object accounting row has no valid size".into()),
    };
    if size < 0 {
        return Err("legacy object size must not be negative".into());
    }
    Ok(size)
}

fn parse_key_identity(key: &str) -> Result<(Option<String>, String), Error> {
    let parts: Vec<&str> = key.split('/').collect();
    match parts.as_slice() {
        ["apps", app, ..] if !app.is_empty() => Ok((None, (*app).into())),
        ["users", user, "apps", app, ..] if !user.is_empty() && !app.is_empty() => {
            Ok((Some((*user).into()), (*app).into()))
        }
        _ => Err("Invalid object key identity".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_keys_decode_form_encoding_and_keep_literal_plus() {
        assert_eq!(
            decode("media/image+%281%29.jpg").unwrap(),
            "media/image (1).jpg"
        );
        assert_eq!(decode("media/image%2B1.jpg").unwrap(), "media/image+1.jpg");
        assert_eq!(
            decode("media/image+%281%29+copy%2B1.jpg").unwrap(),
            "media/image (1) copy+1.jpg"
        );
    }

    #[test]
    fn object_identity_requires_the_expected_prefix() {
        assert_eq!(
            parse_key_identity("apps/a/file").unwrap(),
            (None, "a".into())
        );
        assert_eq!(
            parse_key_identity("users/u/apps/a/file").unwrap(),
            (Some("u".into()), "a".into())
        );
        assert!(parse_key_identity("users/u/other/a/file").is_err());
        assert!(parse_key_identity("apps//file").is_err());
    }
}
