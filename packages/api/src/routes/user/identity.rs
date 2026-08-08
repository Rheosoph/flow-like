//! Derivation and normalization of human-facing identity fields.
//!
//! Federated identity providers surface their pool-internal handle as `username`
//! (`google_110293…`, `signinwithapple_001234.9f8e…`). That value is load-bearing —
//! [`crate::user_management::UserManagement::get_attribute`] uses it as the Cognito
//! pool key — but it must never reach a user-facing surface. Everything a human
//! reads is derived here instead, from the standard OIDC profile claims.

use crate::middleware::jwt::UserInfo;

const MAX_DISPLAY_NAME_LEN: usize = 96;
const MAX_HANDLE_LEN: usize = 64;

/// Minimum length before a substring search is allowed to run.
pub const MIN_SEARCH_LEN: usize = 2;

const IDP_HANDLE_PREFIXES: &[&str] = &[
    "google_",
    "signinwithapple_",
    "apple_",
    "facebook_",
    "loginwithamazon_",
    "amazon_",
    "microsoft_",
    "azuread_",
    "github_",
    "gitlab_",
    "twitter_",
    "linkedin_",
    "okta_",
    "auth0_",
    "keycloak_",
    "oidc_",
    "saml_",
    "cognito_",
];

/// True when a string is an identity-provider handle or an opaque identifier, i.e.
/// something that must never be shown to a human as a name.
pub fn is_idp_handle(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();

    // A provider prefix only means a linked account when what follows is an id
    // rather than words — `apple_pie_lover` is a person, `apple_001234` is not.
    if let Some(prefix) = IDP_HANDLE_PREFIXES
        .iter()
        .find(|prefix| lower.starts_with(**prefix))
    {
        let suffix = &lower[prefix.len()..];
        return suffix.chars().any(|c| c.is_ascii_digit()) || is_opaque_identifier(suffix);
    }

    is_opaque_identifier(&lower)
}

fn is_opaque_identifier(lower: &str) -> bool {
    if is_uuid(lower) {
        return true;
    }

    let alphanumeric = lower.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    if alphanumeric == 0 {
        return true;
    }

    // Google subs are long digit runs.
    if alphanumeric >= 10 && lower.chars().all(|c| !c.is_ascii_alphabetic()) {
        return true;
    }

    // Apple subs look like `001234.9f8e7d6c5b4a….0123`; Cognito subs are hex blobs.
    if lower.len() >= 20
        && lower
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, '-' | '.' | '_'))
        && lower.chars().any(|c| c.is_ascii_digit())
    {
        return true;
    }

    false
}

fn is_uuid(lower: &str) -> bool {
    if lower.len() != 36 {
        return false;
    }

    lower.chars().enumerate().all(|(index, c)| match index {
        8 | 13 | 18 | 23 => c == '-',
        _ => c.is_ascii_hexdigit(),
    })
}

/// Apple hands out relay addresses that are stable but meaningless to a human.
pub fn is_private_relay_email(email: &str) -> bool {
    let lower = email.trim().to_ascii_lowercase();
    lower.ends_with("@privaterelay.appleid.com") || lower.ends_with("@appleid.com")
}

/// Trims, collapses whitespace, strips control characters and caps the length.
/// Returns `None` when nothing usable is left.
pub fn sanitize_display_name(value: &str) -> Option<String> {
    sanitize_bounded(value, MAX_DISPLAY_NAME_LEN)
}

fn sanitize_bounded(value: &str, max_len: usize) -> Option<String> {
    let collapsed = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if collapsed.is_empty() {
        return None;
    }

    let bounded = if collapsed.chars().count() > max_len {
        collapsed.chars().take(max_len).collect::<String>()
    } else {
        collapsed
    };

    let bounded = bounded.trim_end().to_string();
    (!bounded.is_empty()).then_some(bounded)
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `felix.schultz@example.com` → `Felix Schultz`. Returns `None` for relay
/// addresses and for local parts that are themselves opaque.
pub fn humanize_email_local_part(email: &str) -> Option<String> {
    if is_private_relay_email(email) {
        return None;
    }

    let local = email.split('@').next()?.trim();
    let local = local.split('+').next().unwrap_or(local);
    if local.is_empty() || is_opaque_identifier(&local.to_ascii_lowercase()) {
        return None;
    }

    let words = local
        .split(|c: char| matches!(c, '.' | '_' | '-'))
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .collect::<Vec<_>>();

    if words.is_empty() {
        return None;
    }

    sanitize_display_name(&words.join(" "))
}

/// The handle we are willing to show publicly, or `None` when the provider only
/// gave us its internal one.
pub fn derive_public_handle(info: &UserInfo) -> Option<String> {
    let candidate = info.preferred_username.as_deref()?;
    if is_idp_handle(candidate) {
        return None;
    }
    sanitize_bounded(candidate, MAX_HANDLE_LEN)
}

/// Best available human-readable name, in descending order of quality:
/// `name` → `given_name family_name` → public handle → `nickname` → email local part.
pub fn derive_display_name(info: &UserInfo) -> Option<String> {
    if let Some(name) = info.name.as_deref().and_then(sanitize_display_name) {
        return Some(name);
    }

    let given = info.given_name.as_deref().and_then(sanitize_display_name);
    let family = info.family_name.as_deref().and_then(sanitize_display_name);
    match (given, family) {
        (Some(given), Some(family)) => return sanitize_display_name(&format!("{given} {family}")),
        (Some(given), None) => return Some(given),
        (None, Some(family)) => return Some(family),
        (None, None) => {}
    }

    if let Some(handle) = derive_public_handle(info) {
        return Some(handle);
    }

    if let Some(nickname) = info.nickname.as_deref()
        && !is_idp_handle(nickname)
        && let Some(nickname) = sanitize_display_name(nickname)
    {
        return Some(nickname);
    }

    info.email.as_deref().and_then(humanize_email_local_part)
}

/// Escapes the LIKE metacharacters so a user typing `%` searches for a literal `%`.
/// Pairs with `ESCAPE '\'` on the query side.
pub fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 8);
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// A normalized search term. `raw` is what the user typed (trimmed), `lower` is
/// what ranking compares against.
#[derive(Debug, Clone)]
pub struct SearchTerm {
    pub raw: String,
    pub lower: String,
}

impl SearchTerm {
    pub fn parse(query: &str) -> Option<Self> {
        let raw = query
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        if raw.chars().count() < MIN_SEARCH_LEN || raw.chars().count() > 200 {
            return None;
        }

        let lower = raw.to_lowercase();
        Some(SearchTerm { raw, lower })
    }

    pub fn like_pattern(&self) -> String {
        format!("%{}%", escape_like_pattern(&self.raw))
    }
}

const WEIGHT_NAME: i32 = 40;
const WEIGHT_PREFERRED_USERNAME: i32 = 38;
const WEIGHT_EMAIL: i32 = 30;
const WEIGHT_ID: i32 = 20;
const WEIGHT_USERNAME: i32 = 10;

fn match_bonus(haystack: &str, needle: &str) -> Option<i32> {
    if haystack == needle {
        return Some(1000);
    }
    if haystack.starts_with(needle) {
        return Some(600);
    }
    if haystack
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| !word.is_empty() && word.starts_with(needle))
    {
        return Some(450);
    }
    if haystack.contains(needle) {
        return Some(250);
    }
    None
}

/// The fields a candidate is ranked on. Borrowed so ranking stays allocation-free.
#[derive(Debug, Default, Clone, Copy)]
pub struct RankableUser<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub preferred_username: Option<&'a str>,
    pub username: Option<&'a str>,
    pub email: Option<&'a str>,
    pub has_avatar: bool,
}

/// Higher is a better match. Ranks by how the term matched (exact ≫ prefix ≫
/// word-prefix ≫ substring) and on which field, then nudges complete profiles up
/// so a real person outranks a bare shell row.
pub fn score_candidate(candidate: &RankableUser<'_>, term: &SearchTerm) -> i32 {
    let needle = &term.lower;
    let fields = [
        (candidate.name, WEIGHT_NAME),
        (candidate.preferred_username, WEIGHT_PREFERRED_USERNAME),
        (candidate.email, WEIGHT_EMAIL),
        (Some(candidate.id), WEIGHT_ID),
        (candidate.username, WEIGHT_USERNAME),
    ];

    let mut best = 0;
    for (value, weight) in fields {
        let Some(value) = value else { continue };
        let lowered = value.to_lowercase();
        if let Some(bonus) = match_bonus(&lowered, needle) {
            best = best.max(bonus + weight);
        }
    }

    if best == 0 {
        return 0;
    }

    if candidate.name.is_some() {
        best += 6;
    }
    if candidate.preferred_username.is_some() {
        best += 4;
    }
    if candidate.has_avatar {
        best += 2;
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: Option<&str>, given: Option<&str>, family: Option<&str>) -> UserInfo {
        UserInfo {
            sub: "sub".to_string(),
            email: None,
            email_verified: None,
            name: name.map(str::to_string),
            given_name: given.map(str::to_string),
            family_name: family.map(str::to_string),
            middle_name: None,
            nickname: None,
            preferred_username: None,
            phone_number: None,
            phone_number_verified: None,
            picture: None,
            birthdate: None,
            updated_at: None,
            username: None,
            extra: serde_json::Value::Null,
        }
    }

    #[test]
    fn detects_federated_handles() {
        assert!(is_idp_handle("google_110293847561029384756"));
        assert!(is_idp_handle("signinwithapple_001234.9f8e7d6c5b4a.0123"));
        assert!(is_idp_handle("Google_1102938"));
        assert!(is_idp_handle("42c52474-5081-70d7-2b23-4bd8c38d8fb0"));
        assert!(is_idp_handle("110293847561029384756"));
        assert!(is_idp_handle("   "));
    }

    #[test]
    fn keeps_real_handles() {
        assert!(!is_idp_handle("felix"));
        assert!(!is_idp_handle("felix.schultz"));
        assert!(!is_idp_handle("googler"));
        assert!(!is_idp_handle("apple_pie_lover"));
        assert!(!is_idp_handle("decaf"));
    }

    #[test]
    fn prefers_full_name_then_given_family() {
        assert_eq!(
            derive_display_name(&info(Some("Felix Schultz"), None, None)).as_deref(),
            Some("Felix Schultz")
        );
        assert_eq!(
            derive_display_name(&info(None, Some("Felix"), Some("Schultz"))).as_deref(),
            Some("Felix Schultz")
        );
        assert_eq!(
            derive_display_name(&info(None, Some("Felix"), None)).as_deref(),
            Some("Felix")
        );
        assert_eq!(derive_display_name(&info(None, None, None)), None);
    }

    #[test]
    fn falls_back_to_email_but_never_to_relay_or_idp_handle() {
        let mut with_email = info(None, None, None);
        with_email.email = Some("felix.schultz@example.com".to_string());
        assert_eq!(
            derive_display_name(&with_email).as_deref(),
            Some("Felix Schultz")
        );

        let mut relay = info(None, None, None);
        relay.email = Some("kj2h3g4jh2g@privaterelay.appleid.com".to_string());
        assert_eq!(derive_display_name(&relay), None);

        let mut federated = info(None, None, None);
        federated.username = Some("google_110293847561029384756".to_string());
        federated.preferred_username = Some("google_110293847561029384756".to_string());
        assert_eq!(derive_display_name(&federated), None);
        assert_eq!(derive_public_handle(&federated), None);
    }

    #[test]
    fn strips_plus_tags_and_control_characters() {
        assert_eq!(
            humanize_email_local_part("felix+newsletter@example.com").as_deref(),
            Some("Felix")
        );
        assert_eq!(
            sanitize_display_name("  Felix\u{0}\n  Schultz  ").as_deref(),
            Some("Felix Schultz")
        );
        assert_eq!(sanitize_display_name("\u{0}\u{0}"), None);
    }

    #[test]
    fn caps_display_name_length() {
        let long = "a".repeat(500);
        let sanitized = sanitize_display_name(&long).unwrap();
        assert_eq!(sanitized.chars().count(), MAX_DISPLAY_NAME_LEN);
    }

    #[test]
    fn escapes_like_metacharacters() {
        assert_eq!(escape_like_pattern("100%"), "100\\%");
        assert_eq!(escape_like_pattern("a_b"), "a\\_b");
        assert_eq!(escape_like_pattern("c:\\x"), "c:\\\\x");
        assert_eq!(escape_like_pattern("felix"), "felix");
    }

    #[test]
    fn rejects_terms_that_are_too_short() {
        assert!(SearchTerm::parse("f").is_none());
        assert!(SearchTerm::parse("   ").is_none());
        assert!(SearchTerm::parse("fe").is_some());
        assert_eq!(
            SearchTerm::parse("  Felix  Schultz ").unwrap().raw,
            "Felix Schultz"
        );
    }

    #[test]
    fn ranks_exact_above_prefix_above_substring() {
        let term = SearchTerm::parse("felix").unwrap();
        let exact = RankableUser {
            id: "1",
            name: Some("felix"),
            ..Default::default()
        };
        let prefix = RankableUser {
            id: "2",
            name: Some("felixander"),
            ..Default::default()
        };
        let word = RankableUser {
            id: "3",
            name: Some("Herr Felix Schultz"),
            ..Default::default()
        };
        let substring = RankableUser {
            id: "4",
            name: Some("unfelixlike"),
            ..Default::default()
        };

        assert!(score_candidate(&exact, &term) > score_candidate(&prefix, &term));
        assert!(score_candidate(&prefix, &term) > score_candidate(&word, &term));
        assert!(score_candidate(&word, &term) > score_candidate(&substring, &term));
        assert!(score_candidate(&substring, &term) > 0);
    }

    #[test]
    fn ranks_name_above_internal_username() {
        let term = SearchTerm::parse("felix").unwrap();
        let by_name = RankableUser {
            id: "1",
            name: Some("Felix"),
            ..Default::default()
        };
        let by_username = RankableUser {
            id: "2",
            username: Some("felix"),
            ..Default::default()
        };
        assert!(score_candidate(&by_name, &term) > score_candidate(&by_username, &term));
    }

    #[test]
    fn scores_zero_when_nothing_matches() {
        let term = SearchTerm::parse("felix").unwrap();
        let candidate = RankableUser {
            id: "1",
            name: Some("Someone Else"),
            ..Default::default()
        };
        assert_eq!(score_candidate(&candidate, &term), 0);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let term = SearchTerm::parse("FELIX").unwrap();
        let candidate = RankableUser {
            id: "1",
            name: Some("felix schultz"),
            ..Default::default()
        };
        assert!(score_candidate(&candidate, &term) > 0);
    }
}
