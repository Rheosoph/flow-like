use tower_http::cors::CorsLayer;

pub fn cors_from_env() -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let configured = std::env::var("CORS_ALLOWED_ORIGINS")?;
    cors_for_origins(&configured)
}

fn cors_for_origins(configured: &str) -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let mut origins = Vec::new();
    for origin in configured
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let url = url::Url::parse(origin)?;
        let desktop_origin =
            url.scheme() == "tauri" && url.host_str() == Some("localhost") && url.port().is_none();
        if origin.contains('*')
            || !(matches!(url.scheme(), "http" | "https") || desktop_origin)
            || url.host_str().is_none()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(
                "CORS_ALLOWED_ORIGINS must contain HTTP(S) origins or tauri://localhost".into(),
            );
        }
        let canonical = if desktop_origin {
            "tauri://localhost".to_string()
        } else {
            url.origin().ascii_serialization()
        };
        origins.push(canonical.parse::<axum::http::HeaderValue>()?);
    }
    if origins.is_empty() {
        return Err("CORS_ALLOWED_ORIGINS must not be empty".into());
    }
    Ok(CorsLayer::new()
        .allow_origin(origins)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any))
}

#[cfg(test)]
mod tests {
    use super::cors_for_origins;

    #[test]
    fn permits_desktop_and_http_origins_without_paths_or_wildcards() {
        assert!(
            cors_for_origins("tauri://localhost,http://localhost:3001,https://app.example.test/")
                .is_ok()
        );
        for value in [
            "",
            "*",
            "https://*.example.test",
            "https://app.example.test/path",
            "https://user:secret@app.example.test",
            "https://app.example.test?q=x",
            "tauri://remote",
            "tauri://localhost:80",
        ] {
            assert!(cors_for_origins(value).is_err(), "{value}");
        }
    }
}

pub async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
