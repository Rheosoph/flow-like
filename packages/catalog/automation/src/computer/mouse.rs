use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json, rand};

#[crate::register_node]
#[derive(Default)]
pub struct ComputerMouseMoveNode {}

impl ComputerMouseMoveNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerMouseMoveNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_mouse_move",
            "Mouse Move",
            "Moves the mouse cursor to the specified screen coordinates",
            "Automation/Computer/Mouse",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "x",
            "X",
            "X coordinate (horizontal position)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "y",
            "Y",
            "Y coordinate (vertical position)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Coordinate, Mouse};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;

        {
            let mut enigo = session.create_enigo()?;
            enigo
                .move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

/// Generates a point on a cubic Bezier curve
#[cfg(feature = "execute")]
fn bezier_point(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let mt3 = mt2 * mt;

    (
        mt3 * p0.0 + 3.0 * mt2 * t * p1.0 + 3.0 * mt * t2 * p2.0 + t3 * p3.0,
        mt3 * p0.1 + 3.0 * mt2 * t * p1.1 + 3.0 * mt * t2 * p2.1 + t3 * p3.1,
    )
}

/// Easing function for natural acceleration/deceleration
#[cfg(feature = "execute")]
fn ease_in_out_quad(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerNaturalMouseMoveNode {}

impl ComputerNaturalMouseMoveNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerNaturalMouseMoveNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_natural_mouse_move",
            "Natural Mouse Move",
            "Moves the mouse cursor naturally using curved paths with variable speed to avoid bot detection",
            "Automation/Computer/Mouse",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "Target X coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Target Y coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "duration_ms",
            "Duration (ms)",
            "Approximate duration of the movement in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(300)));

        node.add_input_pin(
            "curve_intensity",
            "Curve Intensity",
            "How curved the path is (0.0 = straight, 1.0 = very curved)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.3)));

        node.add_input_pin(
            "overshoot",
            "Overshoot",
            "Whether to slightly overshoot and correct (more human-like)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::Mouse;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let target_x: i64 = context.evaluate_pin("x").await?;
        let target_y: i64 = context.evaluate_pin("y").await?;
        let duration_ms: i64 = context.evaluate_pin("duration_ms").await.unwrap_or(300);
        let curve_intensity: f64 = context.evaluate_pin("curve_intensity").await.unwrap_or(0.3);
        let overshoot: bool = context.evaluate_pin("overshoot").await.unwrap_or(false);

        let mut enigo = session.create_enigo()?;
        let start = enigo.location().unwrap_or((0, 0));
        let end = (target_x as i32, target_y as i32);
        let dur = duration_ms.max(0) as u64;

        flow_like_types::tokio::task::spawn_blocking(move || {
            perform_natural_move(&mut enigo, start, end, dur, curve_intensity, overshoot)
        })
        .await
        .map_err(|e| flow_like_types::anyhow!("Natural mouse move task failed: {}", e))??;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}
/// Which strategy resolved the click target
#[cfg(feature = "execute")]
#[derive(Debug)]
enum ResolvedVia {
    Template,
    Fingerprint,
    FallbackCoordinates,
}

/// Outcome of the click-target resolution hierarchy:
/// template → fingerprint bounding-box → recorded (x, y).
#[cfg(feature = "execute")]
struct ResolvedTarget {
    x: i32,
    y: i32,
    via: ResolvedVia,
}

/// Result of a template matching attempt with full diagnostic info.
#[cfg(feature = "execute")]
use crate::types::screen_match::TemplateMatchResult;

/// Try template matching, returning detailed results for diagnostics.
///
/// Captures the screen via `xcap` (bypassing rustautogui's broken macOS
/// screen capture) and runs NCC directly via rustautogui's `dev` API.
#[cfg(feature = "execute")]
fn try_template_match(template_bytes: &[u8], min_confidence: f32) -> Option<TemplateMatchResult> {
    crate::types::screen_match::try_template_match(template_bytes, min_confidence)
}

/// Resolve the click target using the hierarchy:
/// 1. Template matching (if `use_template_matching` is true and template provided)
/// 2. Fingerprint bounding-box center (if fingerprint provided)
/// 3. Recorded (x, y) coordinates (always available)
///
/// Emits log messages for every step so the user can diagnose failures.
#[cfg(feature = "execute")]
async fn resolve_click_target(
    context: &mut ExecutionContext,
    _session: &AutomationSession,
    x: i64,
    y: i64,
    use_template: bool,
    template_bytes: Option<Vec<u8>>,
    confidence: f64,
    use_fingerprint: bool,
    fingerprint: Option<&crate::types::fingerprints::ElementFingerprint>,
) -> flow_like_types::Result<ResolvedTarget> {
    // ── 1. Template matching ──────────────────────────────────────────
    if use_template {
        if let Some(bytes) = &template_bytes {
            context.log_message(
                &format!(
                    "Template loaded: {} bytes, confidence threshold: {}",
                    bytes.len(),
                    confidence
                ),
                flow_like::flow::execution::LogLevel::Debug,
            );

            match try_template_match(bytes, confidence as f32) {
                Some(result) => {
                    context.log_message(
                        &format!(
                            "Template {}x{}, screen {}x{} (physical), best match: {:?}",
                            result.template_dims.0,
                            result.template_dims.1,
                            result.screen_dims.0,
                            result.screen_dims.1,
                            result
                                .best_match
                                .map(|(mx, my, c)| format!("({},{}) conf={:.4}", mx, my, c))
                                .unwrap_or_else(|| "NONE (zero correlation)".to_string()),
                        ),
                        flow_like::flow::execution::LogLevel::Debug,
                    );

                    if let Some((mx, my, conf)) = result.best_match {
                        if conf >= confidence as f32 {
                            // Convert physical (Retina) coordinates to logical mouse coordinates
                            let (lx, ly) = crate::types::screen_match::physical_to_logical(mx, my);
                            let dist = (((lx as f64 - x as f64).powi(2)
                                + (ly as f64 - y as f64).powi(2))
                            .sqrt()) as i64;
                            if dist > 500 {
                                context.log_message(
                                    &format!(
                                        "Template found at ({}, {}) is {}px from recorded ({}, {}) – large drift",
                                        lx, ly, dist, x, y
                                    ),
                                    flow_like::flow::execution::LogLevel::Warn,
                                );
                            }
                            context.log_message(
                                &format!(
                                    "Template matched at ({}, {}) [logical] confidence {:.4}",
                                    lx, ly, conf
                                ),
                                flow_like::flow::execution::LogLevel::Debug,
                            );
                            return Ok(ResolvedTarget {
                                x: lx,
                                y: ly,
                                via: ResolvedVia::Template,
                            });
                        }
                        context.log_message(
                            &format!(
                                "Template best match at ({},{}) conf={:.4} < threshold {} — not accepted. Debug images saved.",
                                mx, my, conf, confidence
                            ),
                            flow_like::flow::execution::LogLevel::Warn,
                        );
                    } else {
                        context.log_message(
                            &format!(
                                "Template not found on screen (zero correlation). Template {}x{}, screen {}x{}. Debug images saved.",
                                result.template_dims.0, result.template_dims.1,
                                result.screen_dims.0, result.screen_dims.1,
                            ),
                            flow_like::flow::execution::LogLevel::Warn,
                        );
                    }
                }
                None => {
                    context.log_message(
                        "Template grayscale conversion failed — could not decode template image",
                        flow_like::flow::execution::LogLevel::Warn,
                    );
                }
            }
        } else {
            context.log_message(
                "Template matching enabled but no template image provided (FlowPath evaluation failed or empty)",
                flow_like::flow::execution::LogLevel::Warn,
            );
        }
    }

    // ── 2. Fingerprint bounding box ───────────────────────────────────
    if use_fingerprint {
        if let Some(fp) = fingerprint {
            context.log_message(
                &format!(
                    "Fingerprint present: role={:?}, name={:?}, text={:?}, bbox={:?}",
                    fp.role, fp.name, fp.text, fp.bounding_box
                ),
                flow_like::flow::execution::LogLevel::Debug,
            );

            if let Some(ref bbox) = fp.bounding_box {
                let cx = ((bbox.x1 + bbox.x2) / 2.0) as i32;
                let cy = ((bbox.y1 + bbox.y2) / 2.0) as i32;
                let dist = (((cx as f64 - x as f64).powi(2) + (cy as f64 - y as f64).powi(2))
                    .sqrt()) as i64;
                if dist > 500 {
                    context.log_message(
                        &format!(
                            "Fingerprint bbox center ({}, {}) is {}px from recorded ({}, {}) – large drift",
                            cx, cy, dist, x, y
                        ),
                        flow_like::flow::execution::LogLevel::Warn,
                    );
                }
                context.log_message(
                    &format!("Using fingerprint bounding-box center ({}, {})", cx, cy),
                    flow_like::flow::execution::LogLevel::Debug,
                );
                return Ok(ResolvedTarget {
                    x: cx,
                    y: cy,
                    via: ResolvedVia::Fingerprint,
                });
            }
            context.log_message(
                "Fingerprint has no bounding box, cannot use for positioning",
                flow_like::flow::execution::LogLevel::Warn,
            );
        }
    } else if fingerprint.is_some() {
        context.log_message(
            "Fingerprint available but use_fingerprint is disabled, skipping",
            flow_like::flow::execution::LogLevel::Debug,
        );
    }

    // ── 3. Fallback: recorded coordinates ─────────────────────────────
    context.log_message(
        &format!("Using recorded coordinates ({}, {})", x, y),
        flow_like::flow::execution::LogLevel::Debug,
    );
    Ok(ResolvedTarget {
        x: x as i32,
        y: y as i32,
        via: ResolvedVia::FallbackCoordinates,
    })
}

/// Performs natural mouse movement using Bezier curves
#[cfg(feature = "execute")]
fn perform_natural_move(
    enigo: &mut enigo::Enigo,
    start: (i32, i32),
    end: (i32, i32),
    duration_ms: u64,
    curve_intensity: f64,
    overshoot: bool,
) -> flow_like_types::Result<()> {
    use enigo::{Coordinate, Mouse};
    use rand::Rng;

    let mut rng = rand::rng();

    let start_f = (start.0 as f64, start.1 as f64);
    let end_f = (end.0 as f64, end.1 as f64);

    let dx = end_f.0 - start_f.0;
    let dy = end_f.1 - start_f.1;
    let distance = (dx * dx + dy * dy).sqrt();

    // Skip natural movement for very short distances
    if distance < 10.0 {
        enigo
            .move_mouse(end.0, end.1, Coordinate::Abs)
            .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
        return Ok(());
    }

    let steps = ((distance / 10.0) as i32).max(10).min(100);
    let step_delay_ms = (duration_ms as f64 / steps as f64) as u64;

    // Generate random control points for bezier curve
    let curve_offset = distance * curve_intensity;
    let perpendicular = if dx.abs() > 0.001 {
        (-dy / dx, 1.0)
    } else {
        (1.0, 0.0)
    };
    let perp_len = (perpendicular.0 * perpendicular.0 + perpendicular.1 * perpendicular.1).sqrt();
    let perp_norm = (perpendicular.0 / perp_len, perpendicular.1 / perp_len);

    let offset1 = rng.random_range(-curve_offset..curve_offset);
    let offset2 = rng.random_range(-curve_offset..curve_offset);

    let ctrl1 = (
        start_f.0 + dx * 0.3 + perp_norm.0 * offset1,
        start_f.1 + dy * 0.3 + perp_norm.1 * offset1,
    );
    let ctrl2 = (
        start_f.0 + dx * 0.7 + perp_norm.0 * offset2,
        start_f.1 + dy * 0.7 + perp_norm.1 * offset2,
    );

    // Overshoot if enabled
    let overshoot_amount = if overshoot && distance > 50.0 {
        let overshoot_dist = rng.random_range(5.0..15.0);
        let angle = dy.atan2(dx);
        (overshoot_dist * angle.cos(), overshoot_dist * angle.sin())
    } else {
        (0.0, 0.0)
    };

    let overshoot_end = (end_f.0 + overshoot_amount.0, end_f.1 + overshoot_amount.1);

    // Move along the bezier curve
    for i in 1..=steps {
        let t = ease_in_out_quad(i as f64 / steps as f64);
        let (x, y) = bezier_point(start_f, ctrl1, ctrl2, overshoot_end, t);

        let jitter_x = rng.random_range(-1.0..1.0);
        let jitter_y = rng.random_range(-1.0..1.0);

        enigo
            .move_mouse(
                (x + jitter_x) as i32,
                (y + jitter_y) as i32,
                Coordinate::Abs,
            )
            .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;

        let delay_variance = rng.random_range(0.8..1.2);
        std::thread::sleep(std::time::Duration::from_millis(
            (step_delay_ms as f64 * delay_variance) as u64,
        ));
    }

    // Correct from overshoot
    if overshoot && (overshoot_amount.0.abs() > 0.1 || overshoot_amount.1.abs() > 0.1) {
        std::thread::sleep(std::time::Duration::from_millis(rng.random_range(30..80)));

        let correction_steps = 5;
        for i in 1..=correction_steps {
            let t = i as f64 / correction_steps as f64;
            let x = overshoot_end.0 + (end_f.0 - overshoot_end.0) * t;
            let y = overshoot_end.1 + (end_f.1 - overshoot_end.1) * t;

            enigo
                .move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;

            std::thread::sleep(std::time::Duration::from_millis(rng.random_range(10..20)));
        }
    }

    // Ensure exact end position
    enigo
        .move_mouse(end.0, end.1, Coordinate::Abs)
        .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;

    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerMouseClickNode {}

impl ComputerMouseClickNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerMouseClickNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_mouse_click",
            "Mouse Click",
            "Clicks the mouse at the specified coordinates",
            "Automation/Computer/Mouse",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "x",
            "X",
            "X coordinate (horizontal position)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "y",
            "Y",
            "Y coordinate (vertical position)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "button",
            "Button",
            "Mouse button to click",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "left".to_string(),
                    "right".to_string(),
                    "middle".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("left")));

        node.add_input_pin(
            "use_template_matching",
            "Use Template Matching",
            "If enabled, use template matching to find the click target from a recorded screenshot",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "template",
            "Template",
            "Template image for template matching",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Minimum confidence threshold for template matching (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "natural_move",
            "Natural Movement",
            "Use curved, human-like mouse movement to avoid bot detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "move_duration_ms",
            "Move Duration (ms)",
            "Duration of natural mouse movement in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(200)));

        node.add_input_pin(
            "use_fingerprint",
            "Use Fingerprint",
            "If enabled, use fingerprint bounding box as fallback before raw coordinates",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Optional element fingerprint for pre-click validation",
            VariableType::Struct,
        )
        .set_schema::<crate::types::fingerprints::ElementFingerprint>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Button, Coordinate, Mouse};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let button_str: String = context.evaluate_pin("button").await?;
        let use_template: bool = context
            .evaluate_pin("use_template_matching")
            .await
            .unwrap_or(false);
        let template_bytes: Option<Vec<u8>> = if use_template {
            if let Ok(tmpl) = context.evaluate_pin::<FlowPath>("template").await {
                tmpl.get(context, false).await.ok()
            } else {
                None
            }
        } else {
            None
        };
        let confidence: f64 = context.evaluate_pin("confidence").await.unwrap_or(0.8);
        let natural_move: bool = context.evaluate_pin("natural_move").await.unwrap_or(false);
        let move_duration_ms: i64 = context
            .evaluate_pin("move_duration_ms")
            .await
            .unwrap_or(200);
        let use_fingerprint: bool = context
            .evaluate_pin("use_fingerprint")
            .await
            .unwrap_or(true);
        let fingerprint: Option<crate::types::fingerprints::ElementFingerprint> =
            context.evaluate_pin("fingerprint").await.ok();

        let target = resolve_click_target(
            context,
            &session,
            x,
            y,
            use_template,
            template_bytes,
            confidence,
            use_fingerprint,
            fingerprint.as_ref(),
        )
        .await?;

        context.log_message(
            &format!(
                "Click target resolved to ({}, {}) via {:?}",
                target.x, target.y, target.via
            ),
            flow_like::flow::execution::LogLevel::Debug,
        );

        let button = match button_str.as_str() {
            "right" => Button::Right,
            "middle" => Button::Middle,
            _ => Button::Left,
        };

        {
            let mut enigo = session.create_enigo()?;

            if natural_move {
                use rand::Rng;
                let start = enigo.location().unwrap_or((0, 0));
                let overshoot = rand::rng().random_bool(0.3);
                perform_natural_move(
                    &mut enigo,
                    start,
                    (target.x, target.y),
                    move_duration_ms as u64,
                    0.3,
                    overshoot,
                )?;
            } else {
                enigo
                    .move_mouse(target.x, target.y, Coordinate::Abs)
                    .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            }

            std::thread::sleep(std::time::Duration::from_millis(session.click_delay_ms));

            enigo
                .button(button, enigo::Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to click mouse: {}", e))?;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerMouseDoubleClickNode {}

impl ComputerMouseDoubleClickNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerMouseDoubleClickNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_mouse_double_click",
            "Mouse Double Click",
            "Double-clicks the mouse at the specified coordinates",
            "Automation/Computer/Mouse",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "X coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Y coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "use_template_matching",
            "Use Template Matching",
            "If enabled, use template matching to find the click target from a recorded screenshot",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "template",
            "Template",
            "Template image for template matching",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Minimum confidence threshold for template matching (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "natural_move",
            "Natural Movement",
            "Use curved, human-like mouse movement to avoid bot detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "move_duration_ms",
            "Move Duration (ms)",
            "Duration of natural mouse movement in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(200)));

        node.add_input_pin(
            "use_fingerprint",
            "Use Fingerprint",
            "If enabled, use fingerprint bounding box as fallback before raw coordinates",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Optional element fingerprint for pre-click validation",
            VariableType::Struct,
        )
        .set_schema::<crate::types::fingerprints::ElementFingerprint>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Button, Coordinate, Mouse};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let use_template: bool = context
            .evaluate_pin("use_template_matching")
            .await
            .unwrap_or(false);
        let template_bytes: Option<Vec<u8>> = if use_template {
            if let Ok(tmpl) = context.evaluate_pin::<FlowPath>("template").await {
                tmpl.get(context, false).await.ok()
            } else {
                None
            }
        } else {
            None
        };
        let confidence: f64 = context.evaluate_pin("confidence").await.unwrap_or(0.8);
        let natural_move: bool = context.evaluate_pin("natural_move").await.unwrap_or(false);
        let move_duration_ms: i64 = context
            .evaluate_pin("move_duration_ms")
            .await
            .unwrap_or(200);
        let use_fingerprint: bool = context
            .evaluate_pin("use_fingerprint")
            .await
            .unwrap_or(true);
        let fingerprint: Option<crate::types::fingerprints::ElementFingerprint> =
            context.evaluate_pin("fingerprint").await.ok();

        let target = resolve_click_target(
            context,
            &session,
            x,
            y,
            use_template,
            template_bytes,
            confidence,
            use_fingerprint,
            fingerprint.as_ref(),
        )
        .await?;

        context.log_message(
            &format!(
                "DoubleClick target resolved to ({}, {}) via {:?}",
                target.x, target.y, target.via
            ),
            flow_like::flow::execution::LogLevel::Debug,
        );

        {
            let mut enigo = session.create_enigo()?;

            if natural_move {
                use rand::Rng;
                let start = enigo.location().unwrap_or((0, 0));
                let overshoot = rand::rng().random_bool(0.3);
                perform_natural_move(
                    &mut enigo,
                    start,
                    (target.x, target.y),
                    move_duration_ms as u64,
                    0.3,
                    overshoot,
                )?;
            } else {
                enigo
                    .move_mouse(target.x, target.y, Coordinate::Abs)
                    .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            }

            std::thread::sleep(std::time::Duration::from_millis(session.click_delay_ms));

            enigo
                .button(Button::Left, enigo::Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to click mouse: {}", e))?;
        }

        flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        {
            let mut enigo = session.create_enigo()?;
            enigo
                .button(Button::Left, enigo::Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to double-click mouse: {}", e))?;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerMouseDragNode {}

impl ComputerMouseDragNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerMouseDragNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_mouse_drag",
            "Mouse Drag",
            "Drags the mouse from one position to another",
            "Automation/Computer/Mouse",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "from_x",
            "From X",
            "Starting X coordinate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "from_y",
            "From Y",
            "Starting Y coordinate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin("to_x", "To X", "Ending X coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("to_y", "To Y", "Ending Y coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "button",
            "Button",
            "Mouse button to use for dragging",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec!["left".to_string(), "right".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("left")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Button, Coordinate, Mouse};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let from_x: i64 = context.evaluate_pin("from_x").await?;
        let from_y: i64 = context.evaluate_pin("from_y").await?;
        let to_x: i64 = context.evaluate_pin("to_x").await?;
        let to_y: i64 = context.evaluate_pin("to_y").await?;
        let button_str: String = context.evaluate_pin("button").await?;

        let button = match button_str.as_str() {
            "right" => Button::Right,
            _ => Button::Left,
        };

        {
            let mut enigo = session.create_enigo()?;
            enigo
                .move_mouse(from_x as i32, from_y as i32, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            enigo
                .button(button, enigo::Direction::Press)
                .map_err(|e| flow_like_types::anyhow!("Failed to press mouse: {}", e))?;
            enigo
                .move_mouse(to_x as i32, to_y as i32, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            enigo
                .button(button, enigo::Direction::Release)
                .map_err(|e| flow_like_types::anyhow!("Failed to release mouse: {}", e))?;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerScrollNode {}

impl ComputerScrollNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerScrollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_scroll",
            "Scroll",
            "Scrolls the mouse wheel",
            "Automation/Computer/Mouse",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "dx",
            "Delta X",
            "Horizontal scroll amount (positive = right)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "dy",
            "Delta Y",
            "Vertical scroll amount (positive = down)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Axis, Mouse};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let dx: i64 = context.evaluate_pin("dx").await?;
        let dy: i64 = context.evaluate_pin("dy").await?;

        // Send individual scroll ticks with small delays to ensure
        // browsers and other apps process each event correctly.
        // A single large scroll event is often ignored or misinterpreted.
        let tick_delay = std::time::Duration::from_millis(15);
        let session_clone = session.clone();

        flow_like_types::tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            let mut enigo = session_clone.create_enigo()?;
            let dy_dir: i32 = if dy > 0 { 1 } else { -1 };
            let dx_dir: i32 = if dx > 0 { 1 } else { -1 };

            for _ in 0..dy.unsigned_abs() {
                enigo
                    .scroll(dy_dir, Axis::Vertical)
                    .map_err(|e| flow_like_types::anyhow!("Failed to scroll vertically: {}", e))?;
                std::thread::sleep(tick_delay);
            }
            for _ in 0..dx.unsigned_abs() {
                enigo.scroll(dx_dir, Axis::Horizontal).map_err(|e| {
                    flow_like_types::anyhow!("Failed to scroll horizontally: {}", e)
                })?;
                std::thread::sleep(tick_delay);
            }
            Ok(())
        })
        .await
        .map_err(|e| flow_like_types::anyhow!("Scroll task failed: {}", e))??;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}
