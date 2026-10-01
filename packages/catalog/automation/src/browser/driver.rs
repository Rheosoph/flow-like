//! Node-facing helpers over the Chrome DevTools engine (`flow-like-browser`).
use flow_like_browser::input::MouseButton;
use flow_like_browser::input::keys::{Key, Modifiers, NamedKey};
use flow_like_browser::{BrowserError, ErrorClass, Frame};

#[cfg(feature = "execute")]
use crate::types::handles::{BrowserSlot, locked};
#[cfg(feature = "execute")]
use flow_like_browser::script::{ScriptArg, ScriptOptions, ScriptValue};
#[cfg(feature = "execute")]
use flow_like_browser::{Browser, Element, Page};
#[cfg(feature = "execute")]
use std::sync::Arc;

/// The page and frame one node operates on. The frame path is replayed from the main
/// frame for every operation, so a re-rendered iframe never sends a node to a stale document.
#[cfg(feature = "execute")]
pub struct PageContext {
    pub browser: Browser,
    pub page: Page,
    frame: Frame,
    pub(crate) slot: Arc<BrowserSlot>,
    pub(crate) selectors: Vec<crate::types::selectors::Selector>,
}

#[cfg(feature = "execute")]
impl PageContext {
    pub(crate) fn new(
        browser: Browser,
        page: Page,
        slot: Arc<BrowserSlot>,
        selectors: Vec<crate::types::selectors::Selector>,
    ) -> Self {
        let frame = page.main_frame();
        Self {
            browser,
            page,
            frame,
            slot,
            selectors,
        }
    }

    /// Enters every frame of `selectors`, starting at the main frame.
    pub(crate) async fn enter_frame_path(mut self) -> flow_like_types::Result<Self> {
        for selector in self.selectors.clone() {
            let owner = crate::browser::selector::find_element(&self, &selector).await?;
            self.frame = owner.frame().child_frame(&owner).await?;
        }
        Ok(self)
    }

    pub fn frame(&self) -> Frame {
        self.frame.clone()
    }

    /// Runs a user script (never retried after a navigation).
    pub async fn execute(
        &self,
        body: &str,
        args: Vec<ScriptArg>,
    ) -> flow_like_types::Result<ScriptValue> {
        self.run_script(body, args, ScriptOptions::USER).await
    }

    /// Runs an internal read-only script, which may be retried after a navigation interrupts it.
    pub async fn probe(
        &self,
        body: &str,
        args: Vec<ScriptArg>,
    ) -> flow_like_types::Result<ScriptValue> {
        self.run_script(body, args, ScriptOptions::PROBE).await
    }

    /// An element argument pins the script to the frame of that element; without one the
    /// script runs in the replayed current frame.
    pub(crate) fn script_frame(&self, args: &[ScriptArg]) -> flow_like_types::Result<Frame> {
        common_frame(
            &self.frame,
            args.iter().filter_map(|arg| match arg {
                ScriptArg::Element(element) => Some(element.frame().clone()),
                ScriptArg::Json(_) => None,
            }),
        )
    }

    async fn run_script(
        &self,
        body: &str,
        args: Vec<ScriptArg>,
        options: ScriptOptions,
    ) -> flow_like_types::Result<ScriptValue> {
        let frame = self.script_frame(&args)?;
        Ok(frame.execute_script(body, args, options).await?)
    }
}

#[cfg(any(feature = "execute", test))]
fn common_frame(
    current: &Frame,
    frames: impl IntoIterator<Item = Frame>,
) -> flow_like_types::Result<Frame> {
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Ok(current.clone());
    };
    if frames.all(|frame| same_frame(&frame, &first)) {
        return Ok(first);
    }
    Err(BrowserError::InvalidArgument {
        message: "Script arguments come from different frames".to_owned(),
    }
    .into())
}

#[cfg(any(feature = "execute", test))]
fn same_frame(left: &Frame, right: &Frame) -> bool {
    left.id() == right.id() && left.page().target_id() == right.page().target_id()
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn parse_modifiers(values: &[String]) -> flow_like_types::Result<Modifiers> {
    values
        .iter()
        .try_fold(Modifiers::NONE, |mut modifiers, value| {
            modifiers.insert(match value.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Modifiers::CTRL,
                "shift" => Modifiers::SHIFT,
                "alt" | "option" => Modifiers::ALT,
                "meta" | "cmd" | "command" | "win" => Modifiers::META,
                _ => {
                    return Err(flow_like_types::anyhow!(
                        "Unknown browser modifier: {value}"
                    ));
                }
            });
            Ok(modifiers)
        })
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn parse_button(name: &str) -> flow_like_types::Result<MouseButton> {
    match name {
        "left" => Ok(MouseButton::Left),
        "middle" => Ok(MouseButton::Middle),
        "right" => Ok(MouseButton::Right),
        _ => Err(flow_like_types::anyhow!(
            "Supported click buttons are left, middle, and right"
        )),
    }
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn browser_key(name: &str) -> flow_like_types::Result<Key> {
    let named = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "escape" | "esc" => NamedKey::Escape,
        "backspace" => NamedKey::Backspace,
        "delete" => NamedKey::Delete,
        "arrowup" | "up" => NamedKey::ArrowUp,
        "arrowdown" | "down" => NamedKey::ArrowDown,
        "arrowleft" | "left" => NamedKey::ArrowLeft,
        "arrowright" | "right" => NamedKey::ArrowRight,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "space" => NamedKey::Space,
        _ => {
            let mut chars = name.chars();
            return match (chars.next(), chars.next()) {
                (Some(character), None) => Ok(Key::Char(character)),
                _ => Err(flow_like_types::anyhow!("Unknown browser key")),
            };
        }
    };
    Ok(Key::Named(named))
}

#[cfg(any(feature = "execute", test))]
fn click_options(
    button: &str,
    modifiers: &[String],
    count: u8,
) -> flow_like_types::Result<flow_like_browser::input::ClickOptions> {
    Ok(flow_like_browser::input::ClickOptions {
        button: parse_button(button)?,
        modifiers: parse_modifiers(modifiers)
            .map_err(|_| flow_like_types::anyhow!("Unknown click modifier"))?,
        click_count: count,
    })
}

/// A click that opens a dialog completed (chromedriver parity).
#[cfg(feature = "execute")]
pub(crate) async fn click_element(
    _ctx: &PageContext,
    element: &Element,
    button: &str,
    modifiers: &[String],
    count: u8,
) -> flow_like_types::Result<()> {
    element
        .click(click_options(button, modifiers, count)?)
        .await?;
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn key_chord(
    ctx: &PageContext,
    key: &str,
    modifiers: &[String],
) -> flow_like_types::Result<()> {
    let modifiers = parse_modifiers(modifiers)?;
    let key = browser_key(key)?;
    ctx.page.key_chord(key, modifiers).await?;
    Ok(())
}

/// Resolves a snapshot ref of the current page to its element, in the frame that owns it.
/// A ref of another page or document is stale; there is no role/name fallback.
#[cfg(feature = "execute")]
pub(crate) async fn resolve_ref(
    ctx: &PageContext,
    value: &str,
) -> flow_like_types::Result<Element> {
    resolve_ref_on(&ctx.page, &ctx.slot, value).await
}

#[cfg(feature = "execute")]
async fn resolve_ref_on(
    page: &Page,
    slot: &BrowserSlot,
    value: &str,
) -> flow_like_types::Result<Element> {
    let reference = crate::types::selectors::normalize_ref(value).ok_or_else(|| {
        flow_like_types::anyhow!("'{value}' is not a browser snapshot ref such as 'e12'")
    })?;
    let (node, main_loader) = locked(&slot.refs, |refs| {
        if refs.table.page().is_none() {
            return Err(flow_like_types::anyhow!(
                "Element ref '{reference}' has no browser snapshot in this session — take a browser snapshot first"
            ));
        }
        let entry = refs.table.lookup(&reference)?;
        if &entry.node.page != page.target_id() {
            return Err(BrowserError::StaleRef {
                reference: reference.clone(),
            }
            .into());
        }
        Ok((entry.node.clone(), refs.table.main_loader().cloned()))
    })?;
    Ok(page
        .resolve_node(&node, &reference, main_loader.as_ref())
        .await?)
}

/// Refs of elements found outside a snapshot, when the snapshot table still describes the
/// current document; otherwise none.
#[cfg(feature = "execute")]
pub(crate) fn refs_for_elements(ctx: &PageContext, elements: &[Element]) -> Vec<Option<String>> {
    let current_loader = ctx.page.main_loader();
    locked(&ctx.slot.refs, |refs| {
        let same_document = crate::browser::refs::tree::describes_document(
            &refs.table,
            ctx.page.target_id(),
            current_loader.as_ref(),
        );
        if !same_document {
            return vec![None; elements.len()];
        }
        elements
            .iter()
            .map(|element| Some(refs.table.register(element.node_ref())))
            .collect()
    })
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn browser_error(error: &flow_like_types::Error) -> Option<&BrowserError> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<BrowserError>())
}

/// Not found or detached from the document; a stale snapshot ref is not a missing element.
#[cfg(any(feature = "execute", test))]
pub(crate) fn is_missing_element(error: &flow_like_types::Error) -> bool {
    matches!(
        browser_error(error),
        Some(BrowserError::NotFound { .. } | BrowserError::StaleElement)
    )
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn is_javascript_error(error: &flow_like_types::Error) -> bool {
    matches!(browser_error(error), Some(BrowserError::Javascript { .. }))
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn is_navigation_interrupted(error: &flow_like_types::Error) -> bool {
    browser_error(error).is_some_and(|error| {
        matches!(error, BrowserError::NavigationInterrupted)
            || matches!(
                error.class(),
                ErrorClass::AbortedByNavigation
                    | ErrorClass::NoSuchExecutionContext
                    | ErrorClass::FrameInTransit
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_browser::testing::PageHarness;
    use flow_like_browser::types::FrameId;
    use flow_like_types::Context;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn modifiers_accept_the_node_aliases_case_insensitively() {
        let all = parse_modifiers(&strings(&["Ctrl", "SHIFT", "alt", "Cmd"])).unwrap();
        for modifier in [
            Modifiers::CTRL,
            Modifiers::SHIFT,
            Modifiers::ALT,
            Modifiers::META,
        ] {
            assert!(all.contains(modifier));
        }
        for (alias, modifier) in [
            ("control", Modifiers::CTRL),
            ("Option", Modifiers::ALT),
            ("meta", Modifiers::META),
            ("command", Modifiers::META),
            ("win", Modifiers::META),
        ] {
            assert_eq!(parse_modifiers(&strings(&[alias])).unwrap(), modifier);
        }
        assert_eq!(parse_modifiers(&[]).unwrap(), Modifiers::NONE);
        assert_eq!(
            parse_modifiers(&strings(&["shift", "hyper"]))
                .unwrap_err()
                .to_string(),
            "Unknown browser modifier: hyper"
        );
    }

    #[test]
    fn buttons_are_the_three_lowercase_names() {
        assert_eq!(parse_button("left").unwrap(), MouseButton::Left);
        assert_eq!(parse_button("middle").unwrap(), MouseButton::Middle);
        assert_eq!(parse_button("right").unwrap(), MouseButton::Right);
        for rejected in ["Left", "back", ""] {
            assert_eq!(
                parse_button(rejected).unwrap_err().to_string(),
                "Supported click buttons are left, middle, and right"
            );
        }
    }

    #[test]
    fn click_options_check_the_button_before_the_modifiers() {
        let error = |button, modifiers: &[&str]| {
            click_options(button, &strings(modifiers), 1)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            error("back", &["hyper"]),
            "Supported click buttons are left, middle, and right"
        );
        assert_eq!(error("left", &["hyper"]), "Unknown click modifier");
        let options = click_options("right", &strings(&["Control", "option"]), 2).unwrap();
        assert_eq!(options.button, MouseButton::Right);
        assert_eq!(
            options.modifiers,
            Modifiers(Modifiers::CTRL.0 | Modifiers::ALT.0)
        );
        assert_eq!(options.click_count, 2);
    }

    #[test]
    fn keys_are_named_keys_or_exactly_one_character() {
        for (name, key) in [
            ("Enter", NamedKey::Enter),
            ("return", NamedKey::Enter),
            ("tab", NamedKey::Tab),
            ("Esc", NamedKey::Escape),
            ("escape", NamedKey::Escape),
            ("backspace", NamedKey::Backspace),
            ("delete", NamedKey::Delete),
            ("up", NamedKey::ArrowUp),
            ("ArrowDown", NamedKey::ArrowDown),
            ("left", NamedKey::ArrowLeft),
            ("arrowright", NamedKey::ArrowRight),
            ("home", NamedKey::Home),
            ("end", NamedKey::End),
            ("PageUp", NamedKey::PageUp),
            ("pagedown", NamedKey::PageDown),
            ("space", NamedKey::Space),
        ] {
            assert_eq!(browser_key(name).unwrap(), Key::Named(key), "{name}");
        }
        assert_eq!(browser_key("A").unwrap(), Key::Char('A'));
        assert_eq!(browser_key("é").unwrap(), Key::Char('é'));
        assert_eq!(browser_key(" ").unwrap(), Key::Char(' '));
        for rejected in ["", "F13", "ab"] {
            assert_eq!(
                browser_key(rejected).unwrap_err().to_string(),
                "Unknown browser key"
            );
        }
    }

    #[test]
    fn a_stale_ref_reads_like_the_catalog_constant() {
        let reference = "e12";
        assert_eq!(
            BrowserError::StaleRef {
                reference: reference.into()
            }
            .to_string(),
            crate::browser::refs::STALE_REF_ERROR.replace("{ref}", reference)
        );
    }

    #[test]
    fn protocol_errors_keep_their_text_and_type_through_context() {
        let error = flow_like_types::Error::from(BrowserError::Protocol {
            method: "DOM.focus".into(),
            code: -32000,
            message: "Element is not focusable".into(),
        });
        assert_eq!(
            error.to_string(),
            "Chrome DevTools command DOM.focus failed (-32000): Element is not focusable"
        );
        let wrapped = Err::<(), _>(error)
            .context("Focusing the field failed")
            .unwrap_err();
        assert!(matches!(
            browser_error(&wrapped),
            Some(BrowserError::Protocol { code: -32000, .. })
        ));
        assert!(browser_error(&flow_like_types::anyhow!("plain")).is_none());
    }

    #[test]
    fn error_predicates_follow_the_typed_error() {
        let typed = |error: BrowserError| flow_like_types::Error::from(error);
        assert!(is_missing_element(&typed(BrowserError::NotFound {
            message: "No element matches Css selector '#a'".into()
        })));
        assert!(is_missing_element(&typed(BrowserError::StaleElement)));
        assert!(!is_missing_element(&typed(BrowserError::StaleRef {
            reference: "e1".into()
        })));
        assert!(is_javascript_error(&typed(BrowserError::Javascript {
            message: "x is not defined".into()
        })));
        assert!(is_navigation_interrupted(&typed(
            BrowserError::NavigationInterrupted
        )));
        assert!(is_navigation_interrupted(&typed(BrowserError::Protocol {
            method: "Runtime.callFunctionOn".into(),
            code: -32000,
            message: "Execution context was destroyed.".into(),
        })));
        assert!(is_navigation_interrupted(&typed(
            BrowserError::FrameInTransit { frame: "F1".into() }
        )));
        assert!(!is_navigation_interrupted(&typed(BrowserError::Timeout {
            method: "Runtime.evaluate".into(),
            timeout_ms: 10,
        })));
    }

    #[tokio::test]
    async fn scripts_run_in_the_frame_of_their_element_arguments() {
        let harness = PageHarness::new().await;
        let main = harness.page.main_frame();
        harness.attach_child(
            "S2",
            "F2",
            main.id().as_str(),
            "http://127.0.0.1/child",
            "L2",
        );
        let child = harness
            .page
            .frame(&FrameId::from("F2"))
            .expect("the attached child frame is known to the page");

        assert_eq!(
            common_frame(&main, std::iter::empty()).unwrap().id(),
            main.id()
        );
        assert_eq!(
            common_frame(&main, [child.clone(), child.clone()])
                .unwrap()
                .id(),
            child.id()
        );
        let Err(mixed) = common_frame(&child, [child.clone(), main.clone()]) else {
            panic!("elements of two frames must not share one script");
        };
        assert_eq!(
            mixed.to_string(),
            "invalid argument: Script arguments come from different frames"
        );
        assert!(matches!(
            browser_error(&mixed),
            Some(BrowserError::InvalidArgument { .. })
        ));
    }

    #[cfg(feature = "execute")]
    mod refs {
        use super::super::*;
        use crate::types::handles::RefState;
        use flow_like_browser::testing::PageHarness;
        use flow_like_browser::types::{FrameId, LoaderId, TargetId};
        use flow_like_browser::{NodeRef, RefTable};

        fn node(page: &str, loader: &str) -> NodeRef {
            NodeRef {
                page: TargetId::from(page),
                local_root: TargetId::from(page),
                frame_id: FrameId::from(page),
                loader_id: LoaderId::from(loader),
                backend_node_id: 7,
            }
        }

        fn slot_with(page: &str, loader: &str) -> BrowserSlot {
            let mut allocator =
                RefTable::default().begin(&TargetId::from(page), &LoaderId::from(loader));
            let node = node(page, loader);
            let proposed = allocator.propose(&node, "button", "Sign in");
            allocator.commit(proposed, node, "button".into(), "Sign in".into());
            let slot = BrowserSlot::default();
            *slot.refs.lock().unwrap() = RefState {
                table: allocator.finish(),
                elements: Vec::new(),
            };
            slot
        }

        async fn resolve_error(page: &Page, slot: &BrowserSlot, value: &str) -> String {
            match resolve_ref_on(page, slot, value).await {
                Ok(element) => panic!(
                    "{value} resolved to node {} instead of failing",
                    element.backend_node_id()
                ),
                Err(error) => error.to_string(),
            }
        }

        #[tokio::test]
        async fn refs_need_a_snapshot_of_this_session() {
            let harness = PageHarness::new().await;
            let empty = BrowserSlot::default();
            assert_eq!(
                resolve_error(&harness.page, &empty, "button").await,
                "'button' is not a browser snapshot ref such as 'e12'"
            );
            assert_eq!(
                resolve_error(&harness.page, &empty, "[ref=e3]").await,
                "Element ref 'e3' has no browser snapshot in this session — take a browser snapshot first"
            );
        }

        #[tokio::test]
        async fn refs_of_another_page_unknown_refs_and_old_documents_are_stale() {
            let harness = PageHarness::new().await;
            let page = harness.page.target_id().as_str().to_owned();
            let stale =
                |reference: &str| crate::browser::refs::STALE_REF_ERROR.replace("{ref}", reference);

            let other_page = slot_with("T-other", "L1");
            assert_eq!(
                resolve_error(&harness.page, &other_page, "e1").await,
                stale("e1")
            );

            let this_page = slot_with(&page, "L1");
            assert_eq!(
                resolve_error(&harness.page, &this_page, "e9").await,
                stale("e9")
            );

            let no_refs = BrowserSlot::default();
            *no_refs.refs.lock().unwrap() = RefState {
                table: RefTable::default()
                    .begin(&TargetId::from(page.as_str()), &LoaderId::from("L1"))
                    .finish(),
                elements: Vec::new(),
            };
            assert_eq!(
                resolve_error(&harness.page, &no_refs, "e1").await,
                stale("e1")
            );

            let old_document = slot_with(&page, "L-before-navigation");
            let error = resolve_ref_on(&harness.page, &old_document, "@e1")
                .await
                .err()
                .expect("a ref of the previous document must not resolve");
            assert_eq!(error.to_string(), stale("e1"));
            assert!(matches!(
                browser_error(&error),
                Some(BrowserError::StaleRef { reference }) if reference == "e1"
            ));
        }
    }
}
