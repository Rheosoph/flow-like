use std::collections::HashSet;

use super::model::{Platform, Viewer};
use crate::error::ApiError;

/// Mirrors `packages/locales/locales/config.json` `languages`; a test keeps the two equal.
pub const SUPPORTED_LOCALES: [&str; 9] = ["de", "en", "es", "fr", "ja", "ko", "pl", "pt-BR", "zh"];
pub const DEFAULT_LANGUAGE: &str = "en";
const LOCALE_PREFIX: &str = "locale:";
const LANGUAGE_TAG_MAX: usize = 35;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudienceTag {
    Everyone,
    Dev,
    SignedIn,
    SignedOut,
    Desktop,
    Web,
    Locale(&'static str),
}

impl AudienceTag {
    pub fn parse(tag: &str) -> Option<Self> {
        match tag {
            "everyone" => Some(Self::Everyone),
            "dev" => Some(Self::Dev),
            "signed_in" => Some(Self::SignedIn),
            "signed_out" => Some(Self::SignedOut),
            "desktop" => Some(Self::Desktop),
            "web" => Some(Self::Web),
            _ => tag
                .strip_prefix(LOCALE_PREFIX)
                .and_then(canonical_locale)
                .map(Self::Locale),
        }
    }
}

pub fn canonical_locale(code: &str) -> Option<&'static str> {
    SUPPORTED_LOCALES
        .into_iter()
        .find(|locale| locale.eq_ignore_ascii_case(code))
}

/// The viewer language for resolution and cache keys: a supported locale (case-insensitive, or the base of a
/// regional variant such as "de-AT"). Absent or unsupported languages fall back to `en`, because clients may
/// ship locales this hub does not know yet; only a value that is not a language tag at all is a 400.
pub fn parse_language(raw: Option<&str>) -> Result<String, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(DEFAULT_LANGUAGE.to_owned());
    };
    if !is_language_tag(raw) {
        return Err(ApiError::bad_request(format!(
            "language '{raw}' is not a language tag such as en, de-AT or pt-BR"
        )));
    }
    Ok(canonical_locale(raw)
        .or_else(|| {
            raw.split_once('-')
                .and_then(|(base, _)| canonical_locale(base))
        })
        .unwrap_or(DEFAULT_LANGUAGE)
        .to_owned())
}

/// `[A-Za-z]{2,3}(-[A-Za-z0-9]{1,8})*`, at most 35 characters.
fn is_language_tag(raw: &str) -> bool {
    let mut parts = raw.split('-');
    raw.len() <= LANGUAGE_TAG_MAX
        && parts.next().is_some_and(|base| {
            (2..=3).contains(&base.len()) && base.bytes().all(|byte| byte.is_ascii_alphabetic())
        })
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

/// Trims the tags and drops repeats of a tag already listed; unknown tags stay for [`validate`] to reject.
pub fn normalize(tags: &mut Vec<String>) {
    let mut seen = HashSet::new();
    tags.retain_mut(|tag| {
        let trimmed = tag.trim();
        if trimmed.len() != tag.len() {
            *tag = trimmed.to_owned();
        }
        AudienceTag::parse(tag).is_none_or(|parsed| seen.insert(parsed))
    });
}

pub fn validate(tags: &[String]) -> Result<(), ApiError> {
    let mut seen = HashSet::new();
    for tag in tags {
        let parsed = AudienceTag::parse(tag).ok_or_else(|| {
            ApiError::bad_request(format!(
                "Unknown audience tag '{tag}'; expected everyone, dev, signed_in, signed_out, desktop, web or locale:<{}>",
                SUPPORTED_LOCALES.join("|")
            ))
        })?;
        seen.insert(parsed);
    }
    if seen.contains(&AudienceTag::Everyone) && seen.len() > 1 {
        return Err(ApiError::bad_request(
            "The audience tag 'everyone' cannot be combined with other tags",
        ));
    }
    if seen.contains(&AudienceTag::SignedIn) && seen.contains(&AudienceTag::SignedOut) {
        return Err(ApiError::bad_request(
            "An audience cannot require both signed_in and signed_out",
        ));
    }
    Ok(())
}

/// Tags are ANDed across dimensions and ORed within one (platform, locale). Unknown tags never match.
pub fn matches(tags: &[String], viewer: &Viewer) -> bool {
    let mut platforms = Vec::new();
    let mut locales = Vec::new();
    for tag in tags {
        match AudienceTag::parse(tag) {
            None => return false,
            Some(AudienceTag::Everyone) => {}
            Some(AudienceTag::Dev) if !viewer.dev => return false,
            Some(AudienceTag::SignedIn) if !viewer.signed_in => return false,
            Some(AudienceTag::SignedOut) if viewer.signed_in => return false,
            Some(AudienceTag::Dev | AudienceTag::SignedIn | AudienceTag::SignedOut) => {}
            Some(AudienceTag::Desktop) => platforms.push(Platform::Desktop),
            Some(AudienceTag::Web) => platforms.push(Platform::Web),
            Some(AudienceTag::Locale(code)) => locales.push(code),
        }
    }
    (platforms.is_empty() || platforms.contains(&viewer.platform))
        && (locales.is_empty()
            || locales
                .iter()
                .any(|code| language_matches(&viewer.language, code)))
}

fn language_matches(language: &str, code: &str) -> bool {
    language.eq_ignore_ascii_case(code)
        || (language.len() > code.len()
            && language.is_char_boundary(code.len())
            && language[..code.len()].eq_ignore_ascii_case(code)
            && language.as_bytes()[code.len()] == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(dev: bool, signed_in: bool, platform: Platform, language: &str) -> Viewer {
        Viewer {
            dev,
            signed_in,
            platform,
            language: language.into(),
        }
    }

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn supported_locales_match_the_locales_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../../locales/locales/config.json")).unwrap();
        let languages: Vec<&str> = config["languages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(languages, SUPPORTED_LOCALES);
    }

    #[test]
    fn audience_matrix() {
        let base = viewer(false, false, Platform::Web, "en");
        let dev = viewer(true, true, Platform::Desktop, "de-AT");
        assert!(matches(&[], &base));
        assert!(matches(&tags(&["everyone"]), &base));
        assert!(!matches(&tags(&["dev"]), &base));
        assert!(matches(&tags(&["dev"]), &dev));
        assert!(!matches(&tags(&["signed_in"]), &base));
        assert!(matches(&tags(&["signed_out"]), &base));
        assert!(!matches(&tags(&["signed_out"]), &dev));
        assert!(!matches(&tags(&["signed_in", "signed_out"]), &base));
        assert!(!matches(&tags(&["signed_in", "signed_out"]), &dev));
        assert!(matches(&tags(&["web"]), &base));
        assert!(!matches(&tags(&["desktop"]), &base));
        assert!(matches(&tags(&["desktop", "web"]), &base));
        assert!(matches(&tags(&["dev", "desktop", "signed_in"]), &dev));
        assert!(!matches(&tags(&["dev", "web"]), &dev));
        for (language, expected) in [
            ("de", true),
            ("de-AT", true),
            ("DE-at", true),
            ("en", false),
            ("dev", false),
        ] {
            let viewer = viewer(false, false, Platform::Web, language);
            assert_eq!(
                matches(&tags(&["locale:de"]), &viewer),
                expected,
                "{language}"
            );
        }
        let brazil = viewer(false, false, Platform::Web, "pt-br");
        assert!(matches(&tags(&["locale:pt-BR"]), &brazil));
        assert!(!matches(
            &tags(&["locale:pt-BR"]),
            &viewer(false, false, Platform::Web, "pt")
        ));
        assert!(matches(&tags(&["locale:de", "locale:en"]), &base));
        assert!(!matches(&tags(&["vip"]), &dev));
        assert!(!matches(&tags(&["locale:xx"]), &base));
        assert!(!matches(&tags(&["everyone", "vip"]), &base));
    }

    #[test]
    fn validation_rejects_unknown_and_conflicting_tags() {
        assert!(validate(&tags(&[])).is_ok());
        assert!(validate(&tags(&["everyone"])).is_ok());
        assert!(
            validate(&tags(&[
                "dev",
                "signed_in",
                "desktop",
                "web",
                "locale:pt-BR"
            ]))
            .is_ok()
        );
        assert!(validate(&tags(&["everyone", "everyone"])).is_ok());
        assert!(validate(&tags(&["everyone", "dev"])).is_err());
        assert!(validate(&tags(&["signed_in", "signed_out"])).is_err());
        assert!(validate(&tags(&["locale:xx"])).is_err());
        assert!(validate(&tags(&["Dev"])).is_err());
    }

    #[test]
    fn normalization_trims_and_drops_repeated_tags() {
        let mut audience = tags(&[" dev ", "locale:de", "dev", "locale:DE", "vip", "vip"]);
        normalize(&mut audience);
        assert_eq!(audience, ["dev", "locale:de", "vip", "vip"]);
        assert!(validate(&audience).is_err());
    }

    #[test]
    fn language_parameter_falls_back_to_supported_locales() {
        assert_eq!(parse_language(None).unwrap(), "en");
        assert_eq!(parse_language(Some(" ")).unwrap(), "en");
        assert_eq!(parse_language(Some("pt-br")).unwrap(), "pt-BR");
        assert_eq!(parse_language(Some("de-AT")).unwrap(), "de");
        assert_eq!(parse_language(Some("ZH")).unwrap(), "zh");
        assert_eq!(parse_language(Some("zh-Hant-TW")).unwrap(), "zh");
        for unsupported in ["xx", "it", "pt-PT", "fil", "sr-Latn-RS"] {
            assert_eq!(
                parse_language(Some(unsupported)).unwrap(),
                "en",
                "{unsupported}"
            );
        }
        let longest = format!("de{}-abcde", "-abcdefgh".repeat(3));
        assert_eq!(longest.len(), LANGUAGE_TAG_MAX);
        assert_eq!(parse_language(Some(&longest)).unwrap(), "de");
        let too_long = format!("{longest}f");
        for malformed in [
            "e",
            "engl",
            "e1",
            "en_US",
            "en-",
            "en--US",
            "-en",
            "en-abcdefghi",
            "en US",
            "de\u{0}",
            "dé",
            &too_long,
        ] {
            assert_eq!(
                parse_language(Some(malformed)).unwrap_err().status(),
                axum::http::StatusCode::BAD_REQUEST,
                "{malformed:?}"
            );
        }
    }
}
