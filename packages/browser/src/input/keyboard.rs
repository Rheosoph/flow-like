// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::element::{Atom, Element, TEXT_CONTROL_TYPES, mint_elements};
use crate::error::BrowserError;
use crate::input::keys::{KeyEvent, Modifiers, convert_keys};
use crate::input::mouse::{Dispatch, await_or_dialog, committed_error, element_stamp};
use crate::input::{
    ActionOutcome, InputEvent, InputGuard, dialog_outcome, record_sent, unexpected_reply,
};
use crate::page::Page;
use crate::script::{World, array_items};
use crate::session::Session;
use crate::settle::{OpAttempt, OpSpec};

const KEY_TARGET_PROBE: &str = "function(){var d=this.ownerDocument,a=d.activeElement,t=this.tagName.toLowerCase(),s=(d.defaultView||window).getComputedStyle(this),v=s.visibility.toLowerCase();return {tag:t,type:t==='input'?String(this.type).toLowerCase():'',multiple:this.multiple===true,editable:this.isContentEditable===true,active:a===this,focused:d.hasFocus()&&a===this,cssVisible:s.display.toLowerCase()!=='none'&&v!=='hidden'&&v!=='collapse'}}";
const EDITABLE_CARET: &str = "function(isText){var a=document.activeElement,t=this;while(t.parentElement&&t.parentElement.isContentEditable){t=t.parentElement}if((isText&&a!==this)||(!isText&&a!==t)){var r=document.createRange();r.selectNodeContents(this);r.collapse();var s=window.getSelection();s.removeAllRanges();s.addRange(r)}return t}";
const CARET_TO_END: &str =
    "function(){this.setSelectionRange(this.value.length,this.value.length)}";
const SET_VALUE: &str = "function(text){this.value=text}";
const SELECTED_FILES: &str = "function(){return Array.from(this.files||[])}";

static FOCUS: LazyLock<String> = LazyLock::new(|| {
    format!(
        "function(){{{}\nreturn focus(this,false);}}",
        include_str!("../js/focus.js")
    )
});

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct KeyTarget {
    tag: String,
    #[serde(rename = "type")]
    type_: String,
    multiple: bool,
    editable: bool,
    active: bool,
    focused: bool,
    css_visible: bool,
}

impl KeyTarget {
    fn is_input(&self, kind: &str) -> bool {
        self.tag == "input" && self.type_ == kind
    }

    fn is_text(&self) -> bool {
        (self.tag == "input" && TEXT_CONTROL_TYPES.contains(&self.type_.as_str()))
            || self.tag == "textarea"
    }
}

async fn file_paths(text: &str, multiple: bool) -> crate::Result<Vec<String>> {
    if text.is_empty() {
        return Err(BrowserError::InvalidArgument {
            message: "no file path was given for the file input".to_owned(),
        });
    }
    let mut files = Vec::new();
    for path in text.split('\n').map(str::trim) {
        let canonical =
            tokio::fs::canonicalize(path)
                .await
                .map_err(|_| BrowserError::InvalidArgument {
                    message: format!("File not found: {path}"),
                })?;
        files.push(without_verbatim_prefix(&canonical.to_string_lossy()));
    }
    if files.len() > 1 && !multiple {
        return Err(BrowserError::InvalidArgument {
            message: "the element can not hold multiple files".to_owned(),
        });
    }
    Ok(files)
}

fn without_verbatim_prefix(path: &str) -> String {
    match path.strip_prefix("\\\\?\\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => path.to_owned(),
    }
}

pub(crate) async fn dispatch_key_events(
    page: &Page,
    attempt: &OpAttempt,
    mut events: Vec<KeyEvent>,
) -> crate::Result<ActionOutcome> {
    let Some(last) = events.pop() else {
        return Ok(ActionOutcome::Completed);
    };
    let session = page.session();
    let page_session = page.inner.session.clone();
    let dialogs = page.dialog_count();
    if let Some(dialog) = page.pending_dialog() {
        return Ok(dialog_outcome(dialog));
    }
    for event in events {
        let event = InputEvent::Key(event);
        let ticket = match session.send_nowait(event.method(), event.params()) {
            Ok(ticket) => ticket,
            Err(error) => return finished(committed_error(attempt, error)?),
        };
        attempt.commit();
        record_sent(page, &page_session, &event, &ticket, attempt.deadline);
    }
    let event = InputEvent::Key(last);
    let pending = match session.enqueue(event.method(), event.params(), None) {
        Ok(pending) => pending,
        Err(error) => return finished(committed_error(attempt, error)?),
    };
    attempt.commit();
    record_sent(
        page,
        &page_session,
        &event,
        pending.ticket(),
        attempt.deadline,
    );
    finished(await_or_dialog(page, attempt, dialogs, pending).await?)
}

fn finished(dispatch: Dispatch) -> crate::Result<ActionOutcome> {
    Ok(match dispatch {
        Dispatch::Dialog(dialog) => dialog_outcome(dialog),
        Dispatch::Acknowledged | Dispatch::TargetGone => ActionOutcome::Completed,
    })
}

impl Element {
    pub async fn send_keys(&self, text: &str) -> crate::Result<ActionOutcome> {
        let mut sticky = Modifiers::NONE;
        let events = convert_keys(text, &mut sticky, true)?;
        let _guard = InputGuard::new(&self.frame.page);
        let outcome = self
            .run(OpSpec::INPUT, |attempt| {
                let events = events.clone();
                async move { self.send_keys_in(&attempt, text, events).await }
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    async fn send_keys_in(
        &self,
        attempt: &OpAttempt,
        text: &str,
        events: Vec<KeyEvent>,
    ) -> crate::Result<ActionOutcome> {
        let target = self.key_target_in(attempt).await?;
        if target.is_input("file") {
            return self.upload_in(attempt, text, target.multiple).await;
        }
        if target.is_input("color") {
            let value = vec![json!(text)];
            self.call_function_in(attempt, World::Util, SET_VALUE, value, true)
                .await?;
            return Ok(ActionOutcome::Completed);
        }
        self.prepare_typing_in(attempt, target).await?;
        dispatch_key_events(&self.frame.page, attempt, events).await
    }

    async fn prepare_typing_in(&self, attempt: &OpAttempt, target: KeyTarget) -> crate::Result<()> {
        let is_text = target.is_text();
        let (focus_target, state) = self.focus_target_in(attempt, target, is_text).await?;
        if !is_text || !state.focused {
            focus_target.focus_for_keys_in(attempt, &state).await?;
        }
        if is_text && !state.focused {
            self.call_function_in(attempt, World::Util, CARET_TO_END, Vec::new(), true)
                .await?;
        }
        Ok(())
    }

    async fn focus_target_in(
        &self,
        attempt: &OpAttempt,
        target: KeyTarget,
        is_text: bool,
    ) -> crate::Result<(Element, KeyTarget)> {
        if !target.editable {
            return Ok((self.clone(), target));
        }
        let top = self.editable_target_in(attempt, is_text).await?;
        if top.backend_node_id == self.backend_node_id {
            return Ok((top, target));
        }
        let state = top.key_target_in(attempt).await?;
        Ok((top, state))
    }

    async fn key_target_in(&self, attempt: &OpAttempt) -> crate::Result<KeyTarget> {
        let probed = self
            .call_function_in(attempt, World::Util, KEY_TARGET_PROBE, Vec::new(), true)
            .await?;
        KeyTarget::deserialize(&probed).map_err(|error| {
            BrowserError::protocol(
                "Runtime.callFunctionOn",
                -32000,
                format!("unexpected key target probe result {probed}: {error}"),
            )
        })
    }

    async fn editable_target_in(
        &self,
        attempt: &OpAttempt,
        is_text: bool,
    ) -> crate::Result<Element> {
        let before = self.frame.stamp()?;
        let is_text_arg = vec![json!(is_text)];
        let top = self
            .call_function_in(attempt, World::Util, EDITABLE_CARET, is_text_arg, false)
            .await?;
        if is_text {
            return Ok(self.clone());
        }
        let backend_node_id = self.backend_node_of(&top).await?;
        if backend_node_id == self.backend_node_id {
            return Ok(self.clone());
        }
        let method = "DOM.describeNode";
        let minted = mint_elements(&self.frame, &before, vec![backend_node_id], method)?;
        minted
            .into_iter()
            .next()
            .ok_or_else(|| unexpected_reply(method, "node", &top))
    }

    async fn backend_node_of(&self, object: &Value) -> crate::Result<i64> {
        let method = "DOM.describeNode";
        let Some(object_id) = object["objectId"].as_str() else {
            return Err(unexpected_reply(
                "Runtime.callFunctionOn",
                "result.objectId",
                object,
            ));
        };
        let session = self.frame.session()?;
        let described = session.send(method, json!({"objectId": object_id})).await?;
        described["node"]["backendNodeId"]
            .as_i64()
            .ok_or_else(|| unexpected_reply(method, "node.backendNodeId", &described))
    }

    async fn focus_for_keys_in(&self, attempt: &OpAttempt, state: &KeyTarget) -> crate::Result<()> {
        let focused = self.shown_and_focused_in(attempt, state).await?;
        let enabled = self
            .call_atom_in(attempt, Atom::IsEnabled, Vec::new())
            .await?;
        if enabled != Value::Bool(true) {
            return Err(BrowserError::NotInteractable {
                message: "the element is disabled".to_owned(),
            });
        }
        if focused {
            return Ok(());
        }
        let focus = self.call_function_in(attempt, World::Util, &FOCUS, Vec::new(), true);
        match focus.await {
            Err(BrowserError::Javascript { message }) => Err(unfocusable(&message)),
            other => other.map(drop),
        }
    }

    async fn shown_and_focused_in(
        &self,
        attempt: &OpAttempt,
        state: &KeyTarget,
    ) -> crate::Result<bool> {
        let displayed = self
            .call_atom_in(attempt, Atom::IsDisplayed, vec![json!(true)])
            .await?;
        if displayed == Value::Bool(true) {
            return Ok(state.active);
        }
        if state.focused || state.css_visible {
            return Ok(state.focused);
        }
        Err(BrowserError::NotInteractable {
            message: "the element is not displayed".to_owned(),
        })
    }

    async fn upload_in(
        &self,
        attempt: &OpAttempt,
        text: &str,
        multiple: bool,
    ) -> crate::Result<ActionOutcome> {
        let added = file_paths(text, multiple).await?;
        let page = &self.frame.page;
        let owner = element_stamp(self)?.session;
        let session = page.inner.connection.session(Some(owner));
        let mut files = self.kept_files_in(attempt, &session, multiple).await?;
        files.extend(added);
        let dialogs = page.dialog_count();
        if let Some(dialog) = page.pending_dialog() {
            return Ok(dialog_outcome(dialog));
        }
        let params = json!({"files": files, "backendNodeId": self.backend_node_id});
        let pending = match session.enqueue("DOM.setFileInputFiles", params, None) {
            Ok(pending) => pending,
            Err(error) => return finished(committed_error(attempt, error)?),
        };
        attempt.commit();
        self.uploaded(await_or_dialog(page, attempt, dialogs, pending).await?)
    }

    /// Paths of the files a `multiple` input already holds, which chromedriver keeps
    /// ahead of the new ones: each `File` is read back with `DOM.getFileInfo`.
    async fn kept_files_in(
        &self,
        attempt: &OpAttempt,
        session: &Session,
        multiple: bool,
    ) -> crate::Result<Vec<String>> {
        if !multiple {
            return Ok(Vec::new());
        }
        let list = self
            .call_function_in(attempt, World::Util, SELECTED_FILES, Vec::new(), false)
            .await?;
        match list["objectId"].as_str() {
            Some(array) => file_info_paths(session, array).await,
            None => Ok(Vec::new()),
        }
    }

    fn uploaded(&self, dispatched: Dispatch) -> crate::Result<ActionOutcome> {
        if matches!(dispatched, Dispatch::Acknowledged) {
            self.ensure_document("DOM.setFileInputFiles")?;
        }
        finished(dispatched)
    }
}

async fn file_info_paths(session: &Session, array: &str) -> crate::Result<Vec<String>> {
    let params = json!({"objectId": array, "ownProperties": true});
    let properties = session.send("Runtime.getProperties", params).await?;
    let pending = array_items(&properties)
        .into_iter()
        .map(|file| session.enqueue("DOM.getFileInfo", json!({"objectId": file}), None))
        .collect::<crate::Result<Vec<_>>>()?;
    let mut paths = Vec::with_capacity(pending.len());
    for reply in pending {
        let info = reply.await?.result;
        let path = info["path"]
            .as_str()
            .ok_or_else(|| unexpected_reply("DOM.getFileInfo", "path", &info))?;
        paths.push(path.to_owned());
    }
    Ok(paths)
}

fn unfocusable(message: &str) -> BrowserError {
    BrowserError::NotInteractable {
        message: format!(
            "the element cannot be focused ({})",
            message.lines().next().unwrap_or_default()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::scene::{
        self, TARGET, calls_atom, declaration, element, input_events, value,
    };
    use crate::testing::PageHarness;
    use crate::transport::memory::SentCommand;

    #[derive(Clone, Copy)]
    struct Field {
        tag: &'static str,
        kind: &'static str,
        focused: bool,
        displayed: bool,
        enabled: bool,
        css_visible: bool,
        editable: bool,
        multiple: bool,
        selected: &'static [&'static str],
    }

    const TEXT: Field = Field {
        tag: "input",
        kind: "text",
        focused: false,
        displayed: true,
        enabled: true,
        css_visible: true,
        editable: false,
        multiple: false,
        selected: &[],
    };

    fn selected_reply(field: Field, command: &SentCommand) -> Option<Value> {
        let object = command.params["objectId"].as_str().unwrap_or_default();
        match command.method.as_str() {
            "Runtime.getProperties" if object == "selected" => {
                let mut items: Vec<Value> = (0..field.selected.len())
                    .rev()
                    .map(|index| {
                        json!({"name": index.to_string(),
                            "value": {"type": "object", "objectId": format!("file-{index}")}})
                    })
                    .collect();
                let length = field.selected.len();
                items.push(json!({"name": "length", "value": {"type": "number", "value": length}}));
                Some(json!({"result": items}))
            }
            "DOM.getFileInfo" => {
                let index: usize = object.strip_prefix("file-")?.parse().ok()?;
                Some(json!({"path": field.selected.get(index)?}))
            }
            _ if declaration(command) == SELECTED_FILES => Some(json!({"result": {
                "type": "object", "subtype": "array", "objectId": "selected"}})),
            _ => None,
        }
    }

    fn field_reply(field: Field) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync {
        move |command| {
            if let Some(reply) = selected_reply(field, command) {
                return Some(reply);
            }
            let body = declaration(command);
            if body == KEY_TARGET_PROBE {
                return value(json!({
                    "tag": field.tag, "type": field.kind, "multiple": field.multiple,
                    "editable": field.editable, "active": field.focused, "focused": field.focused,
                    "cssVisible": field.css_visible,
                }));
            }
            if calls_atom(command, "isDisplayed") {
                return value(json!(field.displayed));
            }
            if calls_atom(command, "isEnabled") {
                return value(json!(field.enabled));
            }
            if body == FOCUS.as_str() || body == CARET_TO_END || body == SET_VALUE {
                return value(Value::Null);
            }
            scene::reply(command)
        }
    }

    fn scripts(harness: &PageHarness) -> Vec<String> {
        harness
            .control
            .commands_seen()
            .iter()
            .filter_map(|command| {
                let body = declaration(command);
                [
                    (KEY_TARGET_PROBE, "probe"),
                    (FOCUS.as_str(), "focus"),
                    (CARET_TO_END, "caret"),
                    (EDITABLE_CARET, "editable"),
                    (SET_VALUE, "value"),
                    (SELECTED_FILES, "selected"),
                ]
                .into_iter()
                .find(|(script, _)| body == *script)
                .map(|(_, name)| name)
                .or_else(|| calls_atom(command, "isDisplayed").then_some("displayed"))
                .or_else(|| calls_atom(command, "isEnabled").then_some("enabled"))
                .or_else(|| (command.method == "Input.dispatchKeyEvent").then_some("key"))
                .map(str::to_owned)
            })
            .collect()
    }

    fn typed(harness: &PageHarness) -> Vec<(String, String, String)> {
        input_events(&harness.control)
            .iter()
            .map(|command| {
                (
                    command.session.clone().unwrap_or_default(),
                    command.params["type"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    command.params["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect()
    }

    async fn type_into(
        field: Field,
        frame: &str,
        text: &str,
    ) -> (PageHarness, crate::Result<ActionOutcome>) {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(field_reply(field));
        let target = element(&harness, frame, TARGET);
        let outcome = target.send_keys(text).await;
        (harness, outcome)
    }

    #[tokio::test]
    async fn key_events_are_pipelined_on_the_page_session_and_only_the_last_is_awaited() {
        let harness = scene::page_with_oopif().await;
        let reply = field_reply(TEXT);
        harness.control.set_auto_reply(move |command| {
            (command.method != "Input.dispatchKeyEvent")
                .then(|| reply(command))
                .flatten()
        });
        let mut control = harness.control;
        let frame = harness
            .page
            .frame(&crate::types::FrameId::from("F2"))
            .unwrap();
        let stamp = frame.stamp().unwrap();
        let target = Element::new(frame, &stamp, TARGET, None);
        let typing = tokio::spawn(async move { target.send_keys("ab").await });
        let mut sent = Vec::new();
        for _ in 0..6 {
            let command = control.next_command().await;
            assert_eq!(command.method, "Input.dispatchKeyEvent");
            assert_eq!(command.session.as_deref(), Some("S1"));
            sent.push(command);
        }
        assert!(!typing.is_finished(), "the last key event is still awaited");
        let kinds: Vec<_> = sent
            .iter()
            .map(|command| {
                command.params["type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            kinds,
            ["rawKeyDown", "char", "keyUp", "rawKeyDown", "char", "keyUp"]
        );
        control.reply(&sent[5], json!({}));
        assert_eq!(typing.await.unwrap().unwrap(), ActionOutcome::Completed);
    }

    #[tokio::test]
    async fn an_unfocused_text_field_is_focused_and_its_caret_moved_before_typing() {
        let (harness, outcome) = type_into(TEXT, "F2", "hi").await;
        assert_eq!(outcome.unwrap(), ActionOutcome::Completed);
        assert_eq!(
            scripts(&harness),
            [
                "probe",
                "displayed",
                "enabled",
                "focus",
                "caret",
                "key",
                "key",
                "key",
                "key",
                "key",
                "key"
            ]
        );
        let texts: Vec<_> = typed(&harness)
            .into_iter()
            .filter(|(_, kind, _)| kind == "char")
            .map(|(session, _, text)| (session, text))
            .collect();
        assert_eq!(
            texts,
            [
                ("S1".to_owned(), "h".to_owned()),
                ("S1".to_owned(), "i".to_owned())
            ]
        );
    }

    #[tokio::test]
    async fn a_focused_text_field_keeps_its_caret() {
        let field = Field {
            focused: true,
            ..TEXT
        };
        let (harness, outcome) = type_into(field, "T1", "x").await;
        outcome.unwrap();
        assert_eq!(scripts(&harness), ["probe", "key", "key", "key"]);
    }

    #[tokio::test]
    async fn non_text_elements_are_focused_even_when_active_but_keep_their_caret() {
        let field = Field {
            tag: "button",
            kind: "",
            focused: true,
            ..TEXT
        };
        let (harness, outcome) = type_into(field, "T1", "x").await;
        outcome.unwrap();
        assert_eq!(
            scripts(&harness),
            ["probe", "displayed", "enabled", "key", "key", "key"]
        );
    }

    #[tokio::test]
    async fn hidden_disabled_and_unfocusable_elements_are_rejected() {
        let hidden = Field {
            displayed: false,
            css_visible: false,
            ..TEXT
        };
        let (harness, outcome) = type_into(hidden, "T1", "x").await;
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "element not interactable: the element is not displayed"
        );
        assert!(input_events(&harness.control).is_empty());
        let disabled = Field {
            enabled: false,
            ..TEXT
        };
        let (_, outcome) = type_into(disabled, "T1", "x").await;
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "element not interactable: the element is disabled"
        );
        let harness = scene::page_with_oopif().await;
        let reply = field_reply(TEXT);
        harness.control.set_auto_reply(move |command| {
            if declaration(command) == FOCUS.as_str() {
                return Some(json!({
                    "result": {"type": "object"},
                    "exceptionDetails": {"text": "Uncaught", "exception": {"type": "object",
                        "description": "Error: cannot focus element\n    at focus (<anonymous>:1:1)"}},
                }));
            }
            reply(command)
        });
        let error = element(&harness, "T1", TARGET)
            .send_keys("x")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "element not interactable: the element cannot be focused (Error: cannot focus element)"
        );
    }

    #[tokio::test]
    async fn off_screen_fields_that_css_shows_are_still_typed_into() {
        let off_screen = Field {
            displayed: false,
            ..TEXT
        };
        let (harness, outcome) = type_into(off_screen, "T1", "q").await;
        outcome.unwrap();
        assert_eq!(
            scripts(&harness),
            [
                "probe",
                "displayed",
                "enabled",
                "focus",
                "caret",
                "key",
                "key",
                "key"
            ]
        );
    }

    #[tokio::test]
    async fn color_inputs_get_their_value_without_key_events() {
        let color = Field {
            kind: "color",
            ..TEXT
        };
        let (harness, outcome) = type_into(color, "T1", "#ff0000").await;
        outcome.unwrap();
        assert_eq!(scripts(&harness), ["probe", "value"]);
        let set = harness
            .control
            .commands_seen()
            .into_iter()
            .find(|command| declaration(command) == SET_VALUE)
            .unwrap();
        assert_eq!(set.params["arguments"], json!([{"value": "#ff0000"}]));
    }

    fn uploads(harness: &PageHarness) -> Vec<SentCommand> {
        harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method == "DOM.setFileInputFiles")
            .collect()
    }

    #[tokio::test]
    async fn file_inputs_receive_canonical_existing_paths() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("a.txt");
        let second = directory.path().join("b.txt");
        std::fs::write(&first, "a").unwrap();
        std::fs::write(&second, "b").unwrap();
        let text = format!("{}\n {} ", first.display(), second.display());
        let file = Field {
            kind: "file",
            multiple: true,
            ..TEXT
        };
        let (harness, outcome) = type_into(file, "F2", &text).await;
        assert_eq!(outcome.unwrap(), ActionOutcome::Completed);
        let upload = uploads(&harness);
        assert_eq!(upload.len(), 1);
        assert_eq!(upload[0].session.as_deref(), Some("S2"));
        assert_eq!(upload[0].params["backendNodeId"], TARGET);
        let expected: Vec<String> = [&first, &second]
            .iter()
            .map(|path| {
                without_verbatim_prefix(&std::fs::canonicalize(path).unwrap().to_string_lossy())
            })
            .collect();
        assert_eq!(upload[0].params["files"], json!(expected));
        assert!(expected.iter().all(|path| !path.starts_with("\\\\?\\")));
        assert!(input_events(&harness.control).is_empty());

        let single = Field {
            multiple: false,
            ..file
        };
        let (harness, outcome) = type_into(single, "F2", &text).await;
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "invalid argument: the element can not hold multiple files"
        );
        assert!(uploads(&harness).is_empty());

        let missing = directory.path().join("missing.txt");
        let (harness, outcome) = type_into(file, "F2", &missing.display().to_string()).await;
        assert_eq!(
            outcome.unwrap_err().to_string(),
            format!("invalid argument: File not found: {}", missing.display())
        );
        assert!(uploads(&harness).is_empty());
    }

    #[tokio::test]
    async fn multiple_file_inputs_keep_the_files_already_selected() {
        let directory = tempfile::tempdir().unwrap();
        let added = directory.path().join("c.txt");
        std::fs::write(&added, "c").unwrap();
        let canonical = std::fs::canonicalize(&added).unwrap();
        let added_path = without_verbatim_prefix(&canonical.to_string_lossy());
        let text = added.display().to_string();
        let file = Field {
            kind: "file",
            multiple: true,
            selected: &["/picked/a.txt", "/picked/b.txt"],
            ..TEXT
        };
        let (harness, outcome) = type_into(file, "F2", &text).await;
        assert_eq!(outcome.unwrap(), ActionOutcome::Completed);
        let upload = uploads(&harness);
        assert_eq!(upload.len(), 1);
        assert_eq!(
            upload[0].params["files"],
            json!(["/picked/a.txt", "/picked/b.txt", added_path])
        );
        let reads: Vec<SentCommand> = harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| {
                ["Runtime.getProperties", "DOM.getFileInfo"].contains(&command.method.as_str())
            })
            .collect();
        assert_eq!(reads.len(), 3);
        assert!(
            reads
                .iter()
                .all(|command| command.session.as_deref() == Some("S2"))
        );

        let single = Field {
            multiple: false,
            ..file
        };
        let (harness, outcome) = type_into(single, "F2", &text).await;
        assert_eq!(outcome.unwrap(), ActionOutcome::Completed);
        assert_eq!(uploads(&harness)[0].params["files"], json!([added_path]));
        assert!(!scripts(&harness).iter().any(|script| script == "selected"));
    }

    async fn start_upload(
        harness: &mut PageHarness,
        path: String,
    ) -> (
        tokio::task::JoinHandle<crate::Result<ActionOutcome>>,
        SentCommand,
    ) {
        let reply = field_reply(Field {
            kind: "file",
            ..TEXT
        });
        harness.control.set_auto_reply(move |command| {
            (command.method != "DOM.setFileInputFiles")
                .then(|| reply(command))
                .flatten()
        });
        let input = element(harness, "F2", TARGET);
        let upload = tokio::spawn(async move { input.send_keys(&path).await });
        let set = harness
            .control
            .wait_for("DOM.setFileInputFiles", Some("S2"))
            .await;
        (upload, set)
    }

    fn existing_file(directory: &tempfile::TempDir) -> String {
        let path = directory.path().join("upload.txt");
        std::fs::write(&path, "upload").unwrap();
        path.display().to_string()
    }

    #[tokio::test]
    async fn a_dialog_opened_by_the_upload_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let mut harness = scene::page_with_oopif().await;
        let (upload, _) = start_upload(&mut harness, existing_file(&directory)).await;
        harness.emit(
            "Page.javascriptDialogOpening",
            json!({"url": "http://127.0.0.1/", "message": "changed", "type": "alert", "defaultPrompt": ""}),
        );
        assert_eq!(
            upload.await.unwrap().unwrap(),
            ActionOutcome::DialogOpened {
                kind: crate::types::DialogType::Alert,
                message: "changed".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn a_target_closed_after_the_upload_is_completed() {
        let directory = tempfile::tempdir().unwrap();
        let mut harness = scene::page_with_oopif().await;
        let (upload, set) = start_upload(&mut harness, existing_file(&directory)).await;
        harness
            .control
            .reply_error(&set, -32001, "Session with given id not found.");
        assert_eq!(upload.await.unwrap().unwrap(), ActionOutcome::Completed);
    }

    #[tokio::test]
    async fn an_upload_answered_after_a_new_document_is_not_reported_as_done() {
        let directory = tempfile::tempdir().unwrap();
        let mut harness = scene::page_with_oopif().await;
        let (upload, set) = start_upload(&mut harness, existing_file(&directory)).await;
        harness.emit_on(
            "S2",
            "Page.frameNavigated",
            json!({"frame": {"id": "F2", "parentId": "T1", "loaderId": "L9", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        harness
            .page
            .wait_for_state(deadline, |state| {
                (state.frames.committed_loader(&"F2".into()) == Some("L9".into())).then_some(())
            })
            .await
            .unwrap();
        harness.control.reply(&set, json!({}));
        let error = upload.await.unwrap().unwrap_err();
        assert!(
            matches!(&error, BrowserError::LoaderChanged { method } if method == "DOM.setFileInputFiles"),
            "{error}"
        );
    }

    #[test]
    fn windows_verbatim_drive_paths_lose_their_prefix() {
        assert_eq!(
            without_verbatim_prefix("\\\\?\\C:\\files\\a.txt"),
            "C:\\files\\a.txt"
        );
        assert_eq!(
            without_verbatim_prefix("\\\\?\\UNC\\server\\share\\a.txt"),
            "\\\\?\\UNC\\server\\share\\a.txt"
        );
        assert_eq!(without_verbatim_prefix("/tmp/a.txt"), "/tmp/a.txt");
    }

    fn editable_reply(command: &SentCommand) -> Option<Value> {
        let body = declaration(command);
        let on_top = command.params["objectId"] == "obj-40";
        if body == KEY_TARGET_PROBE {
            let tag = if on_top { "div" } else { "span" };
            return value(
                json!({"tag": tag, "type": "", "editable": true, "active": false,
                "focused": false, "cssVisible": true}),
            );
        }
        if body == EDITABLE_CARET {
            return Some(
                json!({"result": {"type": "object", "subtype": "node", "objectId": "top-1"}}),
            );
        }
        if command.method == "DOM.describeNode" && command.params["objectId"] == "top-1" {
            return Some(json!({"node": {"backendNodeId": 40, "nodeName": "DIV"}}));
        }
        field_reply(TEXT)(command)
    }

    #[tokio::test]
    async fn contenteditable_children_focus_their_top_editable_ancestor() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(editable_reply);
        let span = element(&harness, "T1", TARGET);
        assert_eq!(span.send_keys("z").await.unwrap(), ActionOutcome::Completed);
        let seen = harness.control.commands_seen();
        let caret = seen
            .iter()
            .find(|command| declaration(command) == EDITABLE_CARET)
            .unwrap();
        assert_eq!(caret.params["objectId"], format!("obj-{TARGET}"));
        assert_eq!(caret.params["arguments"], json!([{"value": false}]));
        let focus = seen
            .iter()
            .find(|command| declaration(command) == FOCUS.as_str())
            .unwrap();
        assert_eq!(focus.params["objectId"], "obj-40");
        assert_eq!(
            scripts(&harness),
            [
                "probe",
                "editable",
                "probe",
                "displayed",
                "enabled",
                "focus",
                "key",
                "key",
                "key"
            ]
        );
    }

    #[tokio::test]
    async fn a_dialog_on_the_last_key_event_is_reported() {
        let harness = scene::page_with_oopif().await;
        let reply = field_reply(Field {
            focused: true,
            ..TEXT
        });
        harness.control.set_auto_reply(move |command| {
            (command.method != "Input.dispatchKeyEvent")
                .then(|| reply(command))
                .flatten()
        });
        let mut control = harness.control;
        let frame = harness.page.main_frame();
        let stamp = frame.stamp().unwrap();
        let target = Element::new(frame, &stamp, TARGET, None);
        let typing = tokio::spawn(async move { target.send_keys("\n").await });
        for _ in 0..3 {
            control.next_command().await;
        }
        control.emit(
            "Page.javascriptDialogOpening",
            Some("S1"),
            json!({"url": "http://127.0.0.1/", "message": "sent", "type": "alert", "defaultPrompt": ""}),
        );
        assert_eq!(
            typing.await.unwrap().unwrap(),
            ActionOutcome::DialogOpened {
                kind: crate::types::DialogType::Alert,
                message: "sent".to_owned()
            }
        );
        assert_eq!(
            harness.page.inner.lock_state().input.modifiers,
            Modifiers::NONE
        );
    }
}
