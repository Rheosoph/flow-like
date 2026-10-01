// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::BrowserError;
use crate::page::Page;
use crate::session::Session;
use crate::settle::OpSpec;
use crate::types::WindowBounds;

const METHOD_NOT_FOUND: i64 = -32601;
const SERVER_ERROR: i64 = -32000;
const NORMAL: &str = "normal";
const POLL_DELAYS_MS: [u64; 5] = [10, 20, 40, 80, 160];
const MEASURE_VIEWPORT: &str = "[window.innerWidth, window.innerHeight]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    width: i64,
    height: i64,
}

impl Size {
    fn grown_by_frame(self, viewport: Size) -> Size {
        Size {
            width: self.width + (self.width - viewport.width).max(0),
            height: self.height + (self.height - viewport.height).max(0),
        }
    }

    fn sizes(self, bounds: &WindowBounds) -> bool {
        bounds.width == Some(self.width) && bounds.height == Some(self.height)
    }

    fn matches(self, bounds: &WindowBounds) -> bool {
        bounds.left == Some(0) && bounds.top == Some(0) && self.sizes(bounds)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WindowForTarget {
    window_id: i64,
    bounds: WindowBounds,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CurrentBounds {
    bounds: WindowBounds,
}

fn state_of(bounds: &WindowBounds) -> &str {
    bounds.window_state.as_deref().unwrap_or(NORMAL)
}

struct Window {
    root: Session,
    id: i64,
    bounds: WindowBounds,
}

impl Window {
    async fn of(page: &Page) -> crate::Result<Window> {
        let root = page.inner.connection.session(None);
        let params = json!({"targetId": page.target_id()});
        let reply = root.send("Browser.getWindowForTarget", params).await?;
        let found: WindowForTarget = crate::protocol::decode("Browser.getWindowForTarget", reply)?;
        Ok(Window {
            root,
            id: found.window_id,
            bounds: found.bounds,
        })
    }

    fn state(&self) -> &str {
        state_of(&self.bounds)
    }

    async fn resize(&mut self, size: Size) -> crate::Result<bool> {
        self.restore_normal_state().await?;
        let bounds = json!({"left": 0, "top": 0, "width": size.width, "height": size.height});
        self.set_bounds(bounds).await?;
        self.poll_until(|bounds| size.matches(bounds)).await?;
        Ok(size.sizes(&self.bounds))
    }

    // The window state cannot be combined with geometry in one Browser.setWindowBounds call.
    async fn restore_normal_state(&mut self) -> crate::Result<()> {
        if self.state() == NORMAL {
            return Ok(());
        }
        self.set_bounds(json!({"windowState": NORMAL})).await?;
        self.bounds = self.current_bounds().await?;
        self.poll_until(|bounds| state_of(bounds) == NORMAL).await?;
        if self.state() == NORMAL {
            return Ok(());
        }
        Err(BrowserError::protocol(
            "Browser.setWindowBounds",
            SERVER_ERROR,
            format!(
                "failed to change window state to '{NORMAL}', current state is '{}'",
                self.state()
            ),
        ))
    }

    async fn set_bounds(&self, bounds: Value) -> crate::Result<()> {
        let params = json!({"windowId": self.id, "bounds": bounds});
        self.root
            .send("Browser.setWindowBounds", params)
            .await
            .map(drop)
    }

    async fn current_bounds(&self) -> crate::Result<WindowBounds> {
        let params = json!({"windowId": self.id});
        let reply = self.root.send("Browser.getWindowBounds", params).await?;
        let current: CurrentBounds = crate::protocol::decode("Browser.getWindowBounds", reply)?;
        Ok(current.bounds)
    }

    // Resizing animates on macOS, so the new bounds show up over several reads.
    async fn poll_until(&mut self, done: impl Fn(&WindowBounds) -> bool) -> crate::Result<()> {
        for delay in POLL_DELAYS_MS {
            if done(&self.bounds) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(delay)).await;
            self.bounds = self.current_bounds().await?;
        }
        Ok(())
    }
}

impl Page {
    pub async fn fit_viewport(&self, width: u32, height: u32) -> crate::Result<()> {
        let requested = Size {
            width: width.into(),
            height: height.into(),
        };
        let main = self.main_frame();
        let fitted = self
            .run_op(main.id(), OpSpec::READ, |_| self.fit_viewport_in(requested))
            .await;
        match fitted {
            Err(BrowserError::Protocol {
                code: METHOD_NOT_FOUND,
                method,
                ..
            }) => {
                tracing::debug!(method, "the browser cannot size its window; viewport kept");
                Ok(())
            }
            fitted => fitted?.read(),
        }
    }

    async fn fit_viewport_in(&self, requested: Size) -> crate::Result<()> {
        let mut window = Window::of(self).await?;
        if !window.resize(requested).await? {
            tracing::debug!(
                ?requested,
                current = ?window.bounds,
                "the window never reached the requested size; not growing by a viewport measured mid-resize"
            );
            return Ok(());
        }
        let Some(viewport) = self.measure_viewport().await? else {
            return Ok(());
        };
        let grown = requested.grown_by_frame(viewport);
        if grown == requested {
            return Ok(());
        }
        window.resize(grown).await.map(drop)
    }

    async fn measure_viewport(&self) -> crate::Result<Option<Size>> {
        let params = json!({"expression": MEASURE_VIEWPORT, "returnByValue": true});
        let reply = self.session().send("Runtime.evaluate", params).await?;
        let value = &reply["result"]["value"];
        match (value[0].as_i64(), value[1].as_i64()) {
            (Some(width), Some(height)) => Ok(Some(Size { width, height })),
            _ => {
                tracing::debug!(%reply, "the viewport size could not be measured; not growing");
                Ok(None)
            }
        }
    }
}
