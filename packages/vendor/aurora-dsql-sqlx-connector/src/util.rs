// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use regex::Regex;
use std::fmt;
use std::sync::LazyLock;

/// DSQL cluster ID (26 lowercase alphanumeric characters).
/// Validated on construction via `ClusterId::new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterId(String);

impl ClusterId {
    /// Returns `None` if the input is not a valid cluster ID.
    pub fn new(value: &str) -> Option<Self> {
        if is_cluster_id(value) {
            Some(Self(value.to_string()))
        } else {
            None
        }
    }
}

impl fmt::Display for ClusterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ClusterId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Parse the AWS region from a DSQL hostname.
/// Matches pattern: {cluster}.dsql{suffix?}.{region}.on.aws
/// e.g. "cluster123.dsql.us-east-1.on.aws" → Some(Region("us-east-1"))
pub fn parse_region(host: &str) -> Option<aws_config::Region> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[^.]+\.dsql[^.]*\.([a-z0-9-]+)\.on\.aws$").unwrap());
    RE.captures(host).and_then(|caps| {
        caps.get(1)
            .map(|m| aws_config::Region::new(m.as_str().to_owned()))
    })
}

/// Check if a string looks like a bare DSQL cluster ID
/// (26 lowercase alphanumeric characters, no dots).
pub fn is_cluster_id(input: &str) -> bool {
    !input.is_empty()
        && !input.contains('.')
        && input.len() == 26
        && input
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Build a full DSQL hostname from a cluster ID and region.
/// e.g. ("abc123...", "us-east-1") → "abc123....dsql.us-east-1.on.aws"
pub fn build_hostname(cluster_id: &ClusterId, region: &aws_config::Region) -> String {
    format!("{}.dsql.{}.on.aws", cluster_id, region)
}

// --- Shared validation helpers ---
// Used by both `from_connection_string` and `DsqlConnectOptionsBuilder::validate`.

/// Validate that a host string is non-empty.
pub(crate) fn validate_host(host: &str) -> std::result::Result<(), String> {
    if host.is_empty() {
        return Err("Host is required".into());
    }
    Ok(())
}

/// Build the application_name string for the Postgres startup packet.
pub fn build_application_name(prefix: Option<&str>) -> String {
    let version = env!("CARGO_PKG_VERSION");
    match prefix.map(str::trim) {
        Some(p) if !p.is_empty() => format!("{}:aurora-dsql-rust-sqlx/{}", p, version),
        _ => format!("aurora-dsql-rust-sqlx/{}", version),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_region_standard_hostname() {
        let region = parse_region("abc123.dsql.us-east-1.on.aws");
        assert_eq!(region.as_ref().map(|r| r.as_ref()), Some("us-east-1"));
    }

    #[test]
    fn test_parse_region_other_regions() {
        assert_eq!(
            parse_region("abc123.dsql.us-west-2.on.aws")
                .as_ref()
                .map(|r| r.as_ref()),
            Some("us-west-2")
        );
        assert_eq!(
            parse_region("abc123.dsql.eu-west-1.on.aws")
                .as_ref()
                .map(|r| r.as_ref()),
            Some("eu-west-1")
        );
        assert_eq!(
            parse_region("abc123.dsql.ap-southeast-1.on.aws")
                .as_ref()
                .map(|r| r.as_ref()),
            Some("ap-southeast-1")
        );
    }

    #[test]
    fn test_parse_region_invalid_hostname() {
        assert!(parse_region("localhost").is_none());
        assert!(parse_region("example.com").is_none());
        assert!(parse_region("").is_none());
    }

    #[test]
    fn test_is_cluster_id_valid() {
        assert!(is_cluster_id("abcdefghijklmnopqrstuvwxyz"));
        assert!(is_cluster_id("a1b2c3d4e5f6g7h8i9j0klmnop"));
    }

    #[test]
    fn test_is_cluster_id_invalid() {
        assert!(!is_cluster_id(""));
        assert!(!is_cluster_id("too-short"));
        assert!(!is_cluster_id("abc.def.us-east-1.on.aws"));
        assert!(!is_cluster_id("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
        assert!(!is_cluster_id("abcdefghijklmnopqrstuvwxy")); // 25 chars
        assert!(!is_cluster_id("abcdefghijklmnopqrstuvwxyza")); // 27 chars
    }

    #[test]
    fn test_build_hostname() {
        let cluster = ClusterId::new("abcdefghijklmnopqrstuvwxyz").unwrap();
        let region = aws_config::Region::new("us-east-1");
        assert_eq!(
            build_hostname(&cluster, &region),
            "abcdefghijklmnopqrstuvwxyz.dsql.us-east-1.on.aws"
        );
    }

    #[test]
    fn test_build_application_name_no_prefix() {
        let name = build_application_name(None);
        assert!(name.starts_with("aurora-dsql-rust-sqlx/"));
    }

    #[test]
    fn test_build_application_name_with_prefix() {
        let name = build_application_name(Some("myapp"));
        assert!(name.starts_with("myapp:aurora-dsql-rust-sqlx/"));
    }

    #[test]
    fn test_build_application_name_empty_prefix() {
        let name = build_application_name(Some(""));
        assert!(name.starts_with("aurora-dsql-rust-sqlx/"));
    }

    // --- shared validation helper tests ---

    #[test]
    fn test_validate_host_ok() {
        assert!(validate_host("example.dsql.us-east-1.on.aws").is_ok());
    }

    #[test]
    fn test_validate_host_empty() {
        assert!(validate_host("").is_err());
    }
}
