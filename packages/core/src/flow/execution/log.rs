#[cfg(feature = "flow-runtime")]
use flow_like_storage::{
    arrow_array::RecordBatch,
    lancedb::arrow::arrow_schema::FieldRef,
    serde_arrow::{
        self,
        schema::{SchemaLike, TracingOptions},
    },
};
#[cfg(feature = "flow-runtime")]
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use super::LogLevel;

#[cfg(feature = "flow-runtime")]
static STORED_LOG_MESSAGE_FIELDS: Lazy<Vec<FieldRef>> = Lazy::new(|| {
    Vec::<FieldRef>::from_type::<StoredLogMessage>(
        TracingOptions::default().allow_null_fields(true),
    )
    .expect("derive FieldRef for StoredLogMessage")
});

#[cfg(feature = "flow-runtime")]
pub fn into_arrow<I>(logs: I) -> flow_like_types::Result<RecordBatch>
where
    I: IntoIterator<Item = LogMessage>,
{
    LogMessage::into_arrow(logs)
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LogStat {
    pub token_in: Option<u64>,
    pub token_out: Option<u64>,
    pub bit_ids: Option<Vec<String>>,
}

impl LogStat {
    pub fn new(
        token_in: Option<u64>,
        token_out: Option<u64>,
        bit_ids: Option<Vec<String>>,
    ) -> Self {
        LogStat {
            token_in,
            token_out,
            bit_ids,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LogMessage {
    pub message: String,
    pub operation_id: Option<String>,
    pub node_id: Option<String>,
    pub log_level: LogLevel,
    pub stats: Option<LogStat>,
    pub start: SystemTime,
    pub end: SystemTime,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct StoredLogMessage {
    pub message: String,
    pub operation_id: Option<String>,
    pub node_id: Option<String>,
    pub log_level: u8,
    pub token_in: Option<u64>,
    pub token_out: Option<u64>,
    pub bit_ids: Option<Vec<String>>,
    pub start: u64,
    pub end: u64,
}

impl From<LogMessage> for StoredLogMessage {
    fn from(log: LogMessage) -> Self {
        let log_level = log.log_level.to_u8();
        let token_in = log.stats.as_ref().and_then(|s| s.token_in);
        let token_out = log.stats.as_ref().and_then(|s| s.token_out);
        let bit_ids = log.stats.and_then(|s| s.bit_ids);

        StoredLogMessage {
            message: log.message,
            operation_id: log.operation_id,
            node_id: log.node_id,
            log_level,
            token_in,
            token_out,
            bit_ids,
            start: log
                .start
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64,
            end: log
                .end
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64,
        }
    }
}

impl From<StoredLogMessage> for LogMessage {
    fn from(log: StoredLogMessage) -> Self {
        let log_level = LogLevel::from_u8(log.log_level);
        let token_in = log.token_in;
        let token_out = log.token_out;
        let bit_ids = log.bit_ids;

        LogMessage {
            message: log.message,
            operation_id: log.operation_id,
            node_id: log.node_id,
            log_level,
            stats: Some(LogStat::new(token_in, token_out, bit_ids)),
            start: SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(log.start),
            end: SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(log.end),
        }
    }
}

impl LogMessage {
    pub fn new(message: &str, log_level: LogLevel, operation_id: Option<String>) -> Self {
        let now = SystemTime::now();
        LogMessage {
            message: message.to_string(),
            log_level,
            node_id: None,
            operation_id,
            stats: None,
            start: now,
            end: now,
        }
    }

    pub fn put_stats(&mut self, stats: LogStat) {
        self.stats = Some(stats);
    }

    pub fn end(&mut self) {
        self.end = SystemTime::now();
    }

    #[cfg(feature = "flow-runtime")]
    pub fn into_arrow<I>(logs: I) -> flow_like_types::Result<RecordBatch>
    where
        I: IntoIterator<Item = LogMessage>,
    {
        let stored = logs
            .into_iter()
            .map(StoredLogMessage::from)
            .collect::<Vec<StoredLogMessage>>();

        let fields = &*STORED_LOG_MESSAGE_FIELDS;
        let batch = serde_arrow::to_record_batch(fields, &stored)?;
        Ok(batch)
    }
}
