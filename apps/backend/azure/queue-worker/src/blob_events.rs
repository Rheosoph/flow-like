//! Event Grid blob events as delivered into a Storage Queue.
//!
//! The subscriptions use the CloudEvents 1.0 schema, but the Event Grid schema
//! is accepted as well so a subscription created by hand still works. Only the
//! fields the workers act on are modelled; everything else is ignored.

use serde::Deserialize;

pub const BLOB_CREATED: &str = "Microsoft.Storage.BlobCreated";
pub const BLOB_DELETED: &str = "Microsoft.Storage.BlobDeleted";

/// One blob event, normalised across the two delivery schemas.
#[derive(Debug, Clone)]
pub struct BlobEvent {
    pub id: String,
    pub event_type: String,
    /// `/blobServices/default/containers/<container>/blobs/<key>`
    pub subject: String,
    pub content_length: Option<i64>,
    pub etag: Option<String>,
    /// Per-blob monotonic ordering token; compare lexicographically.
    pub sequencer: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireEvent {
    id: String,
    #[serde(rename = "type")]
    cloud_event_type: Option<String>,
    #[serde(rename = "eventType")]
    event_grid_type: Option<String>,
    subject: String,
    #[serde(default)]
    data: WireData,
}

#[derive(Debug, Default, Deserialize)]
struct WireData {
    #[serde(rename = "contentLength")]
    content_length: Option<i64>,
    #[serde(rename = "eTag")]
    etag: Option<String>,
    sequencer: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlobEventError {
    Malformed,
    UnsupportedType,
}

impl BlobEvent {
    pub fn parse(bytes: &[u8]) -> Result<Self, BlobEventError> {
        let wire =
            serde_json::from_slice::<WireEvent>(bytes).map_err(|_| BlobEventError::Malformed)?;
        let event_type = wire
            .cloud_event_type
            .or(wire.event_grid_type)
            .ok_or(BlobEventError::Malformed)?;
        if event_type != BLOB_CREATED && event_type != BLOB_DELETED {
            return Err(BlobEventError::UnsupportedType);
        }
        Ok(Self {
            id: wire.id,
            event_type,
            subject: wire.subject,
            content_length: wire.data.content_length,
            etag: wire.data.etag,
            sequencer: wire.data.sequencer,
        })
    }

    pub fn is_deletion(&self) -> bool {
        self.event_type == BLOB_DELETED
    }

    /// Split the subject into `(container, blob key)`. Blob keys are not
    /// percent-encoded in Event Grid subjects, so no decoding step exists here
    /// (unlike the S3 notification path).
    pub fn container_and_key(&self) -> Option<(&str, &str)> {
        let rest = self
            .subject
            .strip_prefix("/blobServices/default/containers/")?;
        let (container, key) = rest.split_once("/blobs/")?;
        if container.is_empty() || key.is_empty() {
            return None;
        }
        Some((container, key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOUD_EVENT: &str = r#"{
        "id":"9b8f2b7c-1",
        "source":"/subscriptions/x/resourceGroups/y/providers/Microsoft.Storage/storageAccounts/acct",
        "specversion":"1.0",
        "type":"Microsoft.Storage.BlobCreated",
        "subject":"/blobServices/default/containers/content/blobs/apps/app1/storage/db/data.lance",
        "time":"2026-08-15T10:00:00Z",
        "data":{"api":"PutBlob","eTag":"0x8D4BCC2E4835CD0","contentType":"application/octet-stream",
                "contentLength":524288,"blobType":"BlockBlob",
                "url":"https://acct.blob.core.windows.net/content/apps/app1/storage/db/data.lance",
                "sequencer":"00000000000004420000000000028963"}
    }"#;

    #[test]
    fn parses_cloud_event_schema() {
        let event = BlobEvent::parse(CLOUD_EVENT.as_bytes()).expect("parses");
        assert_eq!(event.event_type, BLOB_CREATED);
        assert_eq!(event.content_length, Some(524288));
        assert_eq!(event.etag.as_deref(), Some("0x8D4BCC2E4835CD0"));
        assert_eq!(
            event.container_and_key(),
            Some(("content", "apps/app1/storage/db/data.lance"))
        );
        assert!(!event.is_deletion());
    }

    #[test]
    fn parses_event_grid_schema_and_deletion() {
        let json = r#"{"id":"1","topic":"t","subject":"/blobServices/default/containers/content/blobs/users/u1/apps/a1/x.bin",
            "eventType":"Microsoft.Storage.BlobDeleted","eventTime":"2026-08-15T10:00:00Z",
            "data":{"api":"DeleteBlob","sequencer":"00000000000004420000000000028964"},"dataVersion":""}"#;
        let event = BlobEvent::parse(json.as_bytes()).expect("parses");
        assert!(event.is_deletion());
        assert_eq!(event.content_length, None);
        assert_eq!(
            event.container_and_key(),
            Some(("content", "users/u1/apps/a1/x.bin"))
        );
    }

    #[test]
    fn rejects_other_types_and_garbage() {
        let renamed = CLOUD_EVENT.replace(
            "Microsoft.Storage.BlobCreated",
            "Microsoft.Storage.BlobRenamed",
        );
        assert_eq!(
            BlobEvent::parse(renamed.as_bytes()).unwrap_err(),
            BlobEventError::UnsupportedType
        );
        assert_eq!(
            BlobEvent::parse(b"not json").unwrap_err(),
            BlobEventError::Malformed
        );
    }
}
