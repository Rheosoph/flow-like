use axum::{Json, Router, extract::Query, routing::get};
use flow_like_types::reqwest;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{error::ApiError, state::AppState};

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(fetch_og_metadata))
}

#[derive(Deserialize, IntoParams)]
pub struct OgQuery {
    /// The URL to fetch Open Graph metadata from
    pub url: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OgMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
    pub favicon: Option<String>,
}

const MAX_BODY_SIZE: usize = 512 * 1024; // 512 KB
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn parse_og_tags(html: &str) -> OgMetadata {
    let html_lower = html.to_lowercase();

    // Collect all <meta ...> tags as (lowercase_tag, original_tag) pairs
    let meta_tags: Vec<(&str, &str)> = {
        let mut tags = Vec::new();
        let mut search_from = 0;
        while let Some(start) = html_lower[search_from..].find("<meta") {
            let abs_start = search_from + start;
            if let Some(end) = html_lower[abs_start..].find('>') {
                let abs_end = abs_start + end + 1;
                tags.push((&html_lower[abs_start..abs_end], &html[abs_start..abs_end]));
                search_from = abs_end;
            } else {
                break;
            }
        }
        tags
    };

    fn get_attr_value<'a>(tag_lower: &str, tag_orig: &'a str, attr: &str) -> Option<&'a str> {
        // Try: attr="value", attr='value', attr=value (unquoted)
        for prefix in &[format!("{}=\"", attr), format!("{}='", attr)] {
            if let Some(pos) = tag_lower.find(prefix.as_str()) {
                let delim = prefix.chars().last().unwrap();
                let val_start = pos + prefix.len();
                if let Some(val_end) = tag_orig[val_start..].find(delim) {
                    let v = tag_orig[val_start..val_start + val_end].trim();
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }
        // Unquoted: attr=value (terminated by space or >)
        let unquoted = format!("{}=", attr);
        if let Some(pos) = tag_lower.find(unquoted.as_str()) {
            let val_start = pos + unquoted.len();
            let rest = &tag_orig[val_start..];
            // Must not start with a quote (already handled above)
            if !rest.starts_with('"') && !rest.starts_with('\'') {
                let val_end = rest
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .unwrap_or(rest.len());
                let v = rest[..val_end].trim();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        None
    }

    let extract_meta = |property: &str| -> Option<String> {
        let prop_lower = property.to_lowercase();
        for &(tag_lower, tag_orig) in &meta_tags {
            let has_property = get_attr_value(tag_lower, tag_lower, "property")
                .map(|v| v == prop_lower)
                .unwrap_or(false);
            let has_name = get_attr_value(tag_lower, tag_lower, "name")
                .map(|v| v == prop_lower)
                .unwrap_or(false);

            if (has_property || has_name)
                && let Some(content) = get_attr_value(tag_lower, tag_orig, "content")
            {
                return Some(html_decode(content));
            }
        }
        None
    };

    let extract_title = || -> Option<String> {
        let tag = "<title";
        if let Some(pos) = html_lower.find(tag) {
            let after_tag = pos + tag.len();
            if let Some(gt) = html[after_tag..].find('>') {
                let content_start = after_tag + gt + 1;
                if let Some(end) = html_lower[content_start..].find("</title") {
                    let value = html[content_start..content_start + end].trim();
                    if !value.is_empty() {
                        return Some(html_decode(value));
                    }
                }
            }
        }
        None
    };

    let extract_favicon = |base_url: &str| -> Option<String> {
        let mut search_from = 0;
        while let Some(start) = html_lower[search_from..].find("<link") {
            let abs_start = search_from + start;
            let abs_end = html_lower[abs_start..]
                .find('>')
                .map(|p| abs_start + p + 1)
                .unwrap_or(html.len());
            let tag_lower = &html_lower[abs_start..abs_end];
            let tag_orig = &html[abs_start..abs_end];

            let rel = get_attr_value(tag_lower, tag_lower, "rel").unwrap_or("");
            if (rel == "icon" || rel == "shortcut icon")
                && let Some(href) = get_attr_value(tag_lower, tag_orig, "href")
                && !href.is_empty()
            {
                if href.starts_with("http") {
                    return Some(href.to_string());
                }
                return Some(format!(
                    "{}{}",
                    base_url.trim_end_matches('/'),
                    if href.starts_with('/') {
                        href.to_string()
                    } else {
                        format!("/{}", href)
                    }
                ));
            }
            search_from = abs_end;
        }
        None
    };

    let base_url = html_lower
        .find("og:url")
        .and_then(|_| extract_meta("og:url"))
        .unwrap_or_default();

    let base_domain = if let Ok(u) = reqwest::Url::parse(&base_url) {
        format!("{}://{}", u.scheme(), u.host_str().unwrap_or_default())
    } else {
        String::new()
    };

    OgMetadata {
        title: extract_meta("og:title").or_else(extract_title),
        description: extract_meta("og:description").or_else(|| extract_meta("description")),
        image: extract_meta("og:image"),
        site_name: extract_meta("og:site_name"),
        favicon: extract_favicon(&base_domain),
    }
}

fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
}

/// Fetch Open Graph metadata for a given URL
#[utoipa::path(
    get,
    path = "/og",
    tag = "info",
    params(OgQuery),
    responses(
        (status = 200, description = "Open Graph metadata", body = OgMetadata),
        (status = 400, description = "Invalid URL"),
        (status = 502, description = "Failed to fetch the target URL")
    )
)]
#[tracing::instrument(name = "GET /og", skip_all, fields(url))]
pub async fn fetch_og_metadata(
    Query(params): Query<OgQuery>,
) -> Result<Json<OgMetadata>, ApiError> {
    let url = params.url.trim().to_string();
    tracing::Span::current().record("url", url.as_str());

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ApiError::bad_request(
            "URL must start with http:// or https://",
        ));
    }

    let parsed = reqwest::Url::parse(&url).map_err(|_| ApiError::bad_request("Invalid URL"))?;

    if parsed.host_str().is_none() {
        return Err(ApiError::bad_request("URL must have a valid host"));
    }

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (compatible; FlowLikeBot/1.0)")
        .build()
        .map_err(|e| ApiError::internal(format!("HTTP client error: {e}")))?;

    let response = client
        .get(&url)
        .header("Accept", "text/html")
        .send()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("Failed to fetch URL: {e}")))?;

    if !response.status().is_success() {
        return Err(ApiError::bad_gateway(format!(
            "Target returned status {}",
            response.status()
        )));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
        return Ok(Json(OgMetadata {
            title: None,
            description: None,
            image: None,
            site_name: None,
            favicon: None,
        }));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("Failed to read body: {e}")))?;

    let body = if bytes.len() > MAX_BODY_SIZE {
        String::from_utf8_lossy(&bytes[..MAX_BODY_SIZE]).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };

    // Stop at </head> to avoid parsing the full body
    let head_section = if let Some(pos) = body.to_lowercase().find("</head") {
        &body[..pos]
    } else {
        &body
    };

    Ok(Json(parse_og_tags(head_section)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_og_tags() {
        let html = r#"
            <html><head>
                <title>Fallback Title</title>
                <meta property="og:title" content="My Page" />
                <meta property="og:description" content="A cool description" />
                <meta property="og:image" content="https://example.com/img.png" />
                <meta property="og:site_name" content="Example" />
                <meta property="og:url" content="https://example.com" />
                <link rel="icon" href="/favicon.ico" />
            </head></html>
        "#;
        let og = parse_og_tags(html);
        assert_eq!(og.title.as_deref(), Some("My Page"));
        assert_eq!(og.description.as_deref(), Some("A cool description"));
        assert_eq!(og.image.as_deref(), Some("https://example.com/img.png"));
        assert_eq!(og.site_name.as_deref(), Some("Example"));
        assert_eq!(
            og.favicon.as_deref(),
            Some("https://example.com/favicon.ico")
        );
    }

    #[test]
    fn falls_back_to_title_tag() {
        let html = r#"<html><head><title>Only Title</title></head></html>"#;
        let og = parse_og_tags(html);
        assert_eq!(og.title.as_deref(), Some("Only Title"));
        assert!(og.description.is_none());
        assert!(og.image.is_none());
    }

    #[test]
    fn decodes_html_entities() {
        let html = r#"
            <html><head>
                <meta property="og:title" content="Tom &amp; Jerry&#39;s &quot;Show&quot;" />
            </head></html>
        "#;
        let og = parse_og_tags(html);
        assert_eq!(og.title.as_deref(), Some(r#"Tom & Jerry's "Show""#));
    }

    #[test]
    fn handles_single_quoted_attributes() {
        let html = r#"
            <html><head>
                <meta property='og:title' content='Single Quotes' />
            </head></html>
        "#;
        let og = parse_og_tags(html);
        assert_eq!(og.title.as_deref(), Some("Single Quotes"));
    }

    #[test]
    fn handles_name_attribute_for_description() {
        let html = r#"
            <html><head>
                <meta name="description" content="Name-based desc" />
            </head></html>
        "#;
        let og = parse_og_tags(html);
        assert_eq!(og.description.as_deref(), Some("Name-based desc"));
    }

    #[test]
    fn returns_none_for_empty_html() {
        let og = parse_og_tags("");
        assert!(og.title.is_none());
        assert!(og.description.is_none());
        assert!(og.image.is_none());
        assert!(og.site_name.is_none());
        assert!(og.favicon.is_none());
    }

    #[tokio::test]
    async fn extracts_og_from_flow_like_com() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (compatible; FlowLikeBot/1.0)")
            .build()
            .expect("client");

        let resp = client
            .get("https://flow-like.com")
            .header("Accept", "text/html")
            .send()
            .await
            .expect("request failed");

        let body = resp.text().await.expect("body");
        let head = if let Some(pos) = body.to_lowercase().find("</head") {
            &body[..pos]
        } else {
            &body
        };

        let og = parse_og_tags(head);

        assert!(
            og.title.is_some(),
            "flow-like.com should have an og:title or <title>"
        );
        assert!(og.image.is_some(), "flow-like.com should have an og:image");

        let title = og.title.unwrap();
        assert!(
            title.to_lowercase().contains("flow"),
            "title should mention 'flow', got: {title}"
        );

        // description is optional — log it if present
        if let Some(desc) = &og.description {
            assert!(
                !desc.is_empty(),
                "description should not be empty if present"
            );
        }
    }
}
