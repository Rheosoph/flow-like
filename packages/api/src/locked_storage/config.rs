use crate::entity::sea_orm_active_enums::{LockedEnforcement, LockedRetentionMode};
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// The secret-store entry that can hold every setting as one JSON object, keyed by
/// the variable names below. It keeps size-capped environments such as Lambda's
/// 4 KB free of locked storage; set variables still override its values.
pub const SETTINGS_DOCUMENT: &str = "LOCKED_STORAGE";
const MAX_DOCUMENT_BYTES: usize = 16 * 1024;

pub const SETTINGS: &[&str] = &[
    "AWS_LOCKED_BUCKET",
    "GCP_LOCKED_BUCKET",
    "AZURE_LOCKED_CONTAINER",
    "LOCKED_BUCKET",
    "LOCKED_BUCKET_PROVIDER",
    "LOCKED_BUCKET_ENFORCEMENT",
    "LOCKED_BUCKET_LOCK_MODE",
    "LOCKED_BUCKET_RETENTION_DAYS",
    "LOCKED_BUCKET_MIN_RETENTION_DAYS",
    "LOCKED_BUCKET_MAX_RETENTION_DAYS",
    "LOCKED_BUCKET_MAX_FILE_BYTES",
    "LOCKED_BUCKET_VERIFY_MAX_BYTES",
];

/// String and number values; anything else, or an unknown name, is a typo worth failing on.
pub fn parse_settings_document(document: &str) -> Result<HashMap<String, String>> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(anyhow!(
            "{SETTINGS_DOCUMENT} must be at most {MAX_DOCUMENT_BYTES} bytes"
        ));
    }
    let object: serde_json::Map<String, serde_json::Value> = serde_json::from_str(document)
        .map_err(|_| anyhow!("{SETTINGS_DOCUMENT} must be a JSON object"))?;
    let mut settings = HashMap::new();
    for (name, value) in object {
        if !SETTINGS.contains(&name.as_str()) {
            return Err(anyhow!(
                "{SETTINGS_DOCUMENT} names an unknown setting '{name}'"
            ));
        }
        let value = match value {
            serde_json::Value::String(value) => value,
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Null => continue,
            _ => {
                return Err(anyhow!(
                    "{SETTINGS_DOCUMENT} setting '{name}' must be a string or a number"
                ));
            }
        };
        if !value.trim().is_empty() {
            settings.insert(name, value.trim().to_owned());
        }
    }
    Ok(settings)
}

/// The longest retention any provider accepts (GCS caps retain-until at 100 years).
pub const MAX_RETENTION_DAYS: u32 = 36_500;
const DEFAULT_RETENTION_DAYS: u32 = 3_650;
const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const DEFAULT_VERIFY_MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LockedProvider {
    S3,
    R2,
    Gcs,
    Azure,
}

impl LockedProvider {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "s3" | "aws" | "minio" | "rustfs" => Ok(Self::S3),
            "r2" | "cloudflare" => Ok(Self::R2),
            "gcs" | "gcp" | "google" => Ok(Self::Gcs),
            "azure" | "blob" => Ok(Self::Azure),
            other => Err(anyhow!(
                "LOCKED_BUCKET_PROVIDER '{other}' is not one of s3, r2, gcs, azure"
            )),
        }
    }

    /// Only these providers can lock an individual object version.
    pub fn supports_object_enforcement(self) -> bool {
        !matches!(self, Self::R2)
    }

    pub fn supports_legal_hold(self) -> bool {
        !matches!(self, Self::R2)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockedStorageConfig {
    pub bucket: String,
    pub provider: LockedProvider,
    pub enforcement: LockedEnforcement,
    pub mode: LockedRetentionMode,
    pub default_retention_days: u32,
    pub min_retention_days: u32,
    pub max_retention_days: u32,
    pub max_file_bytes: u64,
    pub verify_max_bytes: u64,
}

impl LockedStorageConfig {
    /// `None` when no locked bucket is configured: the feature is then disabled.
    pub(crate) fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        let Some((bucket, inferred)) = [
            ("AWS_LOCKED_BUCKET", Some(LockedProvider::S3)),
            ("GCP_LOCKED_BUCKET", Some(LockedProvider::Gcs)),
            ("AZURE_LOCKED_CONTAINER", Some(LockedProvider::Azure)),
            ("LOCKED_BUCKET", None),
        ]
        .into_iter()
        .find_map(|(name, provider)| lookup(name).map(|bucket| (bucket, provider))) else {
            return Ok(None);
        };

        let provider = match lookup("LOCKED_BUCKET_PROVIDER") {
            Some(value) => LockedProvider::parse(&value)?,
            None => match inferred {
                Some(provider) => provider,
                None => lookup("STORAGE_PROVIDER")
                    .map(|value| LockedProvider::parse(&value))
                    .transpose()?
                    .unwrap_or(LockedProvider::S3),
            },
        };

        let enforcement = match lookup("LOCKED_BUCKET_ENFORCEMENT") {
            Some(value) => match value.trim().to_ascii_lowercase().as_str() {
                "object" => LockedEnforcement::Object,
                "bucket" => LockedEnforcement::Bucket,
                other => {
                    return Err(anyhow!(
                        "LOCKED_BUCKET_ENFORCEMENT '{other}' is not one of object, bucket"
                    ));
                }
            },
            None if provider.supports_object_enforcement() => LockedEnforcement::Object,
            None => LockedEnforcement::Bucket,
        };
        if enforcement == LockedEnforcement::Object && !provider.supports_object_enforcement() {
            return Err(anyhow!(
                "{provider:?} cannot lock individual objects; set LOCKED_BUCKET_ENFORCEMENT=bucket and configure a bucket lock rule"
            ));
        }

        let mode = match lookup("LOCKED_BUCKET_LOCK_MODE") {
            Some(value) => match value.trim().to_ascii_uppercase().as_str() {
                "COMPLIANCE" => LockedRetentionMode::Compliance,
                "GOVERNANCE" => LockedRetentionMode::Governance,
                other => {
                    return Err(anyhow!(
                        "LOCKED_BUCKET_LOCK_MODE '{other}' is not one of COMPLIANCE, GOVERNANCE"
                    ));
                }
            },
            None => LockedRetentionMode::Compliance,
        };

        let default_retention_days = number(
            &lookup,
            "LOCKED_BUCKET_RETENTION_DAYS",
            DEFAULT_RETENTION_DAYS,
        )?;
        let (min_retention_days, max_retention_days) = match enforcement {
            LockedEnforcement::Bucket => (default_retention_days, default_retention_days),
            LockedEnforcement::Object => (
                number(&lookup, "LOCKED_BUCKET_MIN_RETENTION_DAYS", 1)?,
                number(
                    &lookup,
                    "LOCKED_BUCKET_MAX_RETENTION_DAYS",
                    MAX_RETENTION_DAYS,
                )?,
            ),
        };
        if min_retention_days == 0
            || min_retention_days > default_retention_days
            || default_retention_days > max_retention_days
            || max_retention_days > MAX_RETENTION_DAYS
        {
            return Err(anyhow!(
                "locked retention days must satisfy 1 <= min ({min_retention_days}) <= default ({default_retention_days}) <= max ({max_retention_days}) <= {MAX_RETENTION_DAYS}"
            ));
        }

        Ok(Some(Self {
            bucket,
            provider,
            enforcement,
            mode,
            default_retention_days,
            min_retention_days,
            max_retention_days,
            max_file_bytes: number(
                &lookup,
                "LOCKED_BUCKET_MAX_FILE_BYTES",
                DEFAULT_MAX_FILE_BYTES,
            )?,
            verify_max_bytes: number(
                &lookup,
                "LOCKED_BUCKET_VERIFY_MAX_BYTES",
                DEFAULT_VERIFY_MAX_BYTES,
            )?,
        }))
    }

    /// The retention a new version gets: the caller's choice within the configured
    /// bounds, raised to the minimum, or the bucket rule in bucket mode.
    pub fn retention_days(&self, requested: Option<u32>) -> std::result::Result<u32, String> {
        if self.enforcement == LockedEnforcement::Bucket {
            return Ok(self.default_retention_days);
        }
        let days = requested.unwrap_or(self.default_retention_days);
        if days > self.max_retention_days {
            return Err(format!(
                "Retention of {days} days exceeds the maximum of {} days.",
                self.max_retention_days
            ));
        }
        Ok(days.max(self.min_retention_days))
    }
}

fn number<T: std::str::FromStr>(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &str,
    default: T,
) -> Result<T> {
    match lookup(name) {
        Some(value) => value
            .trim()
            .parse()
            .map_err(|_| anyhow!("{name} must be a whole number, got '{value}'")),
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<Option<LockedStorageConfig>> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        LockedStorageConfig::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn settings_documents_accept_strings_and_numbers_only_for_known_names() {
        let settings = parse_settings_document(
            r#"{"LOCKED_BUCKET":"locked","LOCKED_BUCKET_RETENTION_DAYS":2920,"LOCKED_BUCKET_LOCK_MODE":null}"#,
        )
        .unwrap();
        assert_eq!(
            settings.get("LOCKED_BUCKET").map(String::as_str),
            Some("locked")
        );
        assert_eq!(
            settings
                .get("LOCKED_BUCKET_RETENTION_DAYS")
                .map(String::as_str),
            Some("2920")
        );
        assert!(!settings.contains_key("LOCKED_BUCKET_LOCK_MODE"));
        assert!(parse_settings_document(r#"{"LOCKED_BUCKETS":"typo"}"#).is_err());
        assert!(parse_settings_document(r#"{"LOCKED_BUCKET":["a"]}"#).is_err());
        assert!(parse_settings_document("[]").is_err());
        assert!(parse_settings_document("{}").unwrap().is_empty());
    }

    #[test]
    fn no_bucket_disables_the_feature() {
        assert_eq!(config(&[("STORAGE_PROVIDER", "aws")]).unwrap(), None);
    }

    #[test]
    fn provider_follows_the_variable_that_named_the_bucket() {
        let gcs = config(&[("GCP_LOCKED_BUCKET", "locked")]).unwrap().unwrap();
        assert_eq!(gcs.provider, LockedProvider::Gcs);
        assert_eq!(gcs.enforcement, LockedEnforcement::Object);
        let azure = config(&[("LOCKED_BUCKET", "locked"), ("STORAGE_PROVIDER", "azure")])
            .unwrap()
            .unwrap();
        assert_eq!(azure.provider, LockedProvider::Azure);
        let s3 = config(&[("LOCKED_BUCKET", "locked")]).unwrap().unwrap();
        assert_eq!(s3.provider, LockedProvider::S3);
        assert_eq!(s3.mode, LockedRetentionMode::Compliance);
        assert_eq!(s3.default_retention_days, 3_650);
    }

    #[test]
    fn r2_only_supports_bucket_rules() {
        let r2 = config(&[
            ("LOCKED_BUCKET", "locked"),
            ("LOCKED_BUCKET_PROVIDER", "r2"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(r2.enforcement, LockedEnforcement::Bucket);
        assert!(
            config(&[
                ("LOCKED_BUCKET", "locked"),
                ("LOCKED_BUCKET_PROVIDER", "r2"),
                ("LOCKED_BUCKET_ENFORCEMENT", "object"),
            ])
            .is_err()
        );
    }

    #[test]
    fn retention_bounds_are_validated() {
        assert!(
            config(&[
                ("LOCKED_BUCKET", "locked"),
                ("LOCKED_BUCKET_MIN_RETENTION_DAYS", "400"),
                ("LOCKED_BUCKET_RETENTION_DAYS", "365"),
            ])
            .is_err()
        );
        assert!(
            config(&[
                ("LOCKED_BUCKET", "locked"),
                ("LOCKED_BUCKET_MAX_RETENTION_DAYS", "40000")
            ])
            .is_err()
        );
        assert!(
            config(&[
                ("LOCKED_BUCKET", "locked"),
                ("LOCKED_BUCKET_LOCK_MODE", "soft")
            ])
            .is_err()
        );
    }

    #[test]
    fn requested_retention_is_raised_to_the_minimum_and_capped() {
        let object = config(&[
            ("LOCKED_BUCKET", "locked"),
            ("LOCKED_BUCKET_MIN_RETENTION_DAYS", "30"),
            ("LOCKED_BUCKET_RETENTION_DAYS", "365"),
            ("LOCKED_BUCKET_MAX_RETENTION_DAYS", "3650"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(object.retention_days(None), Ok(365));
        assert_eq!(object.retention_days(Some(7)), Ok(30));
        assert_eq!(object.retention_days(Some(1000)), Ok(1000));
        assert!(object.retention_days(Some(4000)).is_err());

        let bucket = config(&[
            ("LOCKED_BUCKET", "locked"),
            ("LOCKED_BUCKET_ENFORCEMENT", "bucket"),
            ("LOCKED_BUCKET_RETENTION_DAYS", "2190"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(bucket.retention_days(Some(10)), Ok(2190));
        assert_eq!(
            (bucket.min_retention_days, bucket.max_retention_days),
            (2190, 2190)
        );
    }
}
