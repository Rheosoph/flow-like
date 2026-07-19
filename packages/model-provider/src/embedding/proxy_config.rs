use std::sync::RwLock;

const DEFAULT_API_BASE_URL: &str = "http://localhost:3000";

/// Runtime override for the API base URL used by proxied model calls.
///
/// Executors (AWS Lambda, Kubernetes) receive the base URL through the
/// `API_BASE_URL` environment variable. Hosts that only learn their backend at
/// runtime — the desktop app, which resolves it from the active hub profile and
/// can switch profiles mid-session — set it through [`set_api_base_url`].
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

/// Resolve the API base URL: runtime override, then `API_BASE_URL`, then the
/// local development default.
pub fn api_base_url() -> String {
    if let Ok(guard) = API_BASE_URL_OVERRIDE.read()
        && let Some(url) = guard.as_deref()
    {
        return url.to_string();
    }

    std::env::var("API_BASE_URL")
        .ok()
        .and_then(|url| normalize(&url))
        .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())
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
    fn override_takes_precedence_and_ignores_blank_input() {
        set_api_base_url("https://api.example.test/");
        assert_eq!(api_base_url(), "https://api.example.test");

        set_api_base_url("  ");
        assert_eq!(api_base_url(), "https://api.example.test");
    }
}
