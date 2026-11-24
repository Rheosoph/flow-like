//! The public store page's view of an app, limited to what the owner opted in.

mod build;
pub mod get_showcase;
mod outline;
pub mod settings;
pub mod types;

use crate::state::AppState;

/// The store page's languages (`packages/locales/locales/config.json`). Anything else reads
/// English, which keeps the cache to one entry per language.
const LANGUAGES: [&str; 9] = ["de", "en", "es", "fr", "ja", "ko", "pl", "pt-BR", "zh"];

fn language(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|requested| LANGUAGES.iter().find(|known| **known == requested))
        .copied()
        .unwrap_or("en")
}

fn cache_key(app_id: &str, language: &str) -> String {
    format!("showcase:{app_id}:{language}")
}

/// Drop this instance's cached payloads for the app. Other instances expire theirs within the
/// response cache's TTL.
pub(crate) fn evict(state: &AppState, app_id: &str) {
    for language in LANGUAGES {
        state.invalidate_cache(&cache_key(app_id, language));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_languages_read_english() {
        assert_eq!(language(Some("de")), "de");
        assert_eq!(language(Some("pt-BR")), "pt-BR");
        assert_eq!(language(Some("xx")), "en");
        assert_eq!(language(None), "en");
    }
}
