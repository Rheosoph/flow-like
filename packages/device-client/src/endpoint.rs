use std::net::IpAddr;

/// A hub endpoint this client may contact: `secure` anywhere, `plain` only on loopback,
/// and never with credentials, a query or a fragment.
pub(crate) fn checked_url(value: &str, secure: &str, plain: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(value).ok()?;
    let scheme = url.scheme() == secure || (url.scheme() == plain && is_loopback(&url));
    let bare = url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    (scheme && bare).then_some(url)
}

fn is_loopback(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}
