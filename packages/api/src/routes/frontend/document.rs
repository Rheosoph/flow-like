use std::{sync::LazyLock, time::Duration};

use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
};
use regex::Regex;

use crate::{error::ApiError, state::AppState};

use super::{frontend_base_url, frontend_location, metadata};

const SHELL_LIMIT: usize = 8 * 1024 * 1024;

pub(super) const REDIRECT_SCRIPT: &str = r#"const destination = new URL(document.getElementById("open-app").href);
if (!destination.hash) destination.hash = window.location.hash;
window.location.replace(destination.href);"#;

/// The static entry point has no app identifier, so fetching it cannot call this handler again.
fn shell_url(base: &str) -> Result<reqwest::Url, ApiError> {
    let validated = frontend_location(base, "_", "/", None)?;
    let mut url = reqwest::Url::parse(&validated)
        .map_err(|_| ApiError::service_unavailable("Invalid hosted frontend URL"))?;
    url.path_segments_mut()
        .map_err(|_| ApiError::service_unavailable("Invalid hosted frontend URL"))?
        .pop();
    Ok(url)
}

fn selected_shell_url(configured: Option<&str>, base: &str) -> Result<reqwest::Url, ApiError> {
    let Some(configured) = configured.filter(|value| !value.trim().is_empty()) else {
        return shell_url(base);
    };
    let invalid = || {
        ApiError::service_unavailable(
            "FRONTEND_SHELL_URL must be an absolute HTTP URL to the generic /a entry point",
        )
    };
    let url = reqwest::Url::parse(configured).map_err(|_| invalid())?;
    let segments: Vec<_> = url.path_segments().ok_or_else(invalid)?.collect();
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || configured.chars().any(char::is_control)
        || configured.contains('\\')
        || url.path().contains('%')
        || segments.last() != Some(&"a")
        || segments[..segments.len().saturating_sub(1)].contains(&"a")
    {
        return Err(invalid());
    }
    Ok(url)
}

pub(super) async fn document(
    state: AppState,
    app_id: String,
    route: &str,
) -> Result<Response, ApiError> {
    let route = super::normalize_route_path(route);
    let metadata = metadata::load(&state, &app_id, &route).await?;
    let hosted_base = std::env::var("FRONTEND_BASE_URL").ok();
    let web_base = std::env::var("FRONTEND_URL").ok();
    let base = frontend_base_url(hosted_base.as_deref(), web_base.as_deref())?;
    let canonical = frontend_location(base, &app_id, &route, None)?;
    let configured_shell = std::env::var("FRONTEND_SHELL_URL").ok();
    let shell_url = selected_shell_url(configured_shell.as_deref(), base)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::service_unavailable("Hosted frontend is unavailable"))?;
    // Only deployment configuration chooses the origin; viewer headers and query values
    // never accompany this request for the shared static shell.
    let mut response = client
        .get(shell_url)
        .header(header::ACCEPT, "text/html")
        .send()
        .await
        .map_err(|_| ApiError::service_unavailable("Hosted frontend is unavailable"))?;
    if !response.status().is_success()
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/html"))
            })
        || response
            .content_length()
            .is_some_and(|size| size > SHELL_LIMIT as u64)
    {
        return Err(ApiError::service_unavailable(
            "Hosted frontend is unavailable",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::service_unavailable("Hosted frontend is unavailable"))?
    {
        if bytes.len() + chunk.len() > SHELL_LIMIT {
            return Err(ApiError::service_unavailable(
                "Hosted frontend exceeds the response limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let shell = String::from_utf8(bytes)
        .map_err(|_| ApiError::service_unavailable("Hosted frontend HTML is invalid"))?;
    let html = rewrite_shell(&shell, &metadata, &canonical)?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "strict-origin-when-cross-origin"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Html(html),
    )
        .into_response())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn metadata_head(metadata: &metadata::FrontendMetadata, canonical: &str) -> String {
    let name = escape(&metadata.name);
    let description = escape(&metadata.description);
    let canonical = escape(canonical);
    let mut head = format!(
        r#"<title>{name}</title>
<meta name="description" content="{description}">
<link rel="canonical" href="{canonical}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="{name}">
<meta property="og:title" content="{name}">
<meta property="og:description" content="{description}">
<meta property="og:url" content="{canonical}">
<meta name="twitter:card" content="{}">
<meta name="twitter:title" content="{name}">
<meta name="twitter:description" content="{description}">"#,
        escape(metadata.card),
    );
    if let Some(image) = &metadata.image {
        let image = escape(image);
        head.push_str(&format!(
            r#"
<meta property="og:image" content="{image}">
<meta property="og:image:alt" content="{name}">
<meta name="twitter:image" content="{image}">
<meta name="twitter:image:alt" content="{name}">"#,
        ));
    }
    if let Some(icon) = &metadata.icon {
        head.push_str(&format!(
            r#"
<link rel="icon" href="{}">"#,
            escape(icon),
        ));
    }
    head
}

// This reads the generated static shell, not user HTML. Consume comments and raw-text
// elements whole so strings resembling metadata inside scripts remain untouched.
static HEAD_TOKENS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)<!--.*?-->|<script\b(?:[^>"']|"[^"]*"|'[^']*')*>.*?</script\s*>|<style\b(?:[^>"']|"[^"]*"|'[^']*')*>.*?</style\s*>|<title\b(?:[^>"']|"[^"]*"|'[^']*')*>.*?</title\s*>|</?head\b(?:[^>"']|"[^"]*"|'[^']*')*>|<(?:meta|link)\b(?:[^>"']|"[^"]*"|'[^']*')*>"#,
    )
    .expect("static HTML token pattern")
});

static ATTRIBUTES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)([^\s=/>]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)
        .expect("static HTML attribute pattern")
});

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    ATTRIBUTES.captures_iter(tag).find_map(|capture| {
        capture[1].eq_ignore_ascii_case(name).then(|| {
            capture
                .get(2)
                .or_else(|| capture.get(3))
                .or_else(|| capture.get(4))
                .expect("attribute value")
                .as_str()
        })
    })
}

fn replaces_metadata(tag: &str, has_icon: bool) -> bool {
    let lower = tag.to_ascii_lowercase();
    if lower.starts_with("<title") {
        return true;
    }
    if lower.starts_with("<meta") {
        return [attribute(tag, "name"), attribute(tag, "property")]
            .into_iter()
            .flatten()
            .any(|value| {
                let value = value.to_ascii_lowercase();
                value == "description" || value.starts_with("og:") || value.starts_with("twitter:")
            });
    }
    if lower.starts_with("<link") {
        return attribute(tag, "rel").is_some_and(|value| {
            value.split_ascii_whitespace().any(|part| {
                part.eq_ignore_ascii_case("canonical")
                    || (has_icon
                        && (part.eq_ignore_ascii_case("icon")
                            || part.eq_ignore_ascii_case("apple-touch-icon")))
            })
        });
    }
    false
}

fn rewrite_shell(
    shell: &str,
    metadata: &metadata::FrontendMetadata,
    canonical: &str,
) -> Result<String, ApiError> {
    let mut result = String::with_capacity(shell.len() + 1024);
    let mut position = 0;
    let mut in_head = false;
    let mut replaced = false;
    for token in HEAD_TOKENS.find_iter(shell) {
        result.push_str(&shell[position..token.start()]);
        let tag = token.as_str();
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<head") {
            in_head = true;
            result.push_str(tag);
        } else if lower.starts_with("</head") && in_head {
            result.push_str(&metadata_head(metadata, canonical));
            result.push_str(tag);
            in_head = false;
            replaced = true;
        } else if !in_head || !replaces_metadata(tag, metadata.icon.is_some()) {
            result.push_str(tag);
        }
        position = token.end();
    }
    result.push_str(&shell[position..]);
    if !replaced {
        return Err(ApiError::service_unavailable(
            "Hosted frontend HTML has no head",
        ));
    }
    Ok(result)
}

/// Share crawlers read this response; browsers continue to the static app with their query intact.
pub(super) fn render_redirect(
    metadata: &metadata::FrontendMetadata,
    location: &str,
    canonical: &str,
) -> String {
    let head = metadata_head(metadata, canonical);
    let location = escape(location);
    let name = escape(&metadata.name);
    format!(
        r#"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<meta name="referrer" content="no-referrer">
{head}
</head><body>
<a id="open-app" href="{location}">Open {name}</a>
<script>{REDIRECT_SCRIPT}</script>
</body></html>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> metadata::FrontendMetadata {
        metadata::FrontendMetadata {
            name: "Research & \"Sales\" <Team>".into(),
            description: "Compare plans > decide".into(),
            image: Some("https://cdn.example/cover.png?a=1&b=2".into()),
            icon: Some("https://cdn.example/icon.png".into()),
            card: "summary_large_image",
        }
    }

    #[test]
    fn replaces_generic_branding_and_preserves_the_application() {
        let head_script = r#"<script>let x='<meta property="og:title" content="keep">';let y='</head>';</script>"#;
        let body = r#"<body><div id="app">Loading</div><script>self.__next_f.push([1,"<title>keep</title>"])</script></body></html>"#;
        let shell = format!(
            r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><TITLE>Flow-Like</TITLE><meta content='generic' name='description'><meta property="og:image" content="default.png"><meta property="og:image:width" content="1600"><meta NAME="twitter:creator" content="@old"><LINK href="https://old.example" REL="canonical"><link rel="icon" href="old.ico"><link rel="apple-touch-icon" href="old.png"><link rel="stylesheet" href="app.css">{head_script}</head>{body}"#
        );
        let html = rewrite_shell(&shell, &metadata(), "https://web.example/a/app/home").unwrap();
        assert!(html.contains("<title>Research &amp; &quot;Sales&quot; &lt;Team&gt;</title>"));
        assert!(html.contains("Compare plans &gt; decide"));
        assert!(html.contains("https://cdn.example/cover.png?a=1&amp;b=2"));
        assert!(
            html.contains(r#"<meta property="og:url" content="https://web.example/a/app/home">"#)
        );
        assert!(html.contains(r#"<link rel="icon" href="https://cdn.example/icon.png">"#));
        for old in [
            "generic",
            "default.png",
            "og:image:width",
            "@old",
            "https://old.example",
            "old.ico",
            "old.png",
        ] {
            assert!(!html.contains(old), "{old}");
        }
        assert!(html.contains(head_script));
        assert!(html.ends_with(body));
        assert!(html.contains(r#"<link rel="stylesheet" href="app.css">"#));
        assert!(html.contains(r#"<meta name="viewport" content="width=device-width">"#));
    }

    #[test]
    fn apps_without_images_do_not_keep_the_generic_social_image() {
        let mut metadata = metadata();
        metadata.image = None;
        metadata.icon = None;
        metadata.card = "summary";
        let shell = r#"<html><head><meta property="og:image" content="default.png"><meta name="twitter:image" content="default.png"><link rel="icon" href="fallback.ico"></head><body></body></html>"#;
        let html = rewrite_shell(shell, &metadata, "https://web.example/a/app").unwrap();
        assert!(!html.contains("default.png"));
        assert!(!html.contains("og:image"));
        assert!(html.contains(r#"<meta name="twitter:card" content="summary">"#));
        assert!(html.contains("fallback.ico"));
    }

    #[test]
    fn redirect_escapes_values_and_keeps_query_out_of_the_preview_url() {
        let location = "https://web.example/a/app?value=</script>&other=\"quoted\"";
        let canonical = "https://web.example/a/app";
        let html = render_redirect(&metadata(), location, canonical);
        assert!(html.contains("?value=&lt;/script&gt;&amp;other=&quot;quoted&quot;"));
        assert!(html.contains(r#"<meta property="og:url" content="https://web.example/a/app">"#));
        assert_eq!(html.matches("</script>").count(), 1);
        assert!(html.contains(REDIRECT_SCRIPT));
    }

    #[test]
    fn fetches_only_the_generic_shell_under_the_configured_base() {
        assert_eq!(
            shell_url("https://web.example").unwrap().as_str(),
            "https://web.example/a"
        );
        assert_eq!(
            shell_url("https://web.example/prefix/").unwrap().as_str(),
            "https://web.example/prefix/a"
        );
        for base in [
            "file:///tmp",
            "https://user:secret@example.com",
            "https://example.com?x=1",
        ] {
            assert!(shell_url(base).is_err());
        }
        assert!(
            rewrite_shell(
                "<html><body></body></html>",
                &metadata(),
                "https://web.example/a/app"
            )
            .is_err()
        );
    }

    #[test]
    fn internal_shell_origin_does_not_change_the_public_destination() {
        let public_base = "https://web.example/prefix";
        assert_eq!(
            selected_shell_url(Some("http://web:3001/a"), public_base)
                .unwrap()
                .as_str(),
            "http://web:3001/a"
        );
        assert_eq!(
            selected_shell_url(Some("https://internal.example/prefix/a"), public_base)
                .unwrap()
                .as_str(),
            "https://internal.example/prefix/a"
        );
        for configured in [None, Some(" \t")] {
            assert_eq!(
                selected_shell_url(configured, public_base)
                    .unwrap()
                    .as_str(),
                "https://web.example/prefix/a"
            );
        }
        assert_eq!(
            frontend_location(public_base, "app", "/home", None).unwrap(),
            "https://web.example/prefix/a/app/home"
        );
        for configured in [
            "file:///a",
            "http://user:secret@web:3001/a",
            "http://web:3001/a?app=app",
            "http://web:3001/a#app",
            "http://web:3001/a/app",
            "http://web:3001/a/app/a",
            "http://web:3001/%61/app/a",
            "http://web:3001/",
        ] {
            assert!(
                selected_shell_url(Some(configured), public_base).is_err(),
                "{configured}"
            );
        }
    }
}
