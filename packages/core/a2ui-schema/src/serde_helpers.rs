use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serializer};
use std::time::SystemTime;

/// Serialize SystemTime as ISO8601 string
pub fn serialize_systemtime<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let datetime: DateTime<Utc> = (*time).into();
    serializer.serialize_str(&datetime.to_rfc3339())
}

/// Deserialize SystemTime from either ISO8601 string or SystemTime struct
pub fn deserialize_systemtime<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum TimeFormat {
        IsoString(String),
        SystemTimeStruct {
            secs_since_epoch: u64,
            nanos_since_epoch: u32,
        },
    }

    match TimeFormat::deserialize(deserializer)? {
        TimeFormat::IsoString(s) => {
            let datetime = DateTime::parse_from_rfc3339(&s).map_err(serde::de::Error::custom)?;
            Ok(datetime.with_timezone(&Utc).into())
        }
        TimeFormat::SystemTimeStruct {
            secs_since_epoch,
            nanos_since_epoch,
        } => {
            Ok(std::time::UNIX_EPOCH
                + std::time::Duration::new(secs_since_epoch, nanos_since_epoch))
        }
    }
}
