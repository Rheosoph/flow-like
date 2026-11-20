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
    /// Events inside a Lance dataset become one observation of the dataset directory, stamped
    /// with `dataset_sequencer` so the whole batch is answered by a single measurement.
    fn parse(records: &[SqsMessage], dataset_sequencer: &str) -> Self {
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
                match observe(s3_record, dataset_sequencer) {
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
    let batch = Batch::parse(
        &records,
        &accounting::measurement_sequencer(std::time::SystemTime::now()),
    );
    // HEAD and LIST run outside SQL transactions. Accounting checks its object snapshot
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
            let dynamo = dynamo.clone();
            let legacy = legacy.clone();
            let observation = observation.clone();
            async move {
                read_legacy_files(
                    &dynamo,
                    legacy.as_ref(),
                    &observation.bucket,
                    &observation.app_id,
                    &observation.key,
                )
                .await
                .map_err(|error| {
                    DbErr::Custom(format!(
                        "legacy file sizes under {} could not be read: {error}",
                        observation.key
                    ))
                })
            }
        },
        |observation| {
            let s3 = s3.clone();
            let dataset = observation.is_dataset();
            let bucket = observation.bucket.clone();
            let key = observation.key.clone();
            async move {
                if dataset {
                    list_size(s3, bucket, key).await
                } else {
                    head_size(s3, bucket, key).await
                }
            }
        },
    )
    .await;
    Ok(SqsBatchResponse {
        batch_item_failures: batch.failures(&records, results),
    })
}

fn observe(event: &S3EventRecord, dataset_sequencer: &str) -> Result<Option<Observation>, Error> {
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
    observation(event, key, dataset_sequencer).map(Some)
}

fn observation(
    event: &S3EventRecord,
    key: String,
    dataset_sequencer: &str,
) -> Result<Observation, Error> {
    let bucket = event
        .s3
        .bucket
        .name
        .clone()
        .ok_or("Object bucket is missing")?;
    let (user_id, app_id) = parse_key_identity(&key)?;
    if let Some(dataset) = accounting::dataset_prefix(&key) {
        return Ok(Observation {
            bucket,
            key: dataset.to_owned(),
            app_id,
            user_id,
            sequencer: dataset_sequencer.to_owned(),
            legacy_size: 0,
        });
    }
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

/// Total bytes stored under a dataset directory, including the files of versions Lance has not
/// cleaned up yet: what the bucket bills for.
async fn list_size(s3: S3Client, bucket: String, prefix: String) -> Result<i64, DbErr> {
    let mut pages = s3
        .list_objects_v2()
        .bucket(&bucket)
        .prefix(&prefix)
        .into_paginator()
        .send();
    let mut total = 0_i64;
    loop {
        let page = tokio::time::timeout(std::time::Duration::from_secs(8), pages.next())
            .await
            .map_err(|_| DbErr::Custom(format!("S3 LIST of {prefix} timed out")))?;
        let Some(page) = page else {
            return Ok(total);
        };
        let page =
            page.map_err(|error| DbErr::Custom(format!("S3 LIST of {prefix} failed: {error}")))?;
        for object in page.contents() {
            total = total
                .checked_add(object.size().unwrap_or(0))
                .ok_or_else(|| {
                    DbErr::Custom(format!("S3 LIST of {prefix} overflowed the byte total"))
                })?;
        }
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
    result.item.as_ref().map_or(Ok(0), legacy_item_size)
}

/// Legacy rows of every file under a dataset directory, as (object key, size).
async fn read_legacy_files(
    dynamo: &DynamoClient,
    baseline: Option<&LegacyBaseline>,
    bucket: &str,
    app_id: &str,
    prefix: &str,
) -> Result<Vec<(String, i64)>, Error> {
    let Some(baseline) = baseline.filter(|baseline| baseline.bucket == bucket) else {
        return Ok(Vec::new());
    };
    let mut items = dynamo
        .query()
        .table_name(&baseline.table)
        .key_condition_expression("pk = :app AND begins_with(sk, :prefix)")
        .expression_attribute_values(":app", AttributeValue::S(app_id.to_owned()))
        .expression_attribute_values(":prefix", AttributeValue::S(prefix.to_owned()))
        .consistent_read(true)
        .into_paginator()
        .items()
        .send();
    let mut files = Vec::new();
    while let Some(item) = items.next().await {
        let item = item?;
        let key = match item.get("sk") {
            Some(AttributeValue::S(key)) => key.clone(),
            _ => return Err("legacy object accounting row has no object key".into()),
        };
        files.push((key, legacy_item_size(&item)?));
    }
    Ok(files)
}

fn legacy_item_size(
    item: &std::collections::HashMap<String, AttributeValue>,
) -> Result<i64, Error> {
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
            assert!(observe(&event, "1").unwrap().is_none());

            event.s3.object.key = Some("device-archives-other/owner/archive.json".into());
            assert!(observe(&event, "1").is_err());
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
        let batch = Batch::parse(&records, "ff");
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

    #[test]
    fn dataset_files_become_one_observation_of_their_directory_at_the_batch_sequencer() {
        let table = "users/u/apps/a/storage/db/notes.lance/";
        let records = vec![
            message(
                "commit",
                vec![
                    record("ObjectCreated:Put", &format!("{table}data/f1.lance"), "07"),
                    record(
                        "ObjectCreated:Put",
                        &format!("{table}_versions/2.manifest"),
                        "09",
                    ),
                ],
            ),
            message(
                "cleanup",
                vec![record(
                    "ObjectRemoved:Delete",
                    &format!("{table}data/f0.lance"),
                    "08",
                )],
            ),
            message(
                "file",
                vec![record(
                    "ObjectCreated:Put",
                    "apps/a/storage/report.pdf",
                    "0a",
                )],
            ),
        ];
        let batch = Batch::parse(&records, "18a0");
        assert_eq!(batch.messages, vec![0, 0, 1, 2]);
        let observed = batch
            .observations
            .iter()
            .map(|observation| {
                (
                    observation.key.as_str(),
                    observation.sequencer.as_str(),
                    observation.user_id.as_deref(),
                    observation.is_dataset(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            vec![
                (table, "18a0", Some("u"), true),
                (table, "18a0", Some("u"), true),
                (table, "18a0", Some("u"), true),
                ("apps/a/storage/report.pdf", "a", None, false),
            ]
        );
    }
}
