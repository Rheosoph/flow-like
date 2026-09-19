use serde_json::{Value, json};

use super::index::SourceIndex;
use super::network::describe_network_with;
use super::rules::psl_version_unix_secs;
use super::*;
use crate::widget_policy::{has_mixed_script_word, reason_characters_allowed};

const FIXTURE: &str = include_str!("../../tests/fixtures/widget_source_classification.json");
const DAY: i64 = 86_400;

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("widget_source_classification.json must parse")
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect()
}

fn embedded() -> &'static SourceIndex {
    source_index().expect("embedded widget source data must load")
}

const TEST_RULES: &str = "\
# psl-version: 2026-01-01_00-00-00_UTC
i com
i uk
i co.uk
i jp
i *.kawasaki.jp
i !city.kawasaki.jp
i ck
i *.ck
i !www.ck
p blogspot.com
p *.hosted.com
p !own.hosted.com
p apps.cloudops.com
p *.regions.cloudops.com
";

const TEST_CATALOG: &str = r#"{
  "catalogVersion": 7,
  "providersSha256": "",
  "publicSuffixRules": { "pslVersion": "2026-01-01_00-00-00_UTC", "sha256": "" },
  "providers": [
    { "id": "cloudops", "name": "CloudOps", "docs": ["https://cloudops.com/docs"],
      "match": [{ "type": "attribution", "domain": "cloudops.com" }], "receives": true },
    { "id": "cloudops-buckets", "name": "CloudOps Buckets", "docs": ["https://cloudops.com/docs"],
      "match": [{ "type": "shared-suffix", "domain": "buckets.cloudops.com" }], "receives": true },
    { "id": "blogs", "name": "Blogs", "docs": ["https://blogspot.com/docs"],
      "match": [{ "type": "shared-suffix", "domain": "blogspot.com" }], "receives": false }
  ]
}"#;

fn test_index() -> SourceIndex {
    SourceIndex::build(TEST_RULES, TEST_CATALOG).expect("test data must load")
}

fn purpose(
    reason: &str,
    declared: &[(CspDirective, &str)],
    inputs: &[&str],
) -> NetworkPurposeInput {
    NetworkPurposeInput {
        reason: reason.to_string(),
        declared: declared
            .iter()
            .map(|(directive, source)| (*directive, source.to_string()))
            .collect(),
        inputs: inputs.iter().map(|input| input.to_string()).collect(),
    }
}

fn runtime_input(
    purpose: usize,
    slot: &str,
    directive: CspDirective,
    source: &str,
) -> NetworkRuntimeInput {
    NetworkRuntimeInput {
        purpose,
        slot: slot.to_string(),
        directive,
        source: source.to_string(),
    }
}

#[test]
fn golden_fixture_classifications_hold() {
    let fixture = fixture();
    let (catalog_version, psl_version) = widget_source_data_versions();
    assert_eq!(
        fixture["catalogVersion"].as_u64(),
        Some(u64::from(catalog_version))
    );
    assert_eq!(fixture["pslVersion"].as_str(), Some(psl_version.as_str()));

    let rows = fixture["classifications"].as_array().unwrap();
    assert!(rows.len() >= 150);
    let mut failures = Vec::new();
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let provenance = match row["provenance"].as_str() {
            Some("runtime") => SourceProvenance::Runtime,
            _ => SourceProvenance::Declared,
        };
        let expected = match row["rejected"].as_str() {
            Some(code) => json!({ "rejected": code }),
            None => json!({
                "kind": row["kind"],
                "level": row["level"],
                "emphasis": row["emphasis"],
                "providerId": row.get("providerId").cloned().unwrap_or(Value::Null),
            }),
        };
        let actual = match classify_widget_source(source, provenance) {
            Ok(class) => {
                assert!(
                    class.host.ends_with(&class.emphasis),
                    "emphasis of {source} must be a suffix of its host"
                );
                assert_eq!(class.provider.is_some(), class.provider_id.is_some());
                json!({
                    "kind": class.kind,
                    "level": class.level,
                    "emphasis": class.emphasis,
                    "providerId": class.provider_id,
                })
            }
            Err(rejection) => json!({ "rejected": rejection.code() }),
        };
        if actual != expected {
            failures.push(format!(
                "{source} ({provenance:?}): expected {expected}, got {actual}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "classification mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn golden_fixture_covers_every_catalog_provider_and_kind_level_pair() {
    let fixture = fixture();
    let rows = fixture["classifications"].as_array().unwrap();
    let covered: Vec<&str> = rows
        .iter()
        .filter_map(|row| row["providerId"].as_str())
        .collect();
    for provider in &embedded().catalog.providers {
        assert!(
            covered.contains(&provider.id.as_str()),
            "fixture has no row for provider {}",
            provider.id
        );
    }
    for kind in [
        "service",
        "exact",
        "subdomains",
        "tenant-host",
        "tenant-subdomains",
        "shared-host",
        "shared-wildcard",
        "platform-host",
    ] {
        assert!(
            rows.iter().any(|row| row["kind"] == kind),
            "fixture has no {kind} row"
        );
    }
}

#[test]
fn golden_fixture_wildcard_bases() {
    let fixture = fixture();
    for source in strings(&fixture["wildcardBases"]["accepted"]) {
        assert_eq!(validate_wildcard_bases([source]), Ok(()), "{source}");
    }
    let rejected = strings(&fixture["wildcardBases"]["rejected"]);
    let errors = validate_wildcard_bases(rejected.iter().copied()).unwrap_err();
    assert_eq!(errors.len(), rejected.len());
    for (source, error) in rejected.iter().zip(&errors) {
        assert!(error.contains(source), "{error}");
        assert!(
            error.contains("wildcard base is a public suffix"),
            "{error}"
        );
    }
    assert_eq!(validate_wildcard_bases(std::iter::empty()), Ok(()));
}

#[test]
fn golden_fixture_reason_addresses() {
    let fixture = fixture();
    for reason in strings(&fixture["reasonAddress"]["accepted"]) {
        assert!(!reason_contains_address(reason), "must accept {reason:?}");
    }
    for reason in strings(&fixture["reasonAddress"]["rejected"]) {
        assert!(reason_contains_address(reason), "must reject {reason:?}");
    }
}

#[test]
fn reason_rule_8_examples() {
    for accepted in [
        "Node.js",
        "e.g.",
        "Loads tiles, e.g. from a CDN",
        "v1.2 tiles",
        "Loads Three.js models",
    ] {
        assert!(
            !reason_contains_address(accepted),
            "must accept {accepted:?}"
        );
    }
    for rejected in [
        "https://tiles.example.org",
        "wss://live",
        "maps@example.org",
        "www.tiles",
        "example.com",
        "tiles.example.org.",
        "tiles from mapbox.io",
        "example.xn--p1ai",
        "пример.рф",
        "tiles。example。com",
        "ｅｘａｍｐｌｅ．ｃｏｍ",
        "EXAMPLE.COM",
    ] {
        assert!(
            reason_contains_address(rejected),
            "must reject {rejected:?}"
        );
    }
    assert!(
        !reason_contains_address("tiles。例"),
        "U+3002 next to a non-ASCII letter is not a dot"
    );
}

#[test]
fn embedded_data_matches_pins() {
    let index = embedded();
    assert_eq!(
        index.catalog.public_suffix_rules.sha256, index.rules_sha256,
        "run mise run widget-sources:generate"
    );
    assert_eq!(
        index.catalog.public_suffix_rules.psl_version, index.psl_version,
        "run mise run widget-sources:generate"
    );
    assert_eq!(
        index.catalog.providers_sha256, index.providers_sha256,
        "run mise run widget-sources:generate"
    );
    assert!(PUBLIC_SUFFIX_RULES.contains("Mozilla Public"));
    assert!(
        PUBLIC_SUFFIX_RULES
            .lines()
            .any(|line| line.starts_with("# psl-source-sha256: "))
    );
}

#[test]
fn embedded_catalog_invariants_hold() {
    assert_eq!(warm_widget_source_data(), Ok(()));
    assert_eq!(widget_sources_catalog_invariants(), Ok(()));
    let index = embedded();
    for key in WIDGET_SOURCE_ABOUT_KEYS {
        assert!(
            index
                .catalog
                .providers
                .iter()
                .any(|provider| provider.about_key.as_deref() == Some(key)),
            "{key} is unused"
        );
    }
}

#[test]
fn catalog_invariants_reject_broken_catalogs() {
    let broken = r#"{
      "catalogVersion": 1, "providersSha256": "0",
      "publicSuffixRules": { "pslVersion": "2025-01-01_00-00-00_UTC", "sha256": "0" },
      "providers": [
        { "id": "Bad_Id", "name": "Tiles from example.com", "aboutKey": "widgetSourceAboutNothing",
          "docs": ["http://example.org"],
          "match": [
            { "type": "service-host", "domain": "co.uk" },
            { "type": "shared-suffix", "domain": "*.com" },
            { "type": "attribution", "domain": "*.cloudops.com" }
          ] },
        { "id": "dup", "name": "Flοw", "docs": [],
          "match": [{ "type": "shared-host", "domain": "a.example.org" }], "receives": false },
        { "id": "dup", "name": "Emoji 🚀", "docs": ["https://a.example.org"],
          "match": [{ "type": "shared-host", "domain": "a.example.org" }], "receives": false }
      ]
    }"#;
    let index = SourceIndex::build(TEST_RULES, broken).unwrap();
    let errors = invariants::check(&index).unwrap_err().join("\n");
    for expected in [
        "publicSuffixRules.sha256",
        "publicSuffixRules.pslVersion",
        "providersSha256",
        "\"Bad_Id\" is not kebab-case",
        "Bad_Id: name contains an address",
        "unknown aboutKey",
        "is not an https URL",
        "\"co.uk\" is an ICANN public suffix",
        "\"*.com\": needs at least 2 literal rightmost labels",
        "\"*.cloudops.com\": * is only allowed in shared-suffix domains",
        "requires receives",
        "requires userContent",
        "\"dup\" is not unique",
        "dup: name mixes scripts",
        "docs must list at least one https URL",
        "match \"a.example.org\" repeats",
        "dup: name contains a forbidden character",
    ] {
        assert!(
            errors.contains(expected),
            "missing {expected:?} in:\n{errors}"
        );
    }
}

#[test]
fn icann_primitives_follow_the_formal_algorithm() {
    let index = test_index();
    assert_eq!(index.icann_suffix("example.com"), "com");
    assert_eq!(index.icann_suffix("a.example.co.uk"), "co.uk");
    assert_eq!(index.icann_suffix("co.uk"), "co.uk");
    assert_eq!(index.icann_suffix("unlisted"), "unlisted");
    assert_eq!(index.icann_suffix("a.b.unlisted"), "unlisted");
    assert_eq!(index.icann_suffix("a.b.kawasaki.jp"), "b.kawasaki.jp");
    assert_eq!(index.icann_suffix("b.kawasaki.jp"), "b.kawasaki.jp");
    assert_eq!(index.icann_suffix("kawasaki.jp"), "jp");
    assert_eq!(index.icann_suffix("city.kawasaki.jp"), "kawasaki.jp");
    assert_eq!(index.icann_suffix("www.city.kawasaki.jp"), "kawasaki.jp");
    assert_eq!(index.icann_suffix("foo.ck"), "foo.ck");
    assert_eq!(index.icann_suffix("www.ck"), "ck");
    assert_eq!(index.icann_suffix("blogspot.com"), "com");

    assert_eq!(
        index.icann_registrable("a.b.kawasaki.jp"),
        Some("a.b.kawasaki.jp")
    );
    assert_eq!(
        index.icann_registrable("x.city.kawasaki.jp"),
        Some("city.kawasaki.jp")
    );
    assert_eq!(
        index.icann_registrable("www.example.co.uk"),
        Some("example.co.uk")
    );
    assert_eq!(index.icann_registrable("co.uk"), None);

    assert!(index.is_icann_below("kawasaki.jp"));
    assert!(!index.is_icann_below("city.kawasaki.jp"));
    assert!(!index.is_icann_below("jp"));
    assert!(!index.is_icann_below("co.uk"));
    assert!(!index.is_icann_below("www.ck"));

    assert!(index.is_icann_tld("jp"));
    assert!(index.is_icann_tld("ck"));
    assert!(!index.is_icann_tld("kawasaki"));
    assert!(!index.is_icann_tld("blogspot"));
}

#[test]
fn private_rules_define_tenants_except_under_receiving_attributions() {
    let index = test_index();
    let suffix = |host: &str| index.shared_suffix_of(host).map(|suffix| suffix.labels);

    assert_eq!(suffix("me.blogspot.com"), Some(2));
    assert_eq!(suffix("blogspot.com"), Some(2));
    assert_eq!(suffix("com"), None);
    assert_eq!(suffix("site.a.hosted.com"), Some(3));
    assert_eq!(suffix("a.hosted.com"), Some(3));
    assert_eq!(suffix("hosted.com"), None);
    assert_eq!(suffix("site.own.hosted.com"), Some(2));
    assert_eq!(suffix("own.hosted.com"), Some(2));
    assert_eq!(suffix("app.apps.cloudops.com"), None);
    assert_eq!(suffix("x.eu.regions.cloudops.com"), None);
    assert_eq!(suffix("mine.buckets.cloudops.com"), Some(3));

    let blog = index.shared_suffix_of("me.blogspot.com").unwrap();
    assert!(!blog.receives, "catalog wins a tie with the PSL");
    let hosted = index.shared_suffix_of("site.a.hosted.com").unwrap();
    assert!(hosted.receives && hosted.entry.is_none());

    assert_eq!(
        index.shared_below("hosted.com").map(|below| below.level),
        Some(WidgetSourceLevel::Broad)
    );
    assert!(index.shared_below("cloudops.com").is_some());
    assert!(index.shared_below("regions.cloudops.com").is_none());
    assert!(index.shared_below("blogspot.com").is_none());
}

#[test]
fn classification_with_synthetic_data() {
    use WidgetSourceKind as Kind;
    use WidgetSourceLevel as Level;

    let index = test_index();
    let classify = |source: &str| {
        let class = classify_with(&index, source, SourceProvenance::Declared).unwrap();
        (class.kind, class.level, class.emphasis, class.provider_id)
    };

    assert_eq!(
        classify("https://me.blogspot.com"),
        (
            Kind::TenantHost,
            Level::External,
            "me.blogspot.com".into(),
            Some("blogs".into())
        )
    );
    assert_eq!(
        classify("https://*.blogspot.com"),
        (
            Kind::SharedWildcard,
            Level::Shared,
            "blogspot.com".into(),
            Some("blogs".into())
        )
    );
    assert_eq!(
        classify("https://site.a.hosted.com"),
        (
            Kind::TenantHost,
            Level::External,
            "site.a.hosted.com".into(),
            None
        )
    );
    assert_eq!(
        classify("https://*.hosted.com"),
        (
            Kind::SharedWildcard,
            Level::Broad,
            "hosted.com".into(),
            None
        )
    );
    assert_eq!(
        classify("https://*.site.a.hosted.com"),
        (
            Kind::TenantSubdomains,
            Level::External,
            "site.a.hosted.com".into(),
            None
        )
    );
    assert_eq!(
        classify("https://app.apps.cloudops.com"),
        (
            Kind::PlatformHost,
            Level::Broad,
            "app.apps.cloudops.com".into(),
            Some("cloudops".into())
        )
    );
    assert_eq!(
        classify("https://*.regions.cloudops.com"),
        (
            Kind::SharedWildcard,
            Level::Broad,
            "regions.cloudops.com".into(),
            Some("cloudops".into())
        )
    );
    assert_eq!(
        classify("https://mine.buckets.cloudops.com"),
        (
            Kind::TenantHost,
            Level::External,
            "mine.buckets.cloudops.com".into(),
            Some("cloudops-buckets".into())
        )
    );
    assert_eq!(
        classify("https://*.cloudops.com"),
        (
            Kind::SharedWildcard,
            Level::Broad,
            "cloudops.com".into(),
            Some("cloudops".into())
        )
    );
    assert_eq!(
        classify("https://x.city.kawasaki.jp"),
        (
            Kind::Exact,
            Level::External,
            "city.kawasaki.jp".into(),
            None
        )
    );

    assert_eq!(
        classify_with(&index, "https://*.kawasaki.jp", SourceProvenance::Declared),
        Err(WidgetSourceRejection::WildcardPublicSuffix)
    );
    assert_eq!(
        classify_with(
            &index,
            "https://*.b.kawasaki.jp",
            SourceProvenance::Declared
        ),
        Err(WidgetSourceRejection::WildcardPublicSuffix)
    );
    assert!(
        classify_with(
            &index,
            "https://*.city.kawasaki.jp",
            SourceProvenance::Declared
        )
        .is_ok()
    );
    assert!(classify_with(&index, "https://*.www.ck", SourceProvenance::Declared).is_ok());
}

#[test]
fn classes_carry_provider_facts() {
    let class =
        classify_widget_source("https://*.s3.amazonaws.com", SourceProvenance::Declared).unwrap();
    assert_eq!(class.host, "*.s3.amazonaws.com");
    assert_eq!(class.provider.as_deref(), Some("Amazon S3"));
    assert_eq!(
        class.about_key.as_deref(),
        Some("widgetSourceAboutObjectStorage")
    );
    assert_eq!(
        serde_json::to_value(&class).unwrap(),
        json!({
            "kind": "shared-wildcard", "level": "broad", "host": "*.s3.amazonaws.com",
            "emphasis": "s3.amazonaws.com", "providerId": "aws-s3", "provider": "Amazon S3",
            "aboutKey": "widgetSourceAboutObjectStorage"
        })
    );

    let exact =
        classify_widget_source("https://tiles.customer-maps.com", SourceProvenance::Runtime)
            .unwrap();
    assert_eq!(
        serde_json::to_value(&exact).unwrap(),
        json!({
            "kind": "exact", "level": "external", "host": "tiles.customer-maps.com",
            "emphasis": "customer-maps.com"
        })
    );
}

#[test]
fn rejections_carry_stable_codes() {
    let rejected =
        |source: &str, provenance| classify_widget_source(source, provenance).unwrap_err();
    assert_eq!(
        rejected("https://*.example.org", SourceProvenance::Runtime),
        WidgetSourceRejection::Source(CspSourceRejection::Wildcard)
    );
    assert_eq!(
        rejected("https://*.co.uk", SourceProvenance::Declared).code(),
        "wildcard-public-suffix"
    );
    assert_eq!(rejected("", SourceProvenance::Declared).code(), "empty");
    assert_eq!(
        rejected("https://Example.com", SourceProvenance::Declared).code(),
        "uppercase"
    );
    assert_eq!(
        WidgetSourceRejection::DataUnavailable.code(),
        "source-data-unavailable"
    );
    assert_eq!(
        WidgetSourceRejection::WildcardPublicSuffix.to_string(),
        "wildcard base is a public suffix or spans public suffixes"
    );
}

#[test]
fn psl_versions_parse_to_unix_seconds() {
    assert_eq!(
        psl_version_unix_secs("2026-09-15_10-18-26_UTC"),
        Some(1_789_467_506)
    );
    assert_eq!(
        psl_version_unix_secs("2000-02-29_00-00-00_UTC"),
        Some(951_782_400)
    );
    assert_eq!(psl_version_unix_secs("1970-01-01_00-00-00_UTC"), Some(0));
    assert_eq!(psl_version_unix_secs("2026-13-15_10-18-26_UTC"), None);
    assert_eq!(psl_version_unix_secs("2026-09-15"), None);
    assert!(SourceIndex::build("# psl-version: soon\ni com\n", TEST_CATALOG).is_err());
    assert!(
        SourceIndex::build(
            "# psl-version: 2026-01-01_00-00-00_UTC\ni Com\n",
            TEST_CATALOG
        )
        .is_err()
    );
    assert!(SourceIndex::build("i com\n", TEST_CATALOG).is_err());
}

#[test]
fn describe_network_aggregates_levels() {
    let index = embedded();
    let now = index.psl_unix_secs + DAY;
    let purposes = [
        purpose(
            "Loads globe terrain, imagery and 3D tiles from Cesium ion",
            &[
                (CspDirective::ImgSrc, "https://assets.ion.cesium.com"),
                (CspDirective::ConnectSrc, "https://assets.ion.cesium.com"),
                (CspDirective::ConnectSrc, "https://api.cesium.com"),
                (CspDirective::ConnectSrc, "https://api.cesium.com"),
            ],
            &[],
        ),
        purpose(
            "Loads map layers from Amazon S3 buckets in Frankfurt",
            &[(
                CspDirective::ConnectSrc,
                "https://*.s3.eu-central-1.amazonaws.com",
            )],
            &[],
        ),
        purpose(
            "Loads map tiles from tile servers given to it at runtime",
            &[],
            &["tileUrl"],
        ),
        purpose(
            "Loads fonts",
            &[(CspDirective::FontSrc, "https://fonts.gstatic.com")],
            &[],
        ),
    ];
    let runtime = [
        runtime_input(
            2,
            "tileUrl",
            CspDirective::ImgSrc,
            "https://a.tiles.customer-maps.com",
        ),
        runtime_input(
            2,
            "tileUrl",
            CspDirective::ConnectSrc,
            "https://a.tiles.customer-maps.com",
        ),
    ];
    let network = describe_network_with(index, &purposes, &runtime, now).unwrap();
    assert_eq!(network.level, WidgetSourceLevel::Broad);
    assert!(!network.stale);
    assert_eq!(network.catalog_version, index.catalog.catalog_version);
    assert_eq!(network.psl_version, index.psl_version);

    let levels: Vec<_> = network
        .purposes
        .iter()
        .map(|purpose| purpose.level)
        .collect();
    assert_eq!(
        levels,
        [
            WidgetSourceLevel::Shared,
            WidgetSourceLevel::Broad,
            WidgetSourceLevel::External,
            WidgetSourceLevel::Known
        ]
    );

    let cesium = &network.purposes[0].sources;
    assert_eq!(cesium.len(), 2);
    assert_eq!(cesium[0].source, "https://api.cesium.com");
    assert_eq!(cesium[0].directives, [CspDirective::ConnectSrc]);
    assert_eq!(
        cesium[1].directives,
        [CspDirective::ConnectSrc, CspDirective::ImgSrc]
    );

    let runtime_source = &network.purposes[2].sources[0];
    assert_eq!(runtime_source.origin, SourceProvenance::Runtime);
    assert_eq!(runtime_source.slot.as_deref(), Some("tileUrl"));
    assert_eq!(runtime_source.class.kind, WidgetSourceKind::Exact);
    assert_eq!(network.purposes[2].inputs, ["tileUrl"]);

    let serialized = serde_json::to_value(&network).unwrap();
    assert_eq!(serialized["purposes"][2]["sources"][0]["slot"], "tileUrl");
    assert_eq!(
        serialized["purposes"][2]["sources"][0]["emphasis"],
        "customer-maps.com"
    );
    assert_eq!(
        serialized["purposes"][1]["sources"][0]["kind"],
        "shared-wildcard"
    );
    assert!(serialized["purposes"][0].get("inputs").is_none());

    let quiet = describe_network_with(index, &purposes[3..], &[], now).unwrap();
    assert_eq!(quiet.level, WidgetSourceLevel::Known);
    let inputs_only = describe_network_with(index, &purposes[2..3], &[], now).unwrap();
    assert_eq!(inputs_only.level, WidgetSourceLevel::External);
    assert!(inputs_only.purposes[0].sources.is_empty());
}

#[test]
fn describe_network_fails_closed_to_none() {
    let index = embedded();
    let now = index.psl_unix_secs;
    let valid = purpose(
        "Loads fonts",
        &[(CspDirective::FontSrc, "https://fonts.gstatic.com")],
        &[],
    );
    assert!(describe_network_with(index, &[], &[], now).is_none());
    assert!(
        describe_network_with(
            index,
            std::slice::from_ref(&valid),
            &[runtime_input(
                1,
                "x",
                CspDirective::ImgSrc,
                "https://a.example.org"
            )],
            now
        )
        .is_none()
    );
    let broken = purpose(
        "Loads tiles",
        &[(CspDirective::ImgSrc, "https://*.co.uk")],
        &[],
    );
    assert!(describe_network_with(index, &[valid.clone(), broken], &[], now).is_none());
    let runtime_wildcard =
        runtime_input(0, "tileUrl", CspDirective::ImgSrc, "https://*.example.org");
    assert!(
        describe_network_with(
            index,
            std::slice::from_ref(&valid),
            &[runtime_wildcard],
            now
        )
        .is_none()
    );
    assert!(describe_network(std::slice::from_ref(&valid), &[], now).is_some());
}

#[test]
fn runtime_platform_storage_scopes_are_this_apps_files() {
    let index = embedded();
    let now = index.psl_unix_secs;
    let layers = purpose("Shows stored map layers", &[], &["layers"]);
    let scope = "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app-1/";
    let network = describe_network_with(
        index,
        std::slice::from_ref(&layers),
        &[runtime_input(0, "layers", CspDirective::ImgSrc, scope)],
        now,
    )
    .expect("a platform storage scope must not make the network block unavailable");
    let source = &network.purposes[0].sources[0];
    assert_eq!(source.source, scope);
    assert_eq!(source.origin, SourceProvenance::Runtime);
    assert_eq!(source.class.kind, WidgetSourceKind::PlatformStorage);
    assert_eq!(source.class.level, WidgetSourceLevel::Known);
    assert_eq!(source.class.host, "s3.eu-central-1.amazonaws.com");
    assert_eq!(network.purposes[0].level, WidgetSourceLevel::Known);
}

#[test]
fn stale_data_raises_subdomain_kinds_to_broad() {
    let index = embedded();
    let purposes = [purpose(
        "Loads map tiles",
        &[
            (CspDirective::ImgSrc, "https://*.customer-maps.com"),
            (CspDirective::ImgSrc, "https://*.octocat.github.io"),
            (CspDirective::ImgSrc, "https://tiles.customer-maps.com"),
            (
                CspDirective::ImgSrc,
                "https://mybucket.s3.eu-central-1.amazonaws.com",
            ),
        ],
        &[],
    )];
    let levels = |now: i64| {
        let network = describe_network_with(index, &purposes, &[], now).unwrap();
        let levels: Vec<_> = network.purposes[0]
            .sources
            .iter()
            .map(|source| (source.source.clone(), source.class.level))
            .collect();
        (network.stale, network.level, levels)
    };

    let fresh_edge = index.psl_unix_secs + WIDGET_SOURCE_STALE_AFTER_SECS;
    let (stale, level, fresh) = levels(fresh_edge);
    assert!(!stale);
    assert_eq!(level, WidgetSourceLevel::External);
    assert!(
        fresh
            .iter()
            .all(|(_, level)| *level == WidgetSourceLevel::External)
    );

    let (stale, level, raised) = levels(fresh_edge + 1);
    assert!(stale);
    assert_eq!(level, WidgetSourceLevel::Broad);
    let level_of = |source: &str| raised.iter().find(|(s, _)| s == source).unwrap().1;
    assert_eq!(
        level_of("https://*.customer-maps.com"),
        WidgetSourceLevel::Broad
    );
    assert_eq!(
        level_of("https://*.octocat.github.io"),
        WidgetSourceLevel::Broad
    );
    assert_eq!(
        level_of("https://tiles.customer-maps.com"),
        WidgetSourceLevel::External
    );
    assert_eq!(
        level_of("https://mybucket.s3.eu-central-1.amazonaws.com"),
        WidgetSourceLevel::External
    );
    assert_eq!(WIDGET_SOURCE_STALE_AFTER_SECS, 180 * DAY);
}

#[test]
fn name_character_rules() {
    assert!(reason_characters_allowed("Amazon S3"));
    assert!(reason_characters_allowed(
        "Cesium ion (assets), maps & tiles/imagery: A-Z; it's"
    ));
    assert!(reason_characters_allowed("नमस्ते"));
    assert!(!reason_characters_allowed("Tiles 🚀"));
    assert!(!reason_characters_allowed("\"Quoted\""));
    assert!(!reason_characters_allowed("Tab\there"));
    assert!(!reason_characters_allowed("e\u{301}\u{301}\u{301}"));
    assert!(reason_characters_allowed("e\u{301}\u{301}"));
    assert!(!reason_characters_allowed("a\u{200D}b"));
    assert!(reason_characters_allowed("क\u{200D}ष"));
    assert!(!reason_characters_allowed("Git\u{034F}Hub"));
    assert!(!reason_characters_allowed("GitHub\u{3164}"));

    assert!(has_mixed_script_word("Flοw-Like"));
    assert!(has_mixed_script_word("cesium.cοm"));
    assert!(!has_mixed_script_word("Café Zürich"));
    assert!(!has_mixed_script_word("Москва tiles"));
}
