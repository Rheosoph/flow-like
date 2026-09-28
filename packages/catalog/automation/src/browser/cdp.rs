#[cfg(feature = "execute")]
use flow_like_types::Value;

/// Sends one Chrome DevTools Protocol command on the session's current target.
///
/// All CDP traffic from browser nodes goes through here so the transport can be
/// replaced without touching node code. Only Chromium-based browsers accept it.
#[cfg(feature = "execute")]
pub(crate) async fn cdp(
    driver: &thirtyfour::WebDriver,
    method: &str,
    params: Value,
) -> flow_like_types::Result<Value> {
    use thirtyfour::extensions::cdp::ChromeDevTools;

    ChromeDevTools::new(driver.handle.clone())
        .execute_cdp_with_params(method, params)
        .await
        .map_err(|error| {
            flow_like_types::anyhow!(
                "Chrome DevTools command {method} failed (requires Chrome or Edge): {error}"
            )
        })
}
