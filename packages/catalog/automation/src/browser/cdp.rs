#[cfg(feature = "execute")]
use flow_like_types::Value;

/// Sends one Chrome DevTools Protocol command to the current page. The typed
/// `BrowserError` stays in the error chain (`driver::browser_error` finds it).
#[cfg(feature = "execute")]
pub(crate) async fn send(
    ctx: &super::driver::PageContext,
    method: &str,
    params: Value,
) -> flow_like_types::Result<Value> {
    send_to(&ctx.page, method, params).await
}

#[cfg(feature = "execute")]
pub(super) async fn send_to(
    page: &flow_like_browser::Page,
    method: &str,
    params: Value,
) -> flow_like_types::Result<Value> {
    page.cdp(method, params)
        .await
        .map_err(flow_like_types::Error::from)
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_browser::BrowserError;
    use flow_like_browser::testing::PageHarness;
    use flow_like_types::json::json;

    #[tokio::test]
    async fn a_rejected_command_reports_method_code_and_message() {
        let mut harness = PageHarness::new().await;
        let page = harness.page.clone();
        let sent =
            tokio::spawn(
                async move { send_to(&page, "Animation.getPlaybackRate", json!({})).await },
            );
        let command = harness
            .control
            .wait_for("Animation.getPlaybackRate", Some("S1"))
            .await;
        harness
            .control
            .reply_error(&command, -32602, "Invalid parameters");
        let error = sent
            .await
            .expect("the send task finishes")
            .expect_err("a protocol error reply fails the command");
        assert_eq!(
            error.to_string(),
            "Chrome DevTools command Animation.getPlaybackRate failed (-32602): Invalid parameters"
        );
        assert!(matches!(
            super::super::driver::browser_error(&error),
            Some(BrowserError::Protocol { code: -32602, .. })
        ));
    }
}
