use serde_json::json;

use crate::error::BrowserError;
use crate::page::Page;
use crate::types::{DialogOpening, DialogType, FrameId};

const INVALID_PARAMS: i64 = -32602;
const NO_DIALOG_SHOWING: &str = "No dialog is showing";

#[derive(Clone, Debug)]
pub struct Dialog {
    pub kind: DialogType,
    pub message: String,
    pub default_prompt: String,
    pub frame_id: Option<FrameId>,
    pub url: String,
    pub seq: u64,
}

impl Dialog {
    pub(crate) fn open_error(&self) -> BrowserError {
        BrowserError::DialogOpen {
            kind: self.kind.clone(),
            message: self.message.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum DialogAction {
    Accept { prompt_text: Option<String> },
    Dismiss,
}

pub(crate) struct DialogState {
    open: Option<Dialog>,
    opened: u64,
}

impl DialogState {
    pub(crate) fn new() -> Self {
        Self {
            open: None,
            opened: 0,
        }
    }

    pub(crate) fn on_event(&mut self, event: &crate::event_log::Event) -> bool {
        match &*event.method {
            "Page.javascriptDialogOpening" => {
                let Some(opening) = event.decode::<DialogOpening>() else {
                    return false;
                };
                self.opened += 1;
                self.open = Some(Dialog {
                    kind: opening.type_,
                    message: opening.message,
                    default_prompt: opening.default_prompt,
                    frame_id: opening.frame_id,
                    url: opening.url,
                    seq: self.opened,
                });
                true
            }
            "Page.javascriptDialogClosed" => self.open.take().is_some(),
            _ => false,
        }
    }

    pub(crate) fn open(&self) -> Option<&Dialog> {
        self.open.as_ref()
    }

    pub(crate) fn clear(&mut self) {
        self.open = None;
    }
}

impl DialogAction {
    fn params(&self) -> serde_json::Value {
        match self {
            Self::Accept {
                prompt_text: Some(text),
            } => json!({"accept": true, "promptText": text}),
            Self::Accept { prompt_text: None } => json!({"accept": true}),
            Self::Dismiss => json!({"accept": false}),
        }
    }
}

impl Page {
    pub async fn handle_dialog(&self, action: DialogAction) -> crate::Result<Dialog> {
        let dialog = self.pending_dialog().ok_or(BrowserError::NoDialog)?;
        let handled = self
            .session()
            .send("Page.handleJavaScriptDialog", action.params())
            .await;
        match handled {
            Ok(_) => {}
            Err(BrowserError::Protocol { code, message, .. })
                if code == INVALID_PARAMS && message.contains(NO_DIALOG_SHOWING) =>
            {
                self.forget_dialog(&dialog);
                return Err(BrowserError::NoDialog);
            }
            Err(error) => return Err(error),
        }
        self.forget_dialog(&dialog);
        if let Err(error) = crate::input::release_pending(self) {
            tracing::debug!(target_id = %self.target_id(), %error, "releasing the input held by the dialog failed");
        }
        Ok(dialog)
    }

    fn forget_dialog(&self, dialog: &Dialog) {
        let cleared = {
            let mut state = self.inner.lock_state();
            let same = state
                .dialog
                .open()
                .is_some_and(|open| open.seq == dialog.seq);
            if same {
                state.dialog.clear();
            }
            same
        };
        if cleared {
            self.inner.bump_version();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn event(method: &str, params: serde_json::Value) -> crate::event_log::Event {
        crate::event_log::Event {
            seq: 1,
            method: method.into(),
            session: None,
            params: Arc::new(params),
            received: tokio::time::Instant::now(),
        }
    }

    #[test]
    fn dialogs_are_numbered_and_cleared() {
        let mut state = DialogState::new();
        let opening = json!({"url": "u", "frameId": "F", "message": "m", "type": "prompt", "defaultPrompt": "d"});
        assert!(state.on_event(&event("Page.javascriptDialogOpening", opening.clone())));
        let dialog = state.open().unwrap();
        assert_eq!((dialog.kind.clone(), dialog.seq), (DialogType::Prompt, 1));
        assert_eq!(dialog.default_prompt, "d");
        assert!(state.on_event(&event(
            "Page.javascriptDialogClosed",
            json!({"result": true})
        )));
        assert!(state.open().is_none());
        assert!(!state.on_event(&event("Page.javascriptDialogClosed", json!({}))));
        state.on_event(&event("Page.javascriptDialogOpening", opening));
        assert_eq!(state.open().map(|dialog| dialog.seq), Some(2));
        state.clear();
        assert!(state.open().is_none());
    }
}
