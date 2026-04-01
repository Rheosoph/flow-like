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
    ActionType, KeyModifier, MouseButton, RecordedAction, RecordedFingerprint, ScrollDirection,
};

const BROWSER_PROCESSES: &[&str] = &[
    "safari",
    "google chrome",
    "chrome",
    "chromium",
    "firefox",
    "microsoft edge",
    "msedge",
    "arc",
    "brave browser",
    "brave",
    "opera",
    "vivaldi",
    "orion",
    "zen",
    "floorp",
    "waterfox",
    "iexplore",
    "edge",
];

fn is_browser_process(process_name: &str) -> bool {
    let lower = process_name.to_lowercase();
    BROWSER_PROCESSES.iter().any(|b| lower.contains(b))
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
            use_pattern_matching: true,
            template_confidence: 0.8,
            app_id: None,
            board_id: None,
            bot_detection_evasion: false,
            use_fingerprints: true,
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

    let mut prev_exec_pin: Option<(String, String)> = None;
    let mut session_node_id: Option<String> = None;
    let mut session_out_pin_id: Option<String> = None;

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

    // Track the current process name to detect browser context
    let mut current_process: Option<String> = None;

    for action in actions {
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

        tracing::debug!(" Processing action: {:?}", action.action_type);

        // Update current_process from action metadata if available
        if let Some(ref proc) = action.metadata.process_name
            && !proc.is_empty()
        {
            current_process = Some(proc.clone());
        }

        let in_browser = current_process
            .as_deref()
            .map(is_browser_process)
            .unwrap_or(false);

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
        if let Some(ref screenshot_id) = action.screenshot_ref {
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
        if opts.use_fingerprints
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

        let (node_name, extra_pins, _uses_rpa_session) = match &action.action_type {
            ActionType::Click {
                button,
                modifiers: _,
            } => {
                let (x, y) = action.coordinates.unwrap_or((0, 0));
                let button_str = match button {
                    MouseButton::Left => "Left",
                    MouseButton::Right => "Right",
                    MouseButton::Middle => "Middle",
                };

                // Use vision_click_template if pattern matching mode enabled and screenshot available
                if opts.use_pattern_matching && action.screenshot_ref.is_some() {
                    (
                        "vision_click_template",
                        vec![
                            ("confidence", json!(opts.template_confidence)),
                            ("click_type", json!(button_str)),
                            ("fallback_x", json!(x)),
                            ("fallback_y", json!(y)),
                        ],
                        false,
                    )
                } else {
                    let mut pins = vec![
                        ("x", json!(x)),
                        ("y", json!(y)),
                        ("button", json!(button_str)),
                    ];

                    // Add natural movement for bot detection evasion
                    if opts.bot_detection_evasion {
                        let mut rng = flow_like_types::rand::rng();
                        pins.push(("natural_move", json!(true)));
                        pins.push(("move_duration_ms", json!(rng.random_range(150..350))));
                    }

                    // Enable template matching when screenshot is available
                    // (template FlowPath is wired automatically via template_path_node_id)
                    if action.screenshot_ref.is_some() {
                        pins.push(("use_template_matching", json!(true)));
                    }

                    // In browser context: disable fingerprint (template preferred)
                    if in_browser && action.screenshot_ref.is_some() {
                        pins.push(("use_fingerprint", json!(false)));
                    }

                    ("computer_mouse_click", pins, false)
                }
            }
            ActionType::DoubleClick { button: _ } => {
                let (x, y) = action.coordinates.unwrap_or((0, 0));
                let mut pins = vec![("x", json!(x)), ("y", json!(y))];

                if opts.bot_detection_evasion {
                    let mut rng = flow_like_types::rand::rng();
                    pins.push(("natural_move", json!(true)));
                    pins.push(("move_duration_ms", json!(rng.random_range(150..350))));
                }

                if action.screenshot_ref.is_some() {
                    pins.push(("use_template_matching", json!(true)));
                }

                if in_browser && action.screenshot_ref.is_some() {
                    pins.push(("use_fingerprint", json!(false)));
                }

                ("computer_mouse_double_click", pins, false)
            }
            ActionType::Drag { start, end } => (
                "computer_mouse_drag",
                vec![
                    ("start_x", json!(start.0)),
                    ("start_y", json!(start.1)),
                    ("end_x", json!(end.0)),
                    ("end_y", json!(end.1)),
                ],
                false,
            ),
            ActionType::Scroll { direction, amount } => {
                // Skip scroll events with 0 amount
                if *amount == 0 {
                    continue;
                }

                // rdev on macOS reports line-level deltas (typically 1-5 per event).
                // After consolidation the accumulated amount is already in scroll-line units.
                // Pass through directly — enigo.scroll(1) sends one line tick.
                let lines = (*amount).max(1).min(100);
                let (dx, dy) = match direction {
                    ScrollDirection::Down => (0, -lines),
                    ScrollDirection::Up => (0, lines),
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
                let modifier_str = modifiers
                    .iter()
                    .map(|m| match m {
                        KeyModifier::Shift => "shift",
                        KeyModifier::Control => "ctrl",
                        KeyModifier::Alt => "alt",
                        KeyModifier::Meta => "meta",
                    })
                    .collect::<Vec<_>>()
                    .join(",");
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
                window_title: _,
                process,
            } => {
                // Track current process for browser detection
                current_process = Some(process.clone());
                (
                    "computer_focus_window",
                    // Use process name (app name) for more reliable matching
                    // Window titles change with tab/page, but app names stay stable
                    vec![("window_title", json!(process))],
                    false,
                )
            }
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
            Err(_) => {
                tracing::warn!("Node {} not found, skipping action", node_name);
                continue;
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
            if let Some((_, pin)) = node.pins.iter_mut().find(|(_, p)| p.name == *pin_name)
                && let Ok(bytes) = to_vec(value)
            {
                pin.default_value = Some(bytes);
            }
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

        // Connect template path to vision_click_template if pattern matching
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

pub fn action_to_description(action: &RecordedAction) -> String {
    match &action.action_type {
        ActionType::Click { button, modifiers } => {
            let coords = action
                .coordinates
                .map(|(x, y)| format!(" at ({}, {})", x, y))
                .unwrap_or_default();
            let mods = if modifiers.is_empty() {
                String::new()
            } else {
                format!(" with {:?}", modifiers)
            };
            format!("{:?} click{}{}", button, coords, mods)
        }
        ActionType::DoubleClick { button } => {
            let coords = action
                .coordinates
                .map(|(x, y)| format!(" at ({}, {})", x, y))
                .unwrap_or_default();
            format!("{:?} double-click{}", button, coords)
        }
        ActionType::Drag { start, end } => {
            format!(
                "Drag from ({}, {}) to ({}, {})",
                start.0, start.1, end.0, end.1
            )
        }
        ActionType::Scroll { direction, amount } => {
            format!("Scroll {:?} by {}", direction, amount)
        }
        ActionType::KeyType { text } => {
            let preview = if text.len() > 20 {
                format!("{}...", &text[..20])
            } else {
                text.clone()
            };
            format!("Type \"{}\"", preview)
        }
        ActionType::KeyPress { key, modifiers } => {
            if modifiers.is_empty() {
                format!("Press {}", key)
            } else {
                format!("Press {:?}+{}", modifiers, key)
            }
        }
        ActionType::AppLaunch { app_name, .. } => {
            format!("Launch {}", app_name)
        }
        ActionType::WindowFocus { window_title, .. } => {
            format!("Focus window \"{}\"", window_title)
        }
        ActionType::Copy { clipboard_content } => {
            let preview = clipboard_content
                .as_ref()
                .map(|s| {
                    if s.len() > 20 {
                        format!("\"{}...\"", &s[..20])
                    } else {
                        format!("\"{}\"", s)
                    }
                })
                .unwrap_or_else(|| "(empty)".to_string());
            format!("Copy {}", preview)
        }
        ActionType::Paste { clipboard_content } => {
            let preview = clipboard_content
                .as_ref()
                .map(|s| {
                    if s.len() > 20 {
                        format!("\"{}...\"", &s[..20])
                    } else {
                        format!("\"{}\"", s)
                    }
                })
                .unwrap_or_else(|| "(empty)".to_string());
            format!("Paste {}", preview)
        }
    }
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
