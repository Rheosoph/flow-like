use flow_like::{
    flow::board::commands::{
        GenericCommand, nodes::add_node::AddNodeCommand, pins::connect_pins::ConnectPinsCommand,
    },
    state::FlowLikeState,
};
use flow_like_types::json::{json, to_vec};
use flow_like_types::rand::Rng;

use crate::functions::TauriFunctionError;

use super::state::{
    ActionType, BrowserActionKind, KeyModifier, MouseButton, RecordedAction, RecordedFingerprint,
    ScrollDirection,
};

fn modifier_names(modifiers: &[KeyModifier]) -> String {
    modifiers
        .iter()
        .map(|modifier| match modifier {
            KeyModifier::Shift => "shift",
            KeyModifier::Control => "ctrl",
            KeyModifier::Alt => "alt",
            KeyModifier::Meta => "meta",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn mouse_button_name(button: &MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

fn clipboard_shortcut(key: &str, original: &RecordedAction) -> RecordedAction {
    let mut action = original.clone();
    action.id = flow_like_types::create_id();
    action.screenshot_ref = None;
    action.fingerprint = None;
    action.action_type = ActionType::KeyPress {
        key: key.to_string(),
        modifiers: vec![if cfg!(target_os = "macos") {
            KeyModifier::Meta
        } else {
            KeyModifier::Control
        }],
    };
    action
}

/// Expand clipboard operations and pointer-targeted scrolling into executable actions.
fn replay_actions(actions: &[RecordedAction]) -> Vec<RecordedAction> {
    let mut result = Vec::new();
    let mut browser_tab = String::new();
    let mut browser_frames = Vec::<String>::new();
    let mut visited_browser_tabs = std::collections::HashSet::new();
    for action in actions {
        match &action.action_type {
            ActionType::Browser { action: browser } => {
                let mut helper = |kind: BrowserActionKind, selector: String| {
                    let mut derived = action.clone();
                    let mut browser = browser.clone();
                    browser.kind = kind;
                    browser.selector = selector;
                    derived.action_type = ActionType::Browser { action: browser };
                    result.push(derived);
                };
                if browser_tab != browser.tab_id || browser_tab.is_empty() {
                    helper(BrowserActionKind::SelectTab, String::new());
                    browser_tab = browser.tab_id.clone();
                    browser_frames.clear();
                }
                if visited_browser_tabs.insert(browser.tab_id.clone())
                    && !matches!(browser.kind, BrowserActionKind::Navigate)
                {
                    helper(BrowserActionKind::Navigate, String::new());
                }
                if browser_frames != browser.frames {
                    helper(BrowserActionKind::LeaveFrame, String::new());
                    for frame in &browser.frames {
                        helper(BrowserActionKind::WaitForElement, frame.clone());
                        helper(BrowserActionKind::EnterFrame, frame.clone());
                    }
                    browser_frames = browser.frames.clone();
                }
                if matches!(
                    browser.kind,
                    BrowserActionKind::Click
                        | BrowserActionKind::DoubleClick
                        | BrowserActionKind::Type
                        | BrowserActionKind::Select
                        | BrowserActionKind::Key
                        | BrowserActionKind::Scroll
                ) {
                    helper(BrowserActionKind::WaitForElement, browser.selector.clone());
                }
                result.push(action.clone());
            }
            ActionType::Copy { .. } => {
                result.push(clipboard_shortcut("c", action));
                let mut wait = action.clone();
                wait.action_type = ActionType::Wait { milliseconds: 150 };
                result.push(wait);
                result.push(action.clone());
            }
            ActionType::Paste { .. } => {
                result.push(action.clone());
                result.push(clipboard_shortcut("v", action));
            }
            ActionType::Scroll { .. } => {
                if let Some((x, y)) = action.coordinates {
                    let mut pointer = action.clone();
                    pointer.action_type = ActionType::MouseMove { x, y };
                    result.push(pointer);
                }
                result.push(action.clone());
            }
            _ => result.push(action.clone()),
        }
    }
    result
}

fn advance_layout(
    x: &mut f32,
    y: &mut f32,
    nodes_in_row: &mut usize,
    direction: &mut f32,
    spacing: f32,
    row_spacing: f32,
    max_per_row: usize,
) {
    *nodes_in_row += 1;
    if *nodes_in_row >= max_per_row {
        *y += row_spacing;
        *direction *= -1.0;
        *nodes_in_row = 0;
    } else {
        *x += spacing * *direction;
    }
}

/// Options for workflow generation from recorded actions
#[derive(Clone, Debug)]
pub struct GeneratorOptions {
    /// Use pattern matching for clicks when screenshots are available
    pub use_pattern_matching: bool,
    /// Confidence threshold for template matching (0.0-1.0)
    pub template_confidence: f64,
    /// App ID for constructing screenshot paths
    #[allow(dead_code)]
    // app scope is applied at capture time; the generator emits upload-dir-relative paths
    pub app_id: Option<String>,
    /// Board ID for constructing screenshot paths
    pub board_id: Option<String>,
    /// Use natural curved mouse movements to avoid bot detection
    pub bot_detection_evasion: bool,
    /// When true, embed fingerprint data into click nodes for pre-action validation
    pub use_fingerprints: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            use_pattern_matching: false,
            template_confidence: 0.8,
            app_id: None,
            board_id: None,
            bot_detection_evasion: false,
            use_fingerprints: false,
        }
    }
}

pub async fn generate_add_node_commands(
    actions: &[RecordedAction],
    start_position: (f64, f64),
    state: &FlowLikeState,
    options: Option<GeneratorOptions>,
) -> Result<Vec<GenericCommand>, TauriFunctionError> {
    let opts = options.unwrap_or_default();
    let registry = state.node_registry.read().await;
    let mut commands = Vec::new();
    let mut x_offset = start_position.0 as f32;
    let mut y_offset = start_position.1 as f32;
    let node_spacing = 300.0_f32;
    let row_spacing = 400.0_f32;
    let max_nodes_per_row: usize = 8;
    let mut nodes_in_row: usize = 0;
    let mut direction: f32 = 1.0; // 1.0 = right, -1.0 = left

    let mut prev_exec_pin: Option<(String, String)>;
    let mut session_node_id: Option<String>;
    let mut session_out_pin_id: Option<String>;

    // First, add a simple_event node as the trigger
    let mut event_node = registry
        .get_node("events_simple")
        .map_err(|e| TauriFunctionError::new(&format!("events_simple node not found: {}", e)))?;
    event_node.coordinates = Some((x_offset, y_offset, 0.0));

    let add_event_cmd = AddNodeCommand::new(event_node);
    let event_node_id = add_event_cmd.node.id.clone();
    let event_exec_out = add_event_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| p.name == "exec_out" && p.pin_type == flow_like::flow::pin::PinType::Output)
        .map(|(id, _)| id.clone());

    commands.push(GenericCommand::AddNode(add_event_cmd));
    prev_exec_pin = event_exec_out.map(|pin| (event_node_id.clone(), pin));

    advance_layout(
        &mut x_offset,
        &mut y_offset,
        &mut nodes_in_row,
        &mut direction,
        node_spacing,
        row_spacing,
        max_nodes_per_row,
    );

    // Use the unified automation session that supports browser, desktop, and RPA
    let mut session = registry.get_node("automation_start_session").map_err(|e| {
        TauriFunctionError::new(&format!("automation_start_session node not found: {}", e))
    })?;
    session.coordinates = Some((x_offset, y_offset, 0.0));

    // Create the AddNodeCommand which will generate new IDs for the node and pins
    let add_session_cmd = AddNodeCommand::new(session.clone());

    // Use the ACTUAL pin IDs from the created command, not the template
    let actual_session_id = add_session_cmd.node.id.clone();
    let actual_session_exec_in = add_session_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| p.name == "exec_in" && p.pin_type == flow_like::flow::pin::PinType::Input)
        .map(|(id, _)| id.clone());
    let actual_session_exec_out = add_session_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| p.name == "exec_out" && p.pin_type == flow_like::flow::pin::PinType::Output)
        .map(|(id, _)| id.clone());
    let actual_session_handle_out = add_session_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| {
            p.friendly_name == "Session" && p.pin_type == flow_like::flow::pin::PinType::Output
        })
        .map(|(id, _)| id.clone());

    commands.push(GenericCommand::AddNode(add_session_cmd));

    // Connect event to session
    if let (Some((prev_node, prev_pin)), Some(session_exec_in)) =
        (&prev_exec_pin, &actual_session_exec_in)
    {
        commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
            prev_node.clone(),
            actual_session_id.clone(),
            prev_pin.clone(),
            session_exec_in.clone(),
        )));
    }

    session_node_id = Some(actual_session_id.clone());
    session_out_pin_id = actual_session_handle_out.clone();
    prev_exec_pin = actual_session_exec_out.map(|pin| (actual_session_id.clone(), pin));

    advance_layout(
        &mut x_offset,
        &mut y_offset,
        &mut nodes_in_row,
        &mut direction,
        node_spacing,
        row_spacing,
        max_nodes_per_row,
    );

    // Minimum delay threshold to insert a delay node (milliseconds)
    const MIN_DELAY_THRESHOLD_MS: i64 = 500;
    // Minimum delay to insert after Enter key (for page navigation)
    const MIN_DELAY_AFTER_ENTER_MS: i64 = 300;
    let mut last_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;

    // Track last Copy node's text output for connecting to subsequent Paste nodes
    let mut last_copy_text_output: Option<(String, String)> = None; // (node_id, pin_id)

    // Track if last action was an Enter key press (for adding delay before clicks)
    let mut last_was_enter = false;

    let actions = replay_actions(actions);
    for action in &actions {
        // Calculate delay from previous action
        let delay_ms = if let Some(prev_ts) = last_timestamp {
            let diff = action.timestamp.signed_duration_since(prev_ts);
            diff.num_milliseconds()
        } else {
            0
        };
        last_timestamp = Some(action.timestamp);

        // Insert delay node if there was a significant pause
        if delay_ms > MIN_DELAY_THRESHOLD_MS {
            tracing::debug!(" Adding delay node: {}ms", delay_ms);

            if let Ok(mut delay_node) = registry.get_node("delay") {
                delay_node.coordinates = Some((x_offset, y_offset, 0.0));

                // Set the delay duration (Float type, in milliseconds)
                if let Some((_, pin)) = delay_node.pins.iter_mut().find(|(_, p)| p.name == "time")
                    && let Ok(bytes) = to_vec(&json!(delay_ms as f64))
                {
                    pin.default_value = Some(bytes);
                }

                let add_delay_cmd = AddNodeCommand::new(delay_node);
                let delay_node_id = add_delay_cmd.node.id.clone();

                let delay_exec_in = add_delay_cmd
                    .node
                    .pins
                    .iter()
                    .find(|(_, p)| {
                        p.name == "exec_in" && p.pin_type == flow_like::flow::pin::PinType::Input
                    })
                    .map(|(id, _)| id.clone());

                let delay_exec_out = add_delay_cmd
                    .node
                    .pins
                    .iter()
                    .find(|(_, p)| {
                        p.name == "exec_out" && p.pin_type == flow_like::flow::pin::PinType::Output
                    })
                    .map(|(id, _)| id.clone());

                commands.push(GenericCommand::AddNode(add_delay_cmd));

                // Connect previous node to delay
                if let (Some((prev_node, prev_pin)), Some(delay_in)) =
                    (&prev_exec_pin, &delay_exec_in)
                {
                    commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                        prev_node.clone(),
                        delay_node_id.clone(),
                        prev_pin.clone(),
                        delay_in.clone(),
                    )));
                }

                // Update prev_exec_pin to delay's output
                if let Some(delay_out) = delay_exec_out {
                    prev_exec_pin = Some((delay_node_id, delay_out));
                }

                advance_layout(
                    &mut x_offset,
                    &mut y_offset,
                    &mut nodes_in_row,
                    &mut direction,
                    node_spacing,
                    row_spacing,
                    max_nodes_per_row,
                );
            }
        }
        // If last action was Enter and this is a click, insert a minimum delay for page navigation
        else if last_was_enter
            && matches!(
                action.action_type,
                ActionType::Click { .. } | ActionType::DoubleClick { .. }
            )
        {
            tracing::debug!(
                "Adding delay after Enter before click: {}ms",
                MIN_DELAY_AFTER_ENTER_MS
            );

            if let Ok(mut delay_node) = registry.get_node("delay") {
                delay_node.coordinates = Some((x_offset, y_offset, 0.0));

                if let Some((_, pin)) = delay_node.pins.iter_mut().find(|(_, p)| p.name == "time")
                    && let Ok(bytes) = to_vec(&json!(MIN_DELAY_AFTER_ENTER_MS as f64))
                {
                    pin.default_value = Some(bytes);
                }

                let add_delay_cmd = AddNodeCommand::new(delay_node);
                let delay_node_id = add_delay_cmd.node.id.clone();

                let delay_exec_in = add_delay_cmd
                    .node
                    .pins
                    .iter()
                    .find(|(_, p)| {
                        p.name == "exec_in" && p.pin_type == flow_like::flow::pin::PinType::Input
                    })
                    .map(|(id, _)| id.clone());

                let delay_exec_out = add_delay_cmd
                    .node
                    .pins
                    .iter()
                    .find(|(_, p)| {
                        p.name == "exec_out" && p.pin_type == flow_like::flow::pin::PinType::Output
                    })
                    .map(|(id, _)| id.clone());

                commands.push(GenericCommand::AddNode(add_delay_cmd));

                if let (Some((prev_node, prev_pin)), Some(delay_in)) =
                    (&prev_exec_pin, &delay_exec_in)
                {
                    commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                        prev_node.clone(),
                        delay_node_id.clone(),
                        prev_pin.clone(),
                        delay_in.clone(),
                    )));
                }

                if let Some(delay_out) = delay_exec_out {
                    prev_exec_pin = Some((delay_node_id, delay_out));
                }

                advance_layout(
                    &mut x_offset,
                    &mut y_offset,
                    &mut nodes_in_row,
                    &mut direction,
                    node_spacing,
                    row_spacing,
                    max_nodes_per_row,
                );
            }
        }

        tracing::debug!(action_id = %action.id, "Generating recorded action");

        // Track helper nodes needed for pattern matching (path_from_storage_dir, child)
        let mut helper_commands: Vec<GenericCommand> = Vec::new();
        let mut template_path_node_id: Option<String> = None;
        let mut template_path_out_pin_id: Option<String> = None;
        // Track fingerprint node for connecting to click nodes
        let mut fingerprint_node_id: Option<String> = None;
        let mut fingerprint_out_pin_id: Option<String> = None;
        let mut fingerprint_exec_in_pin_id: Option<String> = None;
        let mut fingerprint_exec_out_pin_id: Option<String> = None;

        // Create upload_dir → child FlowPath chain whenever a screenshot is available
        if opts.use_pattern_matching
            && matches!(
                action.action_type,
                ActionType::Click { .. } | ActionType::DoubleClick { .. }
            )
            && let Some(ref screenshot_id) = action.screenshot_ref
        {
            let screenshot_path = match &opts.board_id {
                Some(bid) => format!("rpa/{}/screenshots/{}.png", bid, screenshot_id),
                None => format!("rpa/screenshots/{}.png", screenshot_id),
            };

            if let Ok(mut upload_dir_node) = registry.get_node("path_from_upload_dir") {
                upload_dir_node.coordinates = Some((x_offset - 400.0, y_offset + 200.0, 0.0));
                let upload_dir_cmd = AddNodeCommand::new(upload_dir_node);
                let upload_dir_id = upload_dir_cmd.node.id.clone();

                let upload_path_out = upload_dir_cmd
                    .node
                    .pins
                    .iter()
                    .find(|(_, p)| {
                        p.name == "path" && p.pin_type == flow_like::flow::pin::PinType::Output
                    })
                    .map(|(id, _)| id.clone());

                helper_commands.push(GenericCommand::AddNode(upload_dir_cmd));

                if let (Ok(mut child_node), Some(upload_out)) =
                    (registry.get_node("child"), upload_path_out)
                {
                    child_node.coordinates = Some((x_offset - 200.0, y_offset + 200.0, 0.0));

                    if let Some((_, pin)) = child_node
                        .pins
                        .iter_mut()
                        .find(|(_, p)| p.name == "child_name")
                        && let Ok(bytes) = to_vec(&json!(screenshot_path))
                    {
                        pin.default_value = Some(bytes);
                    }

                    let child_cmd = AddNodeCommand::new(child_node);
                    let child_node_id = child_cmd.node.id.clone();

                    let child_path_in = child_cmd
                        .node
                        .pins
                        .iter()
                        .find(|(_, p)| {
                            p.name == "parent_path"
                                && p.pin_type == flow_like::flow::pin::PinType::Input
                        })
                        .map(|(id, _)| id.clone());

                    let child_path_out = child_cmd
                        .node
                        .pins
                        .iter()
                        .find(|(_, p)| {
                            p.name == "path" && p.pin_type == flow_like::flow::pin::PinType::Output
                        })
                        .map(|(id, _)| id.clone());

                    helper_commands.push(GenericCommand::AddNode(child_cmd));

                    if let Some(child_in) = child_path_in {
                        helper_commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                            upload_dir_id,
                            child_node_id.clone(),
                            upload_out,
                            child_in,
                        )));
                    }

                    template_path_node_id = Some(child_node_id);
                    template_path_out_pin_id = child_path_out;
                }
            }
        }

        // Generate fingerprint_create node before clicks if fingerprint data is available
        let is_click = matches!(
            &action.action_type,
            ActionType::Click { .. } | ActionType::DoubleClick { .. }
        );
        if is_click
            && !opts.use_pattern_matching
            && !opts.use_fingerprints
            && action.coordinates.is_none()
        {
            return Err(TauriFunctionError::new(
                "The recorded click has no coordinates. Record that action again.",
            ));
        }
        if is_click && opts.use_pattern_matching && action.screenshot_ref.is_none() {
            return Err(TauriFunctionError::new(
                "A recorded click has no screenshot template. Record it again or disable pattern matching before inserting.",
            ));
        }
        if is_click && opts.use_pattern_matching && template_path_out_pin_id.is_none() {
            return Err(TauriFunctionError::new(
                "The catalog is missing the screenshot path nodes needed for pattern matching",
            ));
        }
        if is_click
            && !opts.use_pattern_matching
            && opts.use_fingerprints
            && action.fingerprint.is_none()
        {
            return Err(TauriFunctionError::new(
                "A recorded click has no element fingerprint. Record it again or disable element fingerprinting before inserting.",
            ));
        }
        if opts.use_fingerprints
            && !opts.use_pattern_matching
            && is_click
            && let Some(fp) = &action.fingerprint
            && let Some(fp_cmds) =
                generate_fingerprint_node(fp, &registry, x_offset, y_offset - 180.0)
        {
            fingerprint_node_id = Some(fp_cmds.node_id.clone());
            fingerprint_out_pin_id = Some(fp_cmds.fingerprint_out_pin_id.clone());
            fingerprint_exec_in_pin_id = fp_cmds.exec_in_pin_id;
            fingerprint_exec_out_pin_id = fp_cmds.exec_out_pin_id;
            for cmd in fp_cmds.commands {
                helper_commands.push(cmd);
            }
        }
        if is_click
            && opts.use_fingerprints
            && !opts.use_pattern_matching
            && fingerprint_out_pin_id.is_none()
        {
            return Err(TauriFunctionError::new(
                "The catalog is missing the fingerprint node needed for element matching",
            ));
        }

        let (node_name, extra_pins, _uses_rpa_session) = match &action.action_type {
            ActionType::BrowserAttach {
                debugger_address,
                webdriver_url,
                browser_type,
            } => (
                "browser_attach",
                vec![
                    ("debugger_address", json!(debugger_address)),
                    ("webdriver_url", json!(webdriver_url)),
                    ("browser_type", json!(browser_type)),
                ],
                false,
            ),
            ActionType::Browser { action } => {
                let selector = ("selector", json!(action.selector));
                match action.kind {
                    BrowserActionKind::Navigate => {
                        ("browser_goto", vec![("url", json!(action.url))], false)
                    }
                    BrowserActionKind::Click => (
                        "browser_click",
                        vec![
                            selector,
                            ("button", json!(mouse_button_name(&action.button))),
                            (
                                "modifiers",
                                json!(
                                    action
                                        .modifiers
                                        .iter()
                                        .map(|modifier| modifier_names(std::slice::from_ref(
                                            modifier
                                        )))
                                        .collect::<Vec<_>>()
                                ),
                            ),
                        ],
                        false,
                    ),
                    BrowserActionKind::DoubleClick => (
                        "browser_double_click",
                        vec![
                            selector,
                            ("button", json!(mouse_button_name(&action.button))),
                            (
                                "modifiers",
                                json!(
                                    action
                                        .modifiers
                                        .iter()
                                        .map(|modifier| modifier_names(std::slice::from_ref(
                                            modifier
                                        )))
                                        .collect::<Vec<_>>()
                                ),
                            ),
                        ],
                        false,
                    ),
                    BrowserActionKind::Type => (
                        "browser_type_text",
                        vec![
                            selector,
                            ("text", json!(action.value)),
                            ("clear_first", json!(true)),
                        ],
                        false,
                    ),
                    BrowserActionKind::Select => (
                        "browser_select_option",
                        vec![selector, ("value", json!(action.value))],
                        false,
                    ),
                    BrowserActionKind::Key => (
                        "browser_press_key",
                        vec![
                            selector,
                            ("key", json!(action.value)),
                            (
                                "modifiers",
                                json!(
                                    action
                                        .modifiers
                                        .iter()
                                        .map(|modifier| modifier_names(std::slice::from_ref(
                                            modifier
                                        )))
                                        .collect::<Vec<_>>()
                                ),
                            ),
                        ],
                        false,
                    ),
                    BrowserActionKind::SelectTab => (
                        "browser_select_tab",
                        vec![
                            ("target_id", json!(action.tab_id)),
                            ("url", json!(action.url)),
                        ],
                        false,
                    ),
                    BrowserActionKind::WaitForUrl => (
                        "browser_wait_for_url",
                        vec![
                            ("expected_url", json!(action.url)),
                            ("timeout_ms", json!(30000)),
                        ],
                        false,
                    ),
                    BrowserActionKind::WaitForElement => (
                        "browser_wait_for",
                        vec![selector, ("timeout_ms", json!(30000))],
                        false,
                    ),
                    BrowserActionKind::EnterFrame => ("browser_enter_frame", vec![selector], false),
                    BrowserActionKind::LeaveFrame => (
                        "browser_leave_frame",
                        vec![("top_level", json!(true))],
                        false,
                    ),
                    BrowserActionKind::Scroll => (
                        "browser_execute_js",
                        vec![(
                            "script",
                            json!(format!(
                                "const element = document.querySelector({}); if (!element) throw new Error('Recorded scroll target is missing'); element.scrollTo({}, {});",
                                json!(action.selector),
                                action.scroll_x,
                                action.scroll_y
                            )),
                        )],
                        false,
                    ),
                }
            }
            ActionType::Click { button, modifiers }
            | ActionType::DoubleClick { button, modifiers } => {
                let (x, y) = action.coordinates.unwrap_or((0, 0));
                let mut pins = vec![
                    ("x", json!(x)),
                    ("y", json!(y)),
                    ("button", json!(mouse_button_name(button))),
                    ("modifiers", json!(modifier_names(modifiers))),
                    (
                        "use_fingerprint",
                        json!(opts.use_fingerprints && !opts.use_pattern_matching),
                    ),
                    ("use_template_matching", json!(opts.use_pattern_matching)),
                    ("confidence", json!(opts.template_confidence)),
                ];
                if opts.bot_detection_evasion {
                    let mut rng = flow_like_types::rand::rng();
                    pins.push(("natural_move", json!(true)));
                    pins.push(("move_duration_ms", json!(rng.random_range(150..350))));
                }
                (
                    if matches!(action.action_type, ActionType::DoubleClick { .. }) {
                        "computer_mouse_double_click"
                    } else {
                        "computer_mouse_click"
                    },
                    pins,
                    false,
                )
            }
            ActionType::Drag {
                start,
                end,
                button,
                modifiers,
            } => (
                "computer_mouse_drag",
                vec![
                    ("start_x", json!(start.0)),
                    ("start_y", json!(start.1)),
                    ("end_x", json!(end.0)),
                    ("end_y", json!(end.1)),
                    ("button", json!(mouse_button_name(button))),
                    ("modifiers", json!(modifier_names(modifiers))),
                ],
                false,
            ),
            ActionType::MouseMove { x, y } => (
                "computer_mouse_move",
                vec![("x", json!(x)), ("y", json!(y))],
                false,
            ),
            ActionType::Wait { milliseconds } => {
                ("delay", vec![("time", json!(*milliseconds as f64))], false)
            }
            ActionType::Scroll { direction, amount } => {
                // Skip scroll events with 0 amount
                if *amount == 0 {
                    continue;
                }

                // rdev on macOS reports line-level deltas (typically 1-5 per event).
                // After consolidation the accumulated amount is already in scroll-line units.
                // Pass through directly — enigo.scroll(1) sends one line tick.
                let lines = *amount;
                let (dx, dy) = match direction {
                    ScrollDirection::Down => (0, lines),
                    ScrollDirection::Up => (0, -lines),
                    ScrollDirection::Left => (-lines, 0),
                    ScrollDirection::Right => (lines, 0),
                };
                (
                    "computer_scroll",
                    vec![("dx", json!(dx)), ("dy", json!(dy))],
                    false,
                )
            }
            ActionType::KeyType { text } => {
                ("computer_key_type", vec![("text", json!(text))], false)
            }
            ActionType::KeyPress { key, modifiers } => {
                let modifier_str = modifier_names(modifiers);
                (
                    "computer_key_press",
                    vec![("key", json!(key)), ("modifiers", json!(modifier_str))],
                    false,
                )
            }
            ActionType::AppLaunch {
                app_name: _,
                app_path,
            } => (
                "computer_launch_app",
                vec![("path", json!(app_path))],
                false,
            ),
            ActionType::WindowFocus {
                window_title,
                process,
            } => (
                "computer_focus_window",
                vec![
                    ("window_title", json!(window_title)),
                    ("process_name", json!(process)),
                    ("launch_if_not_found", json!(false)),
                ],
                false,
            ),
            ActionType::Copy {
                clipboard_content: _,
            } => {
                // Copy reads from clipboard - we'll track its output to connect to Paste
                ("computer_clipboard_get_text", vec![], false)
            }
            ActionType::Paste { clipboard_content } => {
                // For Paste, we write to clipboard
                // If we have a previous Copy, we'll connect them; otherwise use captured content
                let text = clipboard_content.clone().unwrap_or_default();
                (
                    "computer_clipboard_set_text",
                    vec![("text", json!(text))],
                    false,
                )
            }
        };

        tracing::debug!(" Mapped to node: {}", node_name);
        let mut node = match registry.get_node(node_name) {
            Ok(n) => n,
            Err(error) => {
                return Err(TauriFunctionError::new(&format!(
                    "Cannot replay action: node {node_name} is unavailable: {error}"
                )));
            }
        };
        node.coordinates = Some((x_offset, y_offset, 0.0));

        // Annotate click nodes with fingerprint context for debugging
        if is_click && let Some(fp) = &action.fingerprint {
            let parts: Vec<String> = [
                fp.role.as_ref().map(|r| format!("Role: {}", r)),
                fp.name.as_ref().map(|n| format!("Name: {}", n)),
                fp.text.as_ref().map(|t| format!("Text: {}", t)),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !parts.is_empty() {
                node.description = format!("{} | Target: [{}]", node.description, parts.join(", "));
            }
        }

        for (pin_name, value) in &extra_pins {
            let (_, pin) = node
                .pins
                .iter_mut()
                .find(|(_, p)| p.name == *pin_name)
                .ok_or_else(|| {
                    TauriFunctionError::new(&format!(
                        "Node {node_name} is missing required replay pin {pin_name}"
                    ))
                })?;
            pin.default_value =
                Some(to_vec(value).map_err(|error| TauriFunctionError::new(&error.to_string()))?);
        }

        // Create the AddNodeCommand which generates new IDs
        let add_cmd = AddNodeCommand::new(node);
        let new_node_id = add_cmd.node.id.clone();

        // Extract pin IDs from the CREATED node with new IDs, not the template
        let exec_in_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| {
                p.name == "exec_in" && p.pin_type == flow_like::flow::pin::PinType::Input
            })
            .map(|(id, _)| id.clone());

        let exec_out_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| {
                p.name == "exec_out" && p.pin_type == flow_like::flow::pin::PinType::Output
            })
            .map(|(id, _)| id.clone());

        let session_in_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| {
                (p.friendly_name == "Session" || p.friendly_name == "RPA Session")
                    && p.pin_type == flow_like::flow::pin::PinType::Input
            })
            .map(|(id, _)| id.clone());

        let new_session_out_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| {
                (p.friendly_name == "Session" || p.friendly_name == "RPA Session")
                    && p.pin_type == flow_like::flow::pin::PinType::Output
            })
            .map(|(id, _)| id.clone());

        // For vision_click_template, find the template pin to connect FlowPath
        let template_in_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| {
                p.name == "template" && p.pin_type == flow_like::flow::pin::PinType::Input
            })
            .map(|(id, _)| id.clone());

        // Extract text pins for Copy/Paste connection before add_cmd is moved
        let text_output_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| p.name == "text" && p.pin_type == flow_like::flow::pin::PinType::Output)
            .map(|(id, _)| id.clone());
        let text_input_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| p.name == "text" && p.pin_type == flow_like::flow::pin::PinType::Input)
            .map(|(id, _)| id.clone());

        let fingerprint_in_pin = add_cmd
            .node
            .pins
            .iter()
            .find(|(_, p)| {
                p.name == "fingerprint" && p.pin_type == flow_like::flow::pin::PinType::Input
            })
            .map(|(id, _)| id.clone());

        // Add helper nodes first (path_from_storage_dir, child) for pattern matching
        for cmd in helper_commands {
            commands.push(cmd);
        }

        // Add the node command BEFORE trying to connect its pins
        commands.push(GenericCommand::AddNode(add_cmd));

        // Connect the screenshot template to the computer click node.
        if let (Some(path_node), Some(path_out), Some(template_in)) = (
            &template_path_node_id,
            &template_path_out_pin_id,
            &template_in_pin,
        ) {
            commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                path_node.clone(),
                new_node_id.clone(),
                path_out.clone(),
                template_in.clone(),
            )));
        }

        // Wire fingerprint node into execution chain: prev → fingerprint → action node
        if let (Some(fp_id), Some(fp_exec_in), Some(fp_exec_out)) = (
            &fingerprint_node_id,
            &fingerprint_exec_in_pin_id,
            &fingerprint_exec_out_pin_id,
        ) {
            // Connect prev_exec → fingerprint.exec_in
            if let Some((prev_node, prev_pin)) = &prev_exec_pin {
                commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                    prev_node.clone(),
                    fp_id.clone(),
                    prev_pin.clone(),
                    fp_exec_in.clone(),
                )));
            }
            // Connect fingerprint.exec_out → action_node.exec_in
            if let Some(curr_pin) = &exec_in_pin {
                commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                    fp_id.clone(),
                    new_node_id.clone(),
                    fp_exec_out.clone(),
                    curr_pin.clone(),
                )));
            }
            // Connect fingerprint.fingerprint_out → action_node.fingerprint_in
            if let (Some(fp_out), Some(fp_in)) = (&fingerprint_out_pin_id, &fingerprint_in_pin) {
                commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                    fp_id.clone(),
                    new_node_id.clone(),
                    fp_out.clone(),
                    fp_in.clone(),
                )));
            }
        } else if let (Some((prev_node, prev_pin)), Some(curr_pin)) = (&prev_exec_pin, &exec_in_pin)
        {
            // No fingerprint node — connect directly as before
            commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                prev_node.clone(),
                new_node_id.clone(),
                prev_pin.clone(),
                curr_pin.clone(),
            )));
        }

        if let (Some(session_node), Some(session_pin), Some(curr_session_pin)) =
            (&session_node_id, &session_out_pin_id, &session_in_pin)
        {
            commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                session_node.clone(),
                new_node_id.clone(),
                session_pin.clone(),
                curr_session_pin.clone(),
            )));
        }

        // Handle Copy/Paste node connections
        if matches!(&action.action_type, ActionType::Copy { .. }) {
            // Track Copy node's text output for later Paste connection
            if let Some(pin_id) = text_output_pin {
                last_copy_text_output = Some((new_node_id.clone(), pin_id));
            }
        }

        if matches!(&action.action_type, ActionType::Paste { .. }) {
            // Connect previous Copy's text output to this Paste's text input
            if let Some((copy_node_id, copy_text_pin)) = &last_copy_text_output
                && let Some(paste_text_pin) = text_input_pin
            {
                commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                    copy_node_id.clone(),
                    new_node_id.clone(),
                    copy_text_pin.clone(),
                    paste_text_pin,
                )));
            }
        }

        if let Some(exec_out) = exec_out_pin {
            prev_exec_pin = Some((new_node_id.clone(), exec_out));
        }

        if let Some(new_session_out) = new_session_out_pin {
            session_node_id = Some(new_node_id.clone());
            session_out_pin_id = Some(new_session_out);
        }

        // Track if this was an Enter key press for next iteration
        last_was_enter = matches!(
            &action.action_type,
            ActionType::KeyPress { key, .. } if key == "Enter" || key == "Return"
        );

        advance_layout(
            &mut x_offset,
            &mut y_offset,
            &mut nodes_in_row,
            &mut direction,
            node_spacing,
            row_spacing,
            max_nodes_per_row,
        );
    }

    Ok(commands)
}

struct FingerprintNodeResult {
    node_id: String,
    fingerprint_out_pin_id: String,
    exec_in_pin_id: Option<String>,
    exec_out_pin_id: Option<String>,
    commands: Vec<GenericCommand>,
}

fn generate_fingerprint_node(
    fp: &RecordedFingerprint,
    registry: &flow_like::state::FlowNodeRegistry,
    x: f32,
    y: f32,
) -> Option<FingerprintNodeResult> {
    let mut node = registry.get_node("fingerprint_create").ok()?;
    node.coordinates = Some((x, y, 0.0));

    // Set the fingerprint ID
    if let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == "id")
        && let Ok(bytes) = to_vec(&json!(fp.id))
    {
        pin.default_value = Some(bytes);
    }

    // Set role, name, text
    if let Some(role) = &fp.role
        && let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == "role")
        && let Ok(bytes) = to_vec(&json!(role))
    {
        pin.default_value = Some(bytes);
    }
    if let Some(name) = &fp.name
        && let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == "name")
        && let Ok(bytes) = to_vec(&json!(name))
    {
        pin.default_value = Some(bytes);
    }
    if let Some(text) = &fp.text
        && let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == "text")
        && let Ok(bytes) = to_vec(&json!(text))
    {
        pin.default_value = Some(bytes);
    }

    // Set bounding box if available
    if let Some((x1, y1, x2, y2)) = &fp.bounding_box {
        let bbox_json = json!({
            "x1": *x1 as f32,
            "y1": *y1 as f32,
            "x2": *x2 as f32,
            "y2": *y2 as f32,
            "score": 1.0,
            "class_idx": -1,
            "class_name": null
        });
        if let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == "bounding_box")
            && let Ok(bytes) = to_vec(&bbox_json)
        {
            pin.default_value = Some(bytes);
        }
    }

    // Build selectors from available fingerprint data
    let mut selectors_json = json!({"selectors": [], "fallback_order": []});
    {
        let mut selectors = Vec::new();
        let mut order = Vec::new();
        if let Some(role) = &fp.role {
            order.push(selectors.len());
            selectors
                .push(json!({"kind": "Role", "value": role, "confidence": 0.8, "scope": null}));
        }
        if let Some(name) = &fp.name {
            order.push(selectors.len());
            selectors.push(
                json!({"kind": "AriaLabel", "value": name, "confidence": 0.9, "scope": null}),
            );
        }
        if let Some(text) = &fp.text {
            order.push(selectors.len());
            selectors
                .push(json!({"kind": "Text", "value": text, "confidence": 0.7, "scope": null}));
        }
        if !selectors.is_empty() {
            selectors_json = json!({"selectors": selectors, "fallback_order": order});
        }
    }
    if let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == "selectors")
        && let Ok(bytes) = to_vec(&selectors_json)
    {
        pin.default_value = Some(bytes);
    }

    let add_cmd = AddNodeCommand::new(node);
    let node_id = add_cmd.node.id.clone();

    let fingerprint_out = add_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| {
            p.name == "fingerprint" && p.pin_type == flow_like::flow::pin::PinType::Output
        })
        .map(|(id, _)| id.clone())?;

    let exec_in = add_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| p.name == "exec_in" && p.pin_type == flow_like::flow::pin::PinType::Input)
        .map(|(id, _)| id.clone());

    let exec_out = add_cmd
        .node
        .pins
        .iter()
        .find(|(_, p)| p.name == "exec_out" && p.pin_type == flow_like::flow::pin::PinType::Output)
        .map(|(id, _)| id.clone());

    Some(FingerprintNodeResult {
        node_id,
        fingerprint_out_pin_id: fingerprint_out,
        exec_in_pin_id: exec_in,
        exec_out_pin_id: exec_out,
        commands: vec![GenericCommand::AddNode(add_cmd)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::{
        flow::{
            node::Node,
            pin::{PinType, ValueType},
            variable::VariableType,
        },
        state::{FlowLikeConfig, FlowNodeRegistryInner},
        utils::http::HTTPClient,
    };
    use std::sync::Arc;

    async fn generate(
        actions: &[RecordedAction],
        options: GeneratorOptions,
    ) -> Vec<GenericCommand> {
        try_generate(actions, options).await.unwrap()
    }

    async fn try_generate(
        actions: &[RecordedAction],
        options: GeneratorOptions,
    ) -> Result<Vec<GenericCommand>, TauriFunctionError> {
        let state = FlowLikeState::new(FlowLikeConfig::new(), HTTPClient::new_without_refetch());
        state.node_registry.write().await.node_registry = Arc::new(FlowNodeRegistryInner::prepare(
            &Arc::new(flow_like_catalog::get_catalog()),
        ));
        generate_add_node_commands(actions, (0.0, 0.0), &state, Some(options)).await
    }

    fn nodes(commands: &[GenericCommand]) -> Vec<&Node> {
        commands
            .iter()
            .filter_map(|command| match command {
                GenericCommand::AddNode(command) => Some(&command.node),
                _ => None,
            })
            .collect()
    }

    fn pin(node: &Node, name: &str) -> serde_json::Value {
        let pin = node
            .pins
            .values()
            .find(|pin| pin.name == name && pin.pin_type == PinType::Input)
            .unwrap();
        serde_json::from_slice(pin.default_value.as_ref().unwrap()).unwrap()
    }

    fn verify_connections(commands: &[GenericCommand]) {
        let nodes = nodes(commands);
        for command in commands {
            if let GenericCommand::ConnectPin(connection) = command {
                let from = nodes
                    .iter()
                    .find(|node| node.id == connection.from_node)
                    .unwrap()
                    .pins
                    .get(&connection.from_pin)
                    .unwrap();
                let to = nodes
                    .iter()
                    .find(|node| node.id == connection.to_node)
                    .unwrap()
                    .pins
                    .get(&connection.to_pin)
                    .unwrap();
                assert_eq!(from.pin_type, PinType::Output);
                assert_eq!(to.pin_type, PinType::Input);
                assert_eq!(from.data_type, to.data_type, "{} -> {}", from.name, to.name);
                assert_eq!(from.value_type, to.value_type);
            }
        }
    }

    #[tokio::test]
    async fn visual_matching_requires_artifacts_and_never_silently_uses_coordinates() {
        let action = RecordedAction::new(
            "click",
            ActionType::Click {
                button: MouseButton::Left,
                modifiers: vec![],
            },
        )
        .with_coordinates(10, 20);
        assert!(
            try_generate(
                &[action.clone()],
                GeneratorOptions {
                    use_pattern_matching: true,
                    ..Default::default()
                }
            )
            .await
            .is_err()
        );
        assert!(
            try_generate(
                &[action.clone()],
                GeneratorOptions {
                    use_pattern_matching: false,
                    use_fingerprints: true,
                    ..Default::default()
                }
            )
            .await
            .is_err()
        );
        let mut captured = action;
        captured.screenshot_ref = Some("snapshot".into());
        captured.fingerprint = Some(RecordedFingerprint {
            role: Some("Button".into()),
            name: Some("Submit".into()),
            ..Default::default()
        });
        let commands = generate(
            &[captured],
            GeneratorOptions {
                board_id: Some("board".into()),
                use_pattern_matching: true,
                use_fingerprints: true,
                ..Default::default()
            },
        )
        .await;
        let nodes = nodes(&commands);
        let click = nodes
            .iter()
            .find(|node| node.name == "computer_mouse_click")
            .unwrap();
        assert_eq!(pin(click, "use_template_matching"), json!(true));
        assert_eq!(pin(click, "use_fingerprint"), json!(false));
        assert!(nodes.iter().any(|node| node.name == "path_from_upload_dir"));
        assert!(!nodes.iter().any(|node| node.name == "fingerprint_create"));
        verify_connections(&commands);
    }

    #[tokio::test]
    async fn default_replay_preserves_mouse_buttons_modifiers_and_disabled_matching() {
        let mut action = RecordedAction::new(
            "click",
            ActionType::Click {
                button: MouseButton::Right,
                modifiers: vec![KeyModifier::Shift],
            },
        )
        .with_coordinates(-200, 75);
        action.screenshot_ref = Some("snapshot".into());
        action.fingerprint = Some(RecordedFingerprint {
            id: "fingerprint".into(),
            ..Default::default()
        });
        let commands = generate(
            &[action],
            GeneratorOptions {
                use_pattern_matching: false,
                use_fingerprints: false,
                ..Default::default()
            },
        )
        .await;
        let nodes = nodes(&commands);
        let click = nodes
            .iter()
            .find(|node| node.name == "computer_mouse_click")
            .unwrap();
        assert_eq!(pin(click, "button"), json!("right"));
        assert_eq!(pin(click, "modifiers"), json!("shift"));
        assert_eq!(pin(click, "x"), json!(-200));
        assert_eq!(pin(click, "use_template_matching"), json!(false));
        assert_eq!(pin(click, "use_fingerprint"), json!(false));
        assert!(
            !nodes.iter().any(
                |node| node.name == "fingerprint_create" || node.name == "path_from_upload_dir"
            )
        );
        verify_connections(&commands);
    }

    #[tokio::test]
    async fn clipboard_replay_invokes_shortcuts_and_scroll_moves_to_its_hover_target() {
        let actions = [
            RecordedAction::new(
                "copy",
                ActionType::Copy {
                    clipboard_content: None,
                },
            ),
            RecordedAction::new(
                "paste",
                ActionType::Paste {
                    clipboard_content: Some("recorded".into()),
                },
            ),
            RecordedAction::new(
                "scroll",
                ActionType::Scroll {
                    direction: ScrollDirection::Down,
                    amount: 150,
                },
            )
            .with_coordinates(450, 200),
        ];
        let commands = generate(&actions, GeneratorOptions::default()).await;
        let nodes = nodes(&commands);
        let names: Vec<_> = nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "events_simple",
                "automation_start_session",
                "computer_key_press",
                "delay",
                "computer_clipboard_get_text",
                "computer_clipboard_set_text",
                "computer_key_press",
                "computer_mouse_move",
                "computer_scroll"
            ]
        );
        let keys: Vec<_> = nodes
            .iter()
            .filter(|node| node.name == "computer_key_press")
            .map(|node| pin(node, "key"))
            .collect();
        assert_eq!(keys, [json!("c"), json!("v")]);
        let pointer = nodes
            .iter()
            .find(|node| node.name == "computer_mouse_move")
            .unwrap();
        assert_eq!(pin(pointer, "x"), json!(450));
        assert_eq!(pin(nodes.last().unwrap(), "dy"), json!(150));
        let copy = nodes
            .iter()
            .find(|node| node.name == "computer_clipboard_get_text")
            .unwrap();
        let paste = nodes
            .iter()
            .find(|node| node.name == "computer_clipboard_set_text")
            .unwrap();
        assert!(commands.iter().any(|command| matches!(command, GenericCommand::ConnectPin(connection) if connection.from_node == copy.id && connection.to_node == paste.id && copy.pins[&connection.from_pin].name == "text" && paste.pins[&connection.to_pin].name == "text")));
        verify_connections(&commands);
    }

    #[tokio::test]
    async fn semantic_browser_replay_connects_real_catalog_nodes_and_frame_context() {
        let action = serde_json::from_value::<super::super::state::BrowserAction>(json!({ "kind":"click", "tab_id":"tab", "selector":"#submit", "url":"https://example.test/form", "frames":["iframe[name=form]"], "button":"Middle", "modifiers":["Control"] })).unwrap();
        let actions = [
            RecordedAction::new(
                "attach",
                ActionType::BrowserAttach {
                    debugger_address: "127.0.0.1:9222".into(),
                    webdriver_url: "http://127.0.0.1:9515".into(),
                    browser_type: "Chrome".into(),
                },
            ),
            RecordedAction::new("click", ActionType::Browser { action }),
        ];
        let commands = generate(&actions, GeneratorOptions::default()).await;
        let nodes = nodes(&commands);
        let names: Vec<_> = nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "events_simple",
                "automation_start_session",
                "browser_attach",
                "browser_select_tab",
                "browser_goto",
                "browser_leave_frame",
                "browser_wait_for",
                "browser_enter_frame",
                "browser_wait_for",
                "browser_click"
            ]
        );
        let click = nodes.last().unwrap();
        assert_eq!(pin(click, "button"), json!("middle"));
        assert_eq!(pin(click, "modifiers"), json!(["ctrl"]));
        let modifiers = click
            .pins
            .values()
            .find(|pin| pin.name == "modifiers")
            .unwrap();
        assert_eq!(modifiers.data_type, VariableType::String);
        assert_eq!(modifiers.value_type, ValueType::Array);
        let waited_for: Vec<_> = nodes
            .iter()
            .filter(|node| node.name == "browser_wait_for")
            .map(|node| pin(node, "selector"))
            .collect();
        assert_eq!(waited_for, [json!("iframe[name=form]"), json!("#submit")]);
        verify_connections(&commands);
    }
}
