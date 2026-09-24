use crate::{ProtocolError, Result, StoragePurpose};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const OFFLINE_REPLAY_PATH: &str = "/instances/project/offline/replay";
pub const MAX_OFFLINE_OPERATION_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_OFFLINE_REPLAY_HTTP_BYTES: usize = 12 * 1024 * 1024;

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
        if self.operation_id.len() != 36
            || self.operation_id.bytes().enumerate().any(|(i, b)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    b != b'-'
                } else {
                    !b.is_ascii_hexdigit() || b.is_ascii_uppercase()
                }
            })
        {
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
                if serde_json::to_vec(mutation)
                    .map_err(|_| ProtocolError::Invalid("invalid table mutation"))?
                    .len()
                    > MAX_OFFLINE_OPERATION_BYTES
                {
                    return Err(ProtocolError::Invalid("offline operation exceeds 8 MiB"));
                }
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
                    if data_base64.len() > MAX_OFFLINE_OPERATION_BYTES.div_ceil(3) * 4 {
                        return Err(ProtocolError::Invalid("offline operation exceeds 8 MiB"));
                    }
                    let data = STANDARD
                        .decode(data_base64)
                        .map_err(|_| ProtocolError::Invalid("invalid file base64"))?;
                    if data.len() > MAX_OFFLINE_OPERATION_BYTES
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
}
