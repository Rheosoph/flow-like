use crate::{ProtocolError, Result, StoragePurpose};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const OFFLINE_REPLAY_PATH: &str = "/instances/project/offline/replay";
pub const MAX_OFFLINE_OPERATION_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_OFFLINE_REPLAY_HTTP_BYTES: usize = 12 * 1024 * 1024;

pub const DESKTOP_OFFLINE_REPLAY_ROUTE: &str = "/apps/{app_id}/invoke/offline/replay";
pub const DESKTOP_OFFLINE_CAPABILITIES_ROUTE: &str = "/apps/{app_id}/invoke/offline/capabilities";
pub const OFFLINE_INSTALLATION_HEADER: &str = "x-flow-like-offline-installation";
pub const OFFLINE_SUBJECT_HEADER: &str = "x-flow-like-offline-subject";

pub const OFFLINE_ERROR_INVALID: &str = "OFFLINE_INVALID";
pub const OFFLINE_ERROR_LIMIT_EXCEEDED: &str = "OFFLINE_LIMIT_EXCEEDED";
pub const OFFLINE_ERROR_FORBIDDEN: &str = "OFFLINE_FORBIDDEN";
pub const OFFLINE_ERROR_PRINCIPAL_UNSUPPORTED: &str = "OFFLINE_PRINCIPAL_UNSUPPORTED";
pub const OFFLINE_ERROR_SUBJECT_MISMATCH: &str = "OFFLINE_SUBJECT_MISMATCH";
pub const OFFLINE_ERROR_DIGEST_REUSED: &str = "OFFLINE_DIGEST_REUSED";

const OPERATION_LIMIT_SUBJECT: &str = "offline operation";
const REQUEST_LIMIT_SUBJECT: &str = "offline request";

pub fn desktop_offline_replay_path(app_id: &str) -> String {
    DESKTOP_OFFLINE_REPLAY_ROUTE.replace("{app_id}", app_id)
}

pub fn desktop_offline_capabilities_path(app_id: &str) -> String {
    DESKTOP_OFFLINE_CAPABILITIES_ROUTE.replace("{app_id}", app_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineLimits {
    /// `serde_json::to_vec(&mutation).len()` of a table mutation.
    pub max_operation_bytes: usize,
    /// Decoded bytes of a `FilePut`.
    pub max_file_bytes: usize,
    /// `request_wire_bytes(request)`. `None` leaves it unchecked; the instance route caps the raw body.
    pub max_request_bytes: Option<usize>,
}

pub const INSTANCE_OFFLINE_LIMITS: OfflineLimits = OfflineLimits {
    max_operation_bytes: MAX_OFFLINE_OPERATION_BYTES,
    max_file_bytes: MAX_OFFLINE_OPERATION_BYTES,
    max_request_bytes: None,
};

pub const DESKTOP_OFFLINE_LIMITS: OfflineLimits = OfflineLimits {
    max_operation_bytes: 4 * 1024 * 1024,
    max_file_bytes: 3 * 1024 * 1024,
    max_request_bytes: Some(5_000_000),
};

/// Bytes the serialized request occupies when embedded as a JSON string, as in a Lambda
/// proxy event: `serde_json::to_vec(request)` plus one byte per `"` and `\`, plus the two
/// enclosing quotes. Equals `serde_json::to_string(&body_as_str).len()`.
pub fn request_wire_bytes(request: &OfflineReplayRequest) -> Result<usize> {
    let body = serde_json::to_vec(request)
        .map_err(|_| ProtocolError::Invalid("invalid offline operation"))?;
    let escaped = body
        .iter()
        .filter(|byte| matches!(byte, b'"' | b'\\'))
        .count();
    Ok(body.len() + escaped + 2)
}

/// "8 MiB" for whole MiB, else "4.8 MB".
pub fn format_limit(bytes: usize) -> String {
    const MIB: usize = 1024 * 1024;
    if bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfflineContentProvider {
    S3,
    Az,
    Gs,
}

/// Without `deny_unknown_fields`: newer hubs may add fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopOfflineCapabilities {
    pub version: u32,
    pub limits: OfflineLimits,
    pub provider: OfflineContentProvider,
}

fn is_canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        })
}

pub fn validate_installation_id(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if !is_canonical_uuid(value)
        || bytes[14] != b'4'
        || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
    {
        return Err(ProtocolError::Invalid(
            "installation ID must be a canonical lowercase UUID v4",
        ));
    }
    Ok(())
}

fn ensure_within(bytes: usize, limit: usize, what: &'static str) -> Result<()> {
    if bytes > limit {
        return Err(ProtocolError::TooLarge { what, limit });
    }
    Ok(())
}

/// Exact for padded standard base64; an over-estimate for malformed input, which fails decoding.
fn base64_decoded_len(encoded: &str) -> usize {
    let padding = encoded
        .bytes()
        .rev()
        .take(2)
        .take_while(|byte| *byte == b'=')
        .count();
    (encoded.len().div_ceil(4) * 3).saturating_sub(padding)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfflineResource {
    Table {
        purpose: StoragePurpose,
        database: String,
        table: String,
    },
    File {
        purpose: StoragePurpose,
        path: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfflineExpected {
    TableVersion {
        version: u64,
        #[serde(default)]
        fingerprint: Option<String>,
    },
    FileAbsent,
    FileRevision {
        e_tag: Option<String>,
        version: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfflineMutation {
    TableInsert {
        rows: Vec<Value>,
    },
    TableUpsert {
        id_field: String,
        rows: Vec<Value>,
    },
    TableUpdate {
        filter: String,
        updates: BTreeMap<String, String>,
    },
    TableDelete {
        filter: String,
    },
    FilePut {
        data_base64: String,
        sha256: String,
    },
    FileDelete,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfflineReplayRequest {
    pub operation_id: String,
    pub resource: OfflineResource,
    pub expected: OfflineExpected,
    pub mutation: OfflineMutation,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfflineReplayStatus {
    Applied,
    Conflict,
    OutcomeUnknown,
    Unsupported,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfflineReplayResponse {
    pub operation_id: String,
    pub digest: String,
    pub status: OfflineReplayStatus,
    pub result: Option<OfflineExpected>,
    pub message: Option<String>,
}

/// Paths are object keys relative to a server-selected prefix, never URLs.
pub fn validate_offline_relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 1024
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(ProtocolError::Invalid("invalid offline relative path"));
    }
    Ok(())
}

impl OfflineReplayRequest {
    /// The digest includes the frozen precondition and payload. Once dispatched,
    /// a queued operation must retain the same ID and digest on every retry.
    pub fn digest(&self) -> Result<String> {
        let value = serde_json::to_value(self)
            .map_err(|_| ProtocolError::Invalid("invalid offline operation"))?;
        // serde_json's default map ordering is not a protocol assumption.
        fn canonical(
            value: &Value,
            out: &mut Vec<u8>,
        ) -> std::result::Result<(), serde_json::Error> {
            match value {
                Value::Object(map) => {
                    out.push(b'{');
                    for (index, (key, value)) in map
                        .iter()
                        .collect::<BTreeMap<_, _>>()
                        .into_iter()
                        .enumerate()
                    {
                        if index != 0 {
                            out.push(b',');
                        }
                        serde_json::to_writer(&mut *out, key)?;
                        out.push(b':');
                        canonical(value, out)?;
                    }
                    out.push(b'}');
                }
                Value::Array(values) => {
                    out.push(b'[');
                    for (index, value) in values.iter().enumerate() {
                        if index != 0 {
                            out.push(b',');
                        }
                        canonical(value, out)?;
                    }
                    out.push(b']');
                }
                value => serde_json::to_writer(out, value)?,
            }
            Ok(())
        }
        let mut bytes = Vec::new();
        canonical(&value, &mut bytes)
            .map_err(|_| ProtocolError::Invalid("invalid offline operation"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_with(&INSTANCE_OFFLINE_LIMITS)
    }

    /// The desktop route accepts only purposes a desktop run buffers.
    pub fn validate_desktop(&self, limits: &OfflineLimits) -> Result<()> {
        let (OfflineResource::Table { purpose, .. } | OfflineResource::File { purpose, .. }) =
            &self.resource;
        if !matches!(
            purpose,
            StoragePurpose::Files | StoragePurpose::Storage | StoragePurpose::User
        ) {
            return Err(ProtocolError::Invalid(
                "desktop offline replay does not accept this storage purpose",
            ));
        }
        self.validate_with(limits)
    }

    /// Size failures are `ProtocolError::TooLarge`; everything else is `Invalid`.
    pub fn validate_with(&self, limits: &OfflineLimits) -> Result<()> {
        if !is_canonical_uuid(&self.operation_id) {
            return Err(ProtocolError::Invalid(
                "operation ID must be a canonical UUID",
            ));
        }
        match (&self.resource, &self.expected, &self.mutation) {
            (
                OfflineResource::Table {
                    purpose,
                    database,
                    table,
                },
                OfflineExpected::TableVersion {
                    version,
                    fingerprint,
                },
                mutation,
            ) => {
                if !matches!(purpose, StoragePurpose::Storage | StoragePurpose::User)
                    || !matches!(
                        mutation,
                        OfflineMutation::TableInsert { .. }
                            | OfflineMutation::TableUpsert { .. }
                            | OfflineMutation::TableUpdate { .. }
                            | OfflineMutation::TableDelete { .. }
                    )
                {
                    return Err(ProtocolError::Invalid("invalid offline table operation"));
                }
                validate_offline_relative_path(database)?;
                crate::validate_instance_identifier(table)?;
                if table.contains('.') || database.split('/').any(|part| part.ends_with(".lance")) {
                    return Err(ProtocolError::Invalid("invalid offline table location"));
                }
                if *version == u64::MAX
                    || match (*version, fingerprint) {
                        (0, None) => false,
                        (1.., Some(value)) => !value.strip_prefix("blake3:").is_some_and(|hash| {
                            hash.len() == 64
                                && hash.bytes().all(|byte| {
                                    byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
                                })
                        }),
                        _ => true,
                    }
                {
                    return Err(ProtocolError::Invalid(
                        "invalid offline table manifest revision",
                    ));
                }
                ensure_within(
                    serde_json::to_vec(mutation)
                        .map_err(|_| ProtocolError::Invalid("invalid table mutation"))?
                        .len(),
                    limits.max_operation_bytes,
                    OPERATION_LIMIT_SUBJECT,
                )?;
            }
            (OfflineResource::File { purpose, path }, expected, mutation) => {
                validate_offline_relative_path(path)?;
                if path.split('/').any(|part| part.ends_with(".lance"))
                    || (matches!(purpose, StoragePurpose::Storage | StoragePurpose::User)
                        && path.split('/').next() == Some("db"))
                {
                    return Err(ProtocolError::Invalid(
                        "Lance objects cannot enter the file outbox",
                    ));
                }
                if !matches!(
                    expected,
                    OfflineExpected::FileAbsent | OfflineExpected::FileRevision { .. }
                ) || !matches!(
                    mutation,
                    OfflineMutation::FilePut { .. } | OfflineMutation::FileDelete
                ) {
                    return Err(ProtocolError::Invalid("invalid offline file operation"));
                }
                if let OfflineExpected::FileRevision { e_tag, version } = expected {
                    if e_tag.is_none() && version.is_none()
                        || [e_tag, version].into_iter().flatten().any(|v| {
                            v.is_empty() || v.len() > 1024 || v.chars().any(char::is_control)
                        })
                    {
                        return Err(ProtocolError::Invalid("invalid file revision"));
                    }
                    if e_tag
                        .as_ref()
                        .is_some_and(|value| value.contains(['*', ',']) || value.starts_with("W/"))
                    {
                        return Err(ProtocolError::Invalid(
                            "file revision requires one concrete strong ETag",
                        ));
                    }
                }
                if let OfflineMutation::FilePut {
                    data_base64,
                    sha256,
                } = mutation
                {
                    ensure_within(
                        base64_decoded_len(data_base64),
                        limits.max_file_bytes,
                        OPERATION_LIMIT_SUBJECT,
                    )?;
                    let data = STANDARD
                        .decode(data_base64)
                        .map_err(|_| ProtocolError::Invalid("invalid file base64"))?;
                    if data.len() > limits.max_file_bytes
                        || format!("{:x}", Sha256::digest(&data)) != *sha256
                    {
                        return Err(ProtocolError::Invalid(
                            "file payload digest or size differs",
                        ));
                    }
                }
            }
            _ => return Err(ProtocolError::Invalid("resource and precondition differ")),
        }
        if let Some(limit) = limits.max_request_bytes {
            ensure_within(request_wire_bytes(self)?, limit, REQUEST_LIMIT_SUBJECT)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> OfflineReplayRequest {
        OfflineReplayRequest {
            operation_id: "71bfc449-a2f1-4fa1-aed8-a3819d0f32d5".into(),
            resource: OfflineResource::File {
                purpose: StoragePurpose::Files,
                path: "report.txt".into(),
            },
            expected: OfflineExpected::FileAbsent,
            mutation: OfflineMutation::FilePut {
                data_base64: STANDARD.encode(b"report"),
                sha256: format!("{:x}", Sha256::digest(b"report")),
            },
        }
    }
    #[test]
    fn payload_and_precondition_are_bound() {
        let request = request();
        request.validate().unwrap();
        let mut different = request.clone();
        different.expected = OfflineExpected::FileRevision {
            e_tag: Some("revision".into()),
            version: None,
        };
        assert_ne!(request.digest().unwrap(), different.digest().unwrap());
        if let OfflineMutation::FilePut { data_base64, .. } = &mut different.mutation {
            *data_base64 = STANDARD.encode(b"other");
        }
        assert!(different.validate().is_err());
    }
    #[test]
    fn scope_and_lance_paths_are_rejected() {
        for path in [
            "../other",
            "/absolute",
            "a//b",
            "a/./b",
            "https://other",
            "table.lance/data/file",
        ] {
            let mut request = request();
            request.resource = OfflineResource::File {
                purpose: StoragePurpose::Files,
                path: path.into(),
            };
            assert!(request.validate().is_err(), "{path}");
        }
        let mut request = request();
        request.resource = OfflineResource::File {
            purpose: StoragePurpose::Storage,
            path: "db/table/data".into(),
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn revisions_require_an_exact_object_or_table_incarnation() {
        for e_tag in ["*", "\"first\",\"second\"", "W/\"weak\""] {
            let mut request = request();
            request.expected = OfflineExpected::FileRevision {
                e_tag: Some(e_tag.into()),
                version: None,
            };
            assert!(request.validate().is_err(), "{e_tag}");
        }
        let mut request = request();
        request.resource = OfflineResource::Table {
            purpose: StoragePurpose::Storage,
            database: "db".into(),
            table: "rows".into(),
        };
        request.mutation = OfflineMutation::TableInsert {
            rows: vec![serde_json::json!({"id":1})],
        };
        for (version, fingerprint) in [
            (1, None),
            (1, Some("blake3:short".into())),
            (0, Some(format!("blake3:{}", "a".repeat(64)))),
        ] {
            request.expected = OfflineExpected::TableVersion {
                version,
                fingerprint,
            };
            assert!(request.validate().is_err());
        }
        request.expected = OfflineExpected::TableVersion {
            version: 1,
            fingerprint: Some(format!("blake3:{}", "a".repeat(64))),
        };
        request.validate().unwrap();
    }

    fn table_request(value: String) -> OfflineReplayRequest {
        OfflineReplayRequest {
            operation_id: "71bfc449-a2f1-4fa1-aed8-a3819d0f32d5".into(),
            resource: OfflineResource::Table {
                purpose: StoragePurpose::Storage,
                database: "db".into(),
                table: "rows".into(),
            },
            expected: OfflineExpected::TableVersion {
                version: 0,
                fingerprint: None,
            },
            mutation: OfflineMutation::TableInsert {
                rows: vec![serde_json::json!({ "value": value })],
            },
        }
    }

    fn file_request(purpose: StoragePurpose, data: &[u8]) -> OfflineReplayRequest {
        OfflineReplayRequest {
            resource: OfflineResource::File {
                purpose,
                path: "report.bin".into(),
            },
            mutation: OfflineMutation::FilePut {
                data_base64: STANDARD.encode(data),
                sha256: format!("{:x}", Sha256::digest(data)),
            },
            ..request()
        }
    }

    fn mutation_bytes(request: &OfflineReplayRequest) -> usize {
        serde_json::to_vec(&request.mutation).unwrap().len()
    }

    fn table_request_with_mutation_bytes(bytes: usize) -> OfflineReplayRequest {
        let base = mutation_bytes(&table_request(String::new()));
        table_request("a".repeat(bytes - base))
    }

    fn assert_too_large(result: Result<()>, expected_what: &str, expected_limit: usize) {
        match result {
            Err(ProtocolError::TooLarge { what, limit }) => {
                assert_eq!((what, limit), (expected_what, expected_limit));
            }
            other => panic!("expected TooLarge for {expected_what}, got {other:?}"),
        }
    }

    fn assert_both_pass(request: &OfflineReplayRequest, limits: &OfflineLimits) {
        request.validate_with(limits).unwrap();
        request.validate_desktop(limits).unwrap();
    }

    fn assert_both_too_large(
        request: &OfflineReplayRequest,
        limits: &OfflineLimits,
        what: &str,
        limit: usize,
    ) {
        assert_too_large(request.validate_with(limits), what, limit);
        assert_too_large(request.validate_desktop(limits), what, limit);
    }

    #[test]
    fn validate_equals_validate_with_instance_limits() {
        let oversized_table = table_request_with_mutation_bytes(MAX_OFFLINE_OPERATION_BYTES + 1);
        let oversized_file = file_request(
            StoragePurpose::Files,
            &vec![0; MAX_OFFLINE_OPERATION_BYTES + 3],
        );
        let mut bad_id = request();
        bad_id.operation_id = "71BFC449-A2F1-4FA1-AED8-A3819D0F32D5".into();
        let mut bad_digest = request();
        if let OfflineMutation::FilePut { sha256, .. } = &mut bad_digest.mutation {
            *sha256 = "0".repeat(64);
        }
        let mut mismatched = request();
        mismatched.expected = OfflineExpected::TableVersion {
            version: 0,
            fingerprint: None,
        };
        let cases = [
            request(),
            table_request("ok".into()),
            table_request_with_mutation_bytes(MAX_OFFLINE_OPERATION_BYTES),
            oversized_table.clone(),
            file_request(StoragePurpose::Temporary, b"scratch"),
            oversized_file.clone(),
            bad_id,
            bad_digest,
            mismatched,
        ];
        for case in &cases {
            assert_eq!(
                case.validate().map_err(|error| error.to_string()),
                case.validate_with(&INSTANCE_OFFLINE_LIMITS)
                    .map_err(|error| error.to_string()),
            );
        }
        for oversized in [&oversized_table, &oversized_file] {
            assert_eq!(
                oversized.validate().unwrap_err().to_string(),
                "invalid device protocol input: offline operation exceeds 8 MiB"
            );
        }
        file_request(StoragePurpose::Files, &vec![0; MAX_OFFLINE_OPERATION_BYTES])
            .validate()
            .unwrap();
    }

    #[test]
    fn request_wire_bytes_counts_escaped_quotes_and_backslashes() {
        let request = table_request(r#"quote " and backslash \ and "\"#.into());
        let body = String::from_utf8(serde_json::to_vec(&request).unwrap()).unwrap();
        let escaped = body.bytes().filter(|b| matches!(b, b'"' | b'\\')).count();
        let wire = request_wire_bytes(&request).unwrap();
        assert_eq!(wire, body.len() + escaped + 2);
        assert_eq!(wire, serde_json::to_string(&body).unwrap().len());
        let plain = table_request("plain".into());
        let plain_body = String::from_utf8(serde_json::to_vec(&plain).unwrap()).unwrap();
        assert_eq!(
            request_wire_bytes(&plain).unwrap(),
            serde_json::to_string(&plain_body).unwrap().len()
        );
    }

    #[test]
    fn device_and_server_share_the_size_predicate() {
        let limits = DESKTOP_OFFLINE_LIMITS;

        let at_limit = table_request_with_mutation_bytes(limits.max_operation_bytes);
        assert_eq!(mutation_bytes(&at_limit), limits.max_operation_bytes);
        assert_both_pass(&at_limit, &limits);
        let over = table_request_with_mutation_bytes(limits.max_operation_bytes + 1);
        assert_both_too_large(
            &over,
            &limits,
            "offline operation",
            limits.max_operation_bytes,
        );

        for size in [
            limits.max_file_bytes - 2,
            limits.max_file_bytes - 1,
            limits.max_file_bytes,
        ] {
            assert_both_pass(&file_request(StoragePurpose::User, &vec![7; size]), &limits);
        }
        for size in [limits.max_file_bytes + 1, limits.max_file_bytes + 2] {
            assert_both_too_large(
                &file_request(StoragePurpose::User, &vec![7; size]),
                &limits,
                "offline operation",
                limits.max_file_bytes,
            );
        }

        let max_request_bytes = limits.max_request_bytes.unwrap();
        let quotes = "\"".repeat(1_200_000);
        let base = request_wire_bytes(&table_request(quotes.clone())).unwrap();
        let padded = |extra: usize| table_request(format!("{quotes}{}", "a".repeat(extra)));
        let at_limit = padded(max_request_bytes - base);
        assert_eq!(request_wire_bytes(&at_limit).unwrap(), max_request_bytes);
        assert!(mutation_bytes(&at_limit) < limits.max_operation_bytes);
        assert_both_pass(&at_limit, &limits);
        let over = padded(max_request_bytes - base + 1);
        assert_both_too_large(&over, &limits, "offline request", max_request_bytes);
        assert_eq!(
            over.validate_with(&limits).unwrap_err().to_string(),
            "invalid device protocol input: offline request exceeds 5.0 MB"
        );
    }

    #[test]
    fn validate_desktop_rejects_temporary_purpose() {
        let request = file_request(StoragePurpose::Temporary, b"scratch");
        request.validate_with(&DESKTOP_OFFLINE_LIMITS).unwrap();
        assert!(matches!(
            request.validate_desktop(&DESKTOP_OFFLINE_LIMITS),
            Err(ProtocolError::Invalid(_))
        ));
        for purpose in [
            StoragePurpose::Files,
            StoragePurpose::Storage,
            StoragePurpose::User,
        ] {
            file_request(purpose, b"kept")
                .validate_desktop(&DESKTOP_OFFLINE_LIMITS)
                .unwrap();
        }
    }

    #[test]
    fn capabilities_tolerate_unknown_fields() {
        let capabilities: DesktopOfflineCapabilities = serde_json::from_value(serde_json::json!({
            "version": 1,
            "limits": {
                "maxOperationBytes": 4194304,
                "maxFileBytes": 3145728,
                "maxRequestBytes": 5000000,
                "maxFutureBytes": 1
            },
            "provider": "az",
            "futureField": { "nested": true }
        }))
        .unwrap();
        assert_eq!(
            capabilities,
            DesktopOfflineCapabilities {
                version: 1,
                limits: DESKTOP_OFFLINE_LIMITS,
                provider: OfflineContentProvider::Az,
            }
        );
        let round_trip: DesktopOfflineCapabilities =
            serde_json::from_str(&serde_json::to_string(&capabilities).unwrap()).unwrap();
        assert_eq!(round_trip, capabilities);
        let instance: OfflineLimits = serde_json::from_value(serde_json::json!({
            "maxOperationBytes": MAX_OFFLINE_OPERATION_BYTES,
            "maxFileBytes": MAX_OFFLINE_OPERATION_BYTES,
            "maxRequestBytes": null
        }))
        .unwrap();
        assert_eq!(instance, INSTANCE_OFFLINE_LIMITS);
        for (provider, text) in [
            (OfflineContentProvider::S3, "\"s3\""),
            (OfflineContentProvider::Az, "\"az\""),
            (OfflineContentProvider::Gs, "\"gs\""),
        ] {
            assert_eq!(serde_json::to_string(&provider).unwrap(), text);
        }
    }

    #[test]
    fn installation_ids_must_be_canonical_lowercase_v4() {
        validate_installation_id("71bfc449-a2f1-4fa1-aed8-a3819d0f32d5").unwrap();
        for invalid in [
            "",
            "71BFC449-A2F1-4FA1-AED8-A3819D0F32D5",
            "71bfc449-a2f1-1fa1-aed8-a3819d0f32d5",
            "71bfc449-a2f1-4fa1-7ed8-a3819d0f32d5",
            "71bfc449a2f14fa1aed8a3819d0f32d5",
            "{71bfc449-a2f1-4fa1-aed8-a3819d0f32d5}",
            "71bfc449-a2f1-4fa1-aed8-a3819d0f32d5 ",
        ] {
            assert!(validate_installation_id(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn limits_render_as_mib_or_decimal_mb() {
        assert_eq!(format_limit(MAX_OFFLINE_OPERATION_BYTES), "8 MiB");
        assert_eq!(format_limit(3 * 1024 * 1024), "3 MiB");
        assert_eq!(format_limit(5_000_000), "5.0 MB");
        assert_eq!(format_limit(4_800_000), "4.8 MB");
    }

    #[test]
    fn desktop_paths_substitute_the_app_id() {
        assert_eq!(
            desktop_offline_replay_path("app-1"),
            "/apps/app-1/invoke/offline/replay"
        );
        assert_eq!(
            desktop_offline_capabilities_path("app-1"),
            "/apps/app-1/invoke/offline/capabilities"
        );
    }
}
