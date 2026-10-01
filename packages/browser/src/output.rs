use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::error::BrowserError;
use crate::page::Page;
use crate::session::Session;
use crate::settle::OpSpec;
use crate::types::IoReadResult;

const IO_READ_CHUNK: u64 = 1024 * 1024;
const PARSE_ERROR: i64 = -32700;
const PRINT_METHOD: &str = "Page.printToPDF";
/// How long the stream owner keeps waiting for a print reply after the op gave up on it.
const PDF_STREAM_OWNER_GRACE: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug)]
pub struct Clip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScreenshotOptions {
    pub clip: Option<Clip>,
    pub capture_beyond_viewport: bool,
}

struct IoGuard {
    session: Session,
    handle: String,
}

impl IoGuard {
    fn for_print(session: Session, reply: &Value) -> crate::Result<IoGuard> {
        let Some(handle) = reply["stream"].as_str() else {
            return Err(BrowserError::protocol(
                PRINT_METHOD,
                PARSE_ERROR,
                "Page.printToPDF returned no stream handle for transferMode ReturnAsStream",
            ));
        };
        Ok(IoGuard {
            session,
            handle: handle.to_owned(),
        })
    }
}

impl Drop for IoGuard {
    fn drop(&mut self) {
        let params = json!({"handle": self.handle});
        let sent =
            crate::connection::without_op_deadline(|| self.session.send_nowait("IO.close", params));
        if let Err(error) = sent {
            tracing::debug!(handle = %self.handle, %error, "IO.close could not be sent");
        }
    }
}

impl Page {
    pub async fn screenshot(&self, options: ScreenshotOptions) -> crate::Result<Vec<u8>> {
        let main = self.main_frame();
        self.run_op(main.id(), OpSpec::READ, |_| self.capture_in(options))
            .await?
            .read()
    }

    pub async fn print_pdf(&self, params: serde_json::Value) -> crate::Result<Vec<u8>> {
        let params = stream_params(params)?;
        let main = self.main_frame();
        self.run_op(main.id(), OpSpec::OUTPUT, |attempt| {
            self.print_in(params.clone(), attempt.deadline)
        })
        .await?
        .read()
    }

    pub(crate) async fn activate_for_capture(&self) -> crate::Result<()> {
        if self.inner.settings.headless() {
            return Ok(());
        }
        self.inner
            .connection
            .session(None)
            .send(
                "Target.activateTarget",
                json!({"targetId": self.target_id()}),
            )
            .await
            .map(drop)
    }

    async fn capture_in(&self, options: ScreenshotOptions) -> crate::Result<Vec<u8>> {
        self.activate_for_capture().await?;
        let mut params = json!({
            "format": "png",
            "captureBeyondViewport": options.capture_beyond_viewport,
        });
        if let Some(clip) = options.clip {
            params["clip"] = json!({
                "x": clip.x,
                "y": clip.y,
                "width": clip.width,
                "height": clip.height,
                "scale": clip.scale,
            });
        }
        let reply = self
            .session()
            .send("Page.captureScreenshot", params)
            .await?;
        let Some(data) = reply["data"].as_str() else {
            return Err(BrowserError::protocol(
                "Page.captureScreenshot",
                PARSE_ERROR,
                "Page.captureScreenshot returned no image data",
            ));
        };
        decode_base64("Page.captureScreenshot", data)
    }

    async fn print_in(&self, params: Value, deadline: Instant) -> crate::Result<Vec<u8>> {
        let budget = deadline
            .saturating_duration_since(Instant::now())
            .min(self.inner.connection.default_timeout());
        if budget.is_zero() {
            return Err(print_timeout(budget));
        }
        let opened = self.open_pdf_stream(params, budget.saturating_add(PDF_STREAM_OWNER_GRACE))?;
        let stream = tokio::time::timeout(budget, opened)
            .await
            .map_err(|_| print_timeout(budget))?
            .map_err(|_| BrowserError::Disconnected {
                reason: "the task awaiting the Page.printToPDF reply ended without a result".into(),
            })??;
        read_stream(&stream).await
    }

    // The op can end while Page.printToPDF is in flight (its deadline, a dialog race, a cancelled
    // caller), so the reply is awaited on its own task with a longer timeout of its own, and that
    // task closes a stream nobody claims.
    fn open_pdf_stream(
        &self,
        params: Value,
        owner_timeout: Duration,
    ) -> crate::Result<oneshot::Receiver<crate::Result<IoGuard>>> {
        let session = self.session();
        let pending = crate::connection::without_op_deadline(|| {
            session.enqueue(PRINT_METHOD, params, Some(owner_timeout))
        })?;
        let (hand_over, opened) = oneshot::channel();
        tokio::spawn(async move {
            let stream = pending
                .await
                .and_then(|reply| IoGuard::for_print(session, &reply.result));
            if let Err(Ok(unclaimed)) = hand_over.send(stream) {
                tracing::debug!(
                    handle = %unclaimed.handle,
                    "the PDF op ended before Page.printToPDF replied; closing the stream"
                );
            }
        });
        Ok(opened)
    }
}

fn print_timeout(budget: Duration) -> BrowserError {
    BrowserError::Timeout {
        method: PRINT_METHOD.into(),
        timeout_ms: u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
    }
}

fn stream_params(params: Value) -> crate::Result<Value> {
    let mut options = match params {
        Value::Null => serde_json::Map::new(),
        Value::Object(options) => options,
        other => {
            return Err(BrowserError::InvalidArgument {
                message: format!("PDF print options must be a JSON object, got {other}"),
            });
        }
    };
    options.insert("transferMode".into(), json!("ReturnAsStream"));
    Ok(Value::Object(options))
}

// Stream handles are session-scoped: IO.read on another session fails with "Invalid stream handle".
async fn read_stream(stream: &IoGuard) -> crate::Result<Vec<u8>> {
    let mut document = Vec::new();
    loop {
        let params = json!({"handle": stream.handle, "size": IO_READ_CHUNK});
        let reply = stream.session.send("IO.read", params).await?;
        let chunk: IoReadResult = crate::protocol::decode("IO.read", reply)?;
        if chunk.base64_encoded {
            document.extend(decode_base64("IO.read", &chunk.data)?);
        } else {
            document.extend_from_slice(chunk.data.as_bytes());
        }
        if chunk.eof {
            return Ok(document);
        }
    }
}

fn decode_base64(method: &str, data: &str) -> crate::Result<Vec<u8>> {
    BASE64.decode(data).map_err(|error| {
        BrowserError::protocol(
            method,
            PARSE_ERROR,
            format!(
                "{method} returned {} bytes that are not valid base64: {error}",
                data.len()
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};
    use std::time::Duration;

    use super::*;
    use crate::browser::{BrowserSettings, ConnectionKind};
    use crate::page::PageInner;
    use crate::testing::{PageHarness, default_auto_reply};
    use crate::transport::memory::SentCommand;
    use crate::types::{FrameInfo, FrameTreeNode};

    fn capture_reply(command: &SentCommand) -> Option<Value> {
        match command.method.as_str() {
            "Page.captureScreenshot" => Some(json!({"data": BASE64.encode(b"png")})),
            "Target.activateTarget" => Some(json!({})),
            _ => default_auto_reply(command),
        }
    }

    fn headful_page(harness: &PageHarness) -> Page {
        let settings = BrowserSettings::new(ConnectionKind::Direct, false, Duration::from_secs(30));
        let page = Page {
            inner: Arc::new(PageInner::new(
                harness.connection.clone(),
                Weak::new(),
                Arc::new(settings),
                "T1".into(),
                "S1".into(),
                "T1".into(),
            )),
        };
        let main = FrameTreeNode {
            frame: FrameInfo {
                id: "T1".into(),
                loader_id: "L1".into(),
                url: "http://127.0.0.1/".into(),
                ..FrameInfo::default()
            },
            child_frames: Vec::new(),
        };
        let cursor = harness.connection.events().cursor();
        page.inner
            .lock_state()
            .frames
            .seed(&"S1".into(), &main, cursor);
        page
    }

    #[tokio::test]
    async fn headless_captures_skip_the_activation() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(capture_reply);
        harness.page.activate_for_capture().await.unwrap();
        let png = harness
            .page
            .screenshot(ScreenshotOptions::default())
            .await
            .unwrap();
        assert_eq!(png, b"png");
        assert!(
            !harness
                .control
                .commands_seen()
                .iter()
                .any(|command| command.method == "Target.activateTarget")
        );
    }

    #[tokio::test]
    async fn a_headful_screenshot_activates_its_tab_on_the_root_session_first() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(capture_reply);
        let page = headful_page(&harness);
        let png = page.screenshot(ScreenshotOptions::default()).await.unwrap();
        assert_eq!(png, b"png");
        let seen = harness.control.commands_seen();
        let position = |method: &str| {
            seen.iter()
                .position(|command| command.method == method)
                .unwrap_or_else(|| panic!("{method} was never sent"))
        };
        let activation = &seen[position("Target.activateTarget")];
        assert_eq!(activation.session, None);
        assert_eq!(activation.params, json!({"targetId": "T1"}));
        assert!(position("Target.activateTarget") < position("Page.captureScreenshot"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_print_whose_op_deadline_has_passed_sends_nothing() {
        let harness = PageHarness::new().await;
        let params = stream_params(Value::Null).unwrap();
        let error = harness
            .page
            .print_in(params, Instant::now())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, BrowserError::Timeout { method, timeout_ms: 0 } if method == PRINT_METHOD),
            "{error}"
        );
        assert!(
            !harness
                .control
                .commands_seen()
                .iter()
                .any(|command| command.method == PRINT_METHOD)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_stream_owner_outlives_the_op_deadline_by_the_grace() {
        let mut harness = PageHarness::new().await;
        harness
            .control
            .set_auto_reply(|command| match command.method.as_str() {
                PRINT_METHOD | "IO.close" => None,
                _ => default_auto_reply(command),
            });
        let budget = Duration::from_secs(4);
        let params = stream_params(Value::Null).unwrap();
        let error = harness
            .page
            .print_in(params, Instant::now() + budget)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &error,
                BrowserError::Timeout {
                    timeout_ms: 4000,
                    ..
                }
            ),
            "{error}"
        );
        let print = harness.control.wait_for(PRINT_METHOD, Some("S1")).await;
        tokio::time::sleep(PDF_STREAM_OWNER_GRACE - Duration::from_secs(1)).await;
        harness
            .control
            .reply(&print, json!({"stream": "5", "data": ""}));
        let close = harness.control.wait_for("IO.close", Some("S1")).await;
        assert_eq!(close.params, json!({"handle": "5"}));
    }

    #[tokio::test(start_paused = true)]
    async fn a_print_waits_at_most_the_command_timeout_within_a_longer_op() {
        let harness = PageHarness::new().await;
        harness
            .control
            .set_auto_reply(|command| match command.method.as_str() {
                PRINT_METHOD | "IO.close" => None,
                _ => default_auto_reply(command),
            });
        let started = Instant::now();
        let error = harness
            .page
            .print_in(
                stream_params(Value::Null).unwrap(),
                started + Duration::from_secs(300),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(
                &error,
                BrowserError::Timeout {
                    timeout_ms: 30_000,
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(started.elapsed(), Duration::from_secs(30));
    }

    #[test]
    fn print_options_gain_the_stream_transfer_mode() {
        let params = stream_params(json!({"landscape": true, "transferMode": "ReturnAsBase64"}));
        assert_eq!(
            params.unwrap(),
            json!({"landscape": true, "transferMode": "ReturnAsStream"})
        );
        assert_eq!(
            stream_params(Value::Null).unwrap(),
            json!({"transferMode": "ReturnAsStream"})
        );
        assert!(matches!(
            stream_params(json!([1])),
            Err(BrowserError::InvalidArgument { .. })
        ));
    }
}
