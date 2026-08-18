//! Cloud Storage object notifications as delivered by Pub/Sub.
//!
//! `google_storage_notification` splits one event across two places: the routing
//! facts (`eventType`, `bucketId`, `objectId`, `objectGeneration`) travel as
//! Pub/Sub message *attributes*, and the object resource — where `size` and
//! `etag` live — travels as the message body in JSON API v1 form. The
//! attributes are authoritative for everything they carry, because the body is
//! absent entirely when a notification is created with
//! `payload_format = "NONE"`.
//!
//! Only the fields the workers act on are modelled; everything else is ignored.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const OBJECT_FINALIZE: &str = "OBJECT_FINALIZE";
pub const OBJECT_DELETE: &str = "OBJECT_DELETE";

const ATTRIBUTE_EVENT_TYPE: &str = "eventType";
const ATTRIBUTE_BUCKET: &str = "bucketId";
const ATTRIBUTE_OBJECT: &str = "objectId";
const ATTRIBUTE_GENERATION: &str = "objectGeneration";

/// One object event, normalised across the attribute and body halves.
#[derive(Clone, Debug)]
pub struct GcsEvent {
    /// `<bucket>/<object>/<generation>`, the identity GCS itself assigns.
    pub id: String,
    pub event_type: String,
    pub bucket: String,
    /// The object name. GCS notification attributes carry it unescaped, exactly
    /// like an Event Grid subject and unlike an S3 notification key, so there is
    /// no percent-decoding step here and adding one would corrupt any object
    /// whose name legitimately contains a `%`.
    pub object: String,
    pub size: Option<i64>,
    /// Per-object monotonic ordering token, the GCS analogue of Event Grid's
    /// `sequencer`. It is the object's creation time in microseconds rendered as
    /// a decimal integer, so the staleness comparison that was lexicographic on
    /// Azure has to become **numeric** here: as text, `"9"` sorts above `"10"`,
    /// and every carry into a new digit would make a fresh event look stale.
    pub generation: Option<i64>,
    pub etag: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GcsEventError {
    Malformed,
    UnsupportedType,
}

/// The GCS JSON API v1 object resource, as published in the message body.
#[derive(Debug, Default, Deserialize)]
struct ObjectResource {
    #[serde(default)]
    id: Option<String>,
    /// `size` and `generation` are uint64 fields, and the JSON API renders every
    /// 64-bit integer as a string so that no JSON parser can round it through an
    /// f64 and lose the low bits. A number is accepted too, because nothing in
    /// the contract forbids a future encoder from emitting one.
    #[serde(default)]
    size: Option<Value>,
    #[serde(default)]
    generation: Option<Value>,
    #[serde(default)]
    etag: Option<String>,
}

impl GcsEvent {
    pub fn parse(
        attributes: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<Self, GcsEventError> {
        let event_type = required(attributes, ATTRIBUTE_EVENT_TYPE)?;
        // The allowlist is the same one the Terraform notifications subscribe
        // to. Anything else — OBJECT_METADATA_UPDATE, OBJECT_ARCHIVE — is a
        // subscription that was widened out of band, and processing it would
        // apply a size delta for an event that changed no bytes.
        if event_type != OBJECT_FINALIZE && event_type != OBJECT_DELETE {
            return Err(GcsEventError::UnsupportedType);
        }
        let bucket = required(attributes, ATTRIBUTE_BUCKET)?;
        let object = required(attributes, ATTRIBUTE_OBJECT)?;

        // An absent or unparsable body is not fatal: the attributes alone are
        // enough to route the event, and the workloads that need `size` fail
        // with their own permanent code so the reason is legible.
        let resource = if body.is_empty() {
            ObjectResource::default()
        } else {
            serde_json::from_slice::<ObjectResource>(body).unwrap_or_default()
        };

        let generation = attributes
            .get(ATTRIBUTE_GENERATION)
            .and_then(|value| value.trim().parse::<i64>().ok())
            .or_else(|| lenient_i64(resource.generation.as_ref()));

        let id = resource.id.filter(|id| !id.is_empty()).unwrap_or_else(|| {
            // The resource `id` is exactly this concatenation, so reconstructing
            // it keeps the event identity stable whether or not a body arrived.
            match generation {
                Some(generation) => format!("{bucket}/{object}/{generation}"),
                None => format!("{bucket}/{object}"),
            }
        });

        Ok(Self {
            id,
            event_type,
            bucket,
            object,
            size: lenient_i64(resource.size.as_ref()),
            generation,
            etag: resource.etag,
        })
    }

    pub fn is_deletion(&self) -> bool {
        self.event_type == OBJECT_DELETE
    }
}

fn required(attributes: &BTreeMap<String, String>, name: &str) -> Result<String, GcsEventError> {
    attributes
        .get(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or(GcsEventError::Malformed)
}

fn lenient_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(text) => text.trim().parse::<i64>().ok(),
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT_BODY: &str = r#"{
        "kind":"storage#object",
        "id":"flow-like-content/apps/app1/storage/db/data.lance/1755254400000001",
        "name":"apps/app1/storage/db/data.lance",
        "bucket":"flow-like-content",
        "generation":"1755254400000001",
        "metageneration":"1",
        "contentType":"application/octet-stream",
        "size":"524288",
        "etag":"CIGl8fjX2/oCEAE=",
        "timeCreated":"2026-08-15T10:00:00.000Z"
    }"#;

    fn attributes(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn finalize_attributes() -> BTreeMap<String, String> {
        attributes(&[
            ("eventType", OBJECT_FINALIZE),
            ("bucketId", "flow-like-content"),
            ("objectId", "apps/app1/storage/db/data.lance"),
            ("objectGeneration", "1755254400000001"),
            ("payloadFormat", "JSON_API_V1"),
        ])
    }

    #[test]
    fn parses_attributes_and_object_resource() {
        let event =
            GcsEvent::parse(&finalize_attributes(), OBJECT_BODY.as_bytes()).expect("parses");
        assert_eq!(event.event_type, OBJECT_FINALIZE);
        assert_eq!(event.bucket, "flow-like-content");
        assert_eq!(event.object, "apps/app1/storage/db/data.lance");
        assert_eq!(event.size, Some(524_288));
        assert_eq!(event.generation, Some(1_755_254_400_000_001));
        assert_eq!(event.etag.as_deref(), Some("CIGl8fjX2/oCEAE="));
        assert!(!event.is_deletion());
    }

    #[test]
    fn survives_a_notification_published_without_a_body() {
        let event = GcsEvent::parse(&finalize_attributes(), b"").expect("parses");
        assert_eq!(event.size, None);
        assert_eq!(event.generation, Some(1_755_254_400_000_001));
        assert_eq!(
            event.id,
            "flow-like-content/apps/app1/storage/db/data.lance/1755254400000001"
        );
    }

    #[test]
    fn parses_deletions_and_keeps_object_names_unescaped() {
        let event = GcsEvent::parse(
            &attributes(&[
                ("eventType", OBJECT_DELETE),
                ("bucketId", "flow-like-content"),
                ("objectId", "users/u1/apps/a1/100% done.bin"),
                ("objectGeneration", "1755254400000002"),
            ]),
            b"",
        )
        .expect("parses");
        assert!(event.is_deletion());
        assert_eq!(event.object, "users/u1/apps/a1/100% done.bin");
    }

    #[test]
    fn rejects_other_event_types_and_missing_routing_attributes() {
        let mut archived = finalize_attributes();
        archived.insert("eventType".to_string(), "OBJECT_ARCHIVE".to_string());
        assert_eq!(
            GcsEvent::parse(&archived, OBJECT_BODY.as_bytes()).unwrap_err(),
            GcsEventError::UnsupportedType
        );

        let mut nameless = finalize_attributes();
        nameless.remove("objectId");
        assert_eq!(
            GcsEvent::parse(&nameless, OBJECT_BODY.as_bytes()).unwrap_err(),
            GcsEventError::Malformed
        );

        assert_eq!(
            GcsEvent::parse(&BTreeMap::new(), b"").unwrap_err(),
            GcsEventError::Malformed
        );
    }

    /// The generation is what orders events, and it arrives as a decimal string
    /// wide enough that a lexicographic comparison reverses at every digit
    /// boundary. This is the test that pins the numeric reading.
    #[test]
    fn generations_are_read_as_numbers_not_as_text() {
        let mut later = finalize_attributes();
        later.insert("objectGeneration".to_string(), "10".to_string());
        let mut earlier = finalize_attributes();
        earlier.insert("objectGeneration".to_string(), "9".to_string());

        let later = GcsEvent::parse(&later, b"").expect("parses");
        let earlier = GcsEvent::parse(&earlier, b"").expect("parses");
        assert!(later.generation > earlier.generation);
        assert!("10" < "9");
    }
}
