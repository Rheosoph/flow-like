use std::sync::RwLock;

const DEFAULT_API_BASE_URL: &str = "http://localhost:8080";

/// Runtime override for the API base URL used by proxied model calls.
///
/// Executors receive the base URL through `API_BASE_URL`. Older deployments
/// may still provide `API_URL`, which remains a compatibility fallback. Hosts
/// that learn their backend at runtime set it through [`set_api_base_url`].
static API_BASE_URL_OVERRIDE: RwLock<Option<String>> = RwLock::new(None);

/// Point proxied model calls at `url`. Empty or whitespace-only input is
/// ignored so a misconfigured profile cannot clear a working endpoint. A URL
/// without a scheme is assumed to be `https`.
pub fn set_api_base_url(url: &str) {
    let Some(normalized) = normalize(url) else {
        return;
    };

    if let Ok(mut guard) = API_BASE_URL_OVERRIDE.write() {
        *guard = Some(normalized);
    }
}

/// Resolve the API base URL: runtime override, `API_BASE_URL`, legacy
/// `API_URL`, then the local development default.
pub fn api_base_url() -> String {
    if let Ok(guard) = API_BASE_URL_OVERRIDE.read()
        && let Some(url) = guard.as_deref()
    {
        return url.to_string();
    }

    resolve_environment(|name| std::env::var(name).ok())
        .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())
}

fn resolve_environment(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["API_BASE_URL", "API_URL"]
        .into_iter()
        .find_map(|name| get(name).and_then(|url| normalize(&url)))
}

fn normalize(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains("://") {
        return Some(trimmed.to_string());
    }

    Some(format!("https://{trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_scheme_and_strips_trailing_slash() {
        assert_eq!(
            normalize("api.flow-like.com/"),
            Some("https://api.flow-like.com".to_string())
        );
        assert_eq!(
            normalize(" http://localhost:8080 "),
            Some("http://localhost:8080".to_string())
        );
    }

    #[test]
    fn normalize_rejects_blank_urls() {
        assert_eq!(normalize("   "), None);
        assert_eq!(normalize("/"), None);
    }

    #[test]
    fn environment_prefers_api_base_url_and_accepts_legacy_api_url() {
        let both = resolve_environment(|name| match name {
            "API_BASE_URL" => Some("https://canonical.example".to_string()),
            "API_URL" => Some("https://legacy.example".to_string()),
            _ => None,
        });
        assert_eq!(both.as_deref(), Some("https://canonical.example"));

        let legacy_only = resolve_environment(|name| {
            (name == "API_URL").then(|| "https://legacy.example/".to_string())
        });
        assert_eq!(legacy_only.as_deref(), Some("https://legacy.example"));
    }

    #[test]
    fn override_takes_precedence_and_ignores_blank_input() {
        set_api_base_url("https://api.example.test/");
        assert_eq!(api_base_url(), "https://api.example.test");

        set_api_base_url("  ");
        assert_eq!(api_base_url(), "https://api.example.test");
    }
}
