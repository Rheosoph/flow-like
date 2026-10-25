use crate::accounting::{self, Failure, Observation};
use aws_lambda_events::{
    s3::{S3Event, S3EventRecord},
    sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent, SqsMessage},
};
use aws_sdk_dynamodb::{types::AttributeValue, Client as DynamoClient};
use aws_sdk_s3::Client as S3Client;
use flow_like_db::DbDialect;
use lambda_runtime::{tracing, Error, LambdaEvent};
use sea_orm::{DatabaseConnection, DbErr};
use std::collections::BTreeMap;

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

/// The batch's object observations, each with the index of the message it came from, and the
/// first error of every message that could not be read.
struct Batch {
    observations: Vec<Observation>,
    messages: Vec<usize>,
    errors: BTreeMap<usize, String>,
}

impl Batch {
    fn parse(records: &[SqsMessage]) -> Self {
        let mut batch = Self {
            observations: Vec::new(),
            messages: Vec::new(),
            errors: BTreeMap::new(),
        };
        for (message, record) in records.iter().enumerate() {
            let event = record
                .body
                .as_deref()
                .ok_or_else(|| Error::from("Record body is missing"))
                .and_then(|body| serde_json::from_str::<S3Event>(body).map_err(Error::from));
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    batch.errors.entry(message).or_insert(error.to_string());
                    continue;
                }
            };
            for s3_record in &event.records {
                match observe(s3_record) {
                    Ok(Some(observation)) => {
                        batch.observations.push(observation);
                        batch.messages.push(message);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        batch.errors.entry(message).or_insert(error.to_string());
                    }
                }
            }
        }
        batch
    }

    /// A message fails when any of its records failed, so SQS redelivers only those messages.
    fn failures(
        self,
        records: &[SqsMessage],
        results: Vec<Result<(), Failure>>,
    ) -> Vec<BatchItemFailure> {
        let Self {
            messages,
            mut errors,
            ..
        } = self;
        for (result, message) in results.into_iter().zip(messages) {
            if let Err(failure) = result {
                errors.entry(message).or_insert_with(|| failure.to_string());
            }
        }
        errors
            .into_iter()
            .map(|(message, error)| {
                let message_id = records[message].message_id.clone();
                tracing::error!(%error, ?message_id, "file accounting event failed");
                BatchItemFailure {
                    item_identifier: message_id.unwrap_or_default(),
                }
            })
            .collect()
    }
}

pub(crate) async fn function_handler(
    event: LambdaEvent<SqsEvent>,
    dynamo: DynamoClient,
    s3: S3Client,
    legacy: Option<LegacyBaseline>,
    db: DatabaseConnection,
    dialect: DbDialect,
) -> Result<SqsBatchResponse, Error> {
    let records = event.payload.records;
    let batch = Batch::parse(&records);
    // HEAD runs outside SQL transactions. Accounting checks its object snapshot
    // before committing and samples again if another event changed that snapshot.
    let results = accounting::apply_batch(
        &db,
        dialect,
        &batch.observations,
        |observation| {
            let dynamo = dynamo.clone();
            let legacy = legacy.clone();
            let observation = observation.clone();
            async move {
                read_legacy_size(
                    &dynamo,
                    legacy.as_ref(),
                    &observation.bucket,
                    &observation.app_id,
                    &observation.key,
                )
                .await
                .map_err(|error| {
                    DbErr::Custom(format!(
                        "legacy object size read for {} failed: {error}",
                        observation.key
                    ))
                })
            }
        },
        |observation| {
            head_size(
                s3.clone(),
                observation.bucket.clone(),
                observation.key.clone(),
            )
        },
    )
    .await;
    Ok(SqsBatchResponse {
        batch_item_failures: batch.failures(&records, results),
    })
}

fn observe(event: &S3EventRecord) -> Result<Option<Observation>, Error> {
    let name = event.event_name.as_deref().ok_or("Event name is missing")?;
    if !name.starts_with("ObjectCreated:") && !name.starts_with("ObjectRemoved:") {
        return Ok(None);
    }
    let key = decode(
        event
            .s3
            .object
            .key
            .as_deref()
            .ok_or("Object key is missing")?,
    )?;
    // DeviceArchive metadata accounts for these server-owned payloads separately.
    if key.starts_with("device-archives/") {
        return Ok(None);
    }
    observation(event, key).map(Some)
}

fn observation(event: &S3EventRecord, key: String) -> Result<Observation, Error> {
    let bucket = event
        .s3
        .bucket
        .name
        .clone()
        .ok_or("Object bucket is missing")?;
    let (user_id, app_id) = parse_key_identity(&key)?;
    let sequencer = event
        .s3
        .object
        .sequencer
        .as_deref()
        .ok_or("Object sequencer is missing")?;
    Ok(Observation {
        bucket,
        key,
        app_id,
        user_id,
        sequencer: accounting::normalize_sequencer(sequencer)?,
        legacy_size: 0,
    })
}

async fn head_size(s3: S3Client, bucket: String, key: String) -> Result<i64, DbErr> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        s3.head_object().bucket(&bucket).key(&key).send(),
    )
    .await
    .map_err(|_| DbErr::Custom(format!("S3 HEAD of {key} timed out")))?;
    match result {
        Ok(object) => object
            .content_length()
            .ok_or_else(|| DbErr::Custom(format!("S3 HEAD of {key} returned no content length"))),
        Err(error)
            if error
                .as_service_error()
                .is_some_and(|error| error.is_not_found()) =>
        {
            Ok(0)
        }
        Err(error) => Err(DbErr::Custom(format!("S3 HEAD of {key} failed: {error}"))),
    }
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
    use std::sync::Arc;

    fn record(event_name: &str, key: &str, sequencer: &str) -> S3EventRecord {
        let mut event = S3EventRecord::default();
        event.event_name = Some(event_name.into());
        event.s3.bucket.name = Some("content".into());
        event.s3.object.key = Some(key.into());
        event.s3.object.sequencer = Some(sequencer.into());
        event
    }

    fn message(id: &str, records: Vec<S3EventRecord>) -> SqsMessage {
        SqsMessage {
            message_id: Some(id.into()),
            body: Some(serde_json::to_string(&S3Event { records }).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn archive_notifications_are_acknowledged_without_storage_or_database_access() {
        for event_name in ["ObjectCreated:Put", "ObjectRemoved:Delete"] {
            let mut event = S3EventRecord::default();
            event.event_name = Some(event_name.into());
            event.s3.bucket.name = Some("content".into());
            event.s3.object.key = Some("device-archives%2Fowner%2Farchive.json".into());
            // A missing sequencer would fail accounting.
            assert!(observe(&event).unwrap().is_none());

            event.s3.object.key = Some("device-archives-other/owner/archive.json".into());
            assert!(observe(&event).is_err());
        }
    }

    #[test]
    fn batch_failures_name_only_messages_with_a_failed_record() {
        let records = vec![
            message("valid", vec![record("ObjectCreated:Put", "apps/a/x", "01")]),
            SqsMessage {
                message_id: Some("unreadable".into()),
                body: Some("not json".into()),
                ..Default::default()
            },
            message(
                "archive",
                vec![record("ObjectCreated:Put", "device-archives/owner/a", "1")],
            ),
            message(
                "mixed",
                vec![
                    record("ObjectRemoved:Delete", "apps/b/y", "2"),
                    record("ObjectCreated:Put", "users/u/apps/a/z", "3"),
                ],
            ),
            message(
                "partly-invalid",
                vec![
                    record("ObjectCreated:Put", "apps/c/w", "4"),
                    record("ObjectCreated:Put", "elsewhere/w", "5"),
                ],
            ),
        ];
        let batch = Batch::parse(&records);
        assert_eq!(batch.messages, vec![0, 3, 3, 4]);
        assert_eq!(
            batch
                .observations
                .iter()
                .map(|observation| (observation.app_id.as_str(), observation.sequencer.as_str()))
                .collect::<Vec<_>>(),
            vec![("a", "1"), ("b", "2"), ("a", "3"), ("c", "4")]
        );
        let failed = Arc::new(DbErr::Custom("app b chunk failed".into()));
        let failures = batch.failures(&records, vec![Ok(()), Err(failed), Ok(()), Ok(())]);
        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.item_identifier.as_str())
                .collect::<Vec<_>>(),
            vec!["unreadable", "mixed", "partly-invalid"]
        );
    }

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
