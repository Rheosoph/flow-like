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
        node.set_version(1);
        node.set_flowscript_name("computer", "mouseMove");
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
        session.ensure_active(context).await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;

        let mut enigo = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            check_cancellation(cancellation.as_ref())?;
            enigo
                .move_mouse(i32::try_from(x)?, i32::try_from(y)?, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;

            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "naturalMouseMove");
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
        session.ensure_active(context).await?;
        let target_x: i64 = context.evaluate_pin("x").await?;
        let target_y: i64 = context.evaluate_pin("y").await?;
        let duration_ms: i64 = context.evaluate_pin("duration_ms").await.unwrap_or(300);
        let curve_intensity: f64 = context.evaluate_pin("curve_intensity").await.unwrap_or(0.3);
        let overshoot: bool = context.evaluate_pin("overshoot").await.unwrap_or(false);

        let mut enigo = session.create_enigo(context).await?;
        let start = enigo.location().map_err(|error| {
            flow_like_types::anyhow!(
                "Cannot read cursor position for natural movement: {}",
                error
            )
        })?;
        let end = (i32::try_from(target_x)?, i32::try_from(target_y)?);
        let dur = duration_ms.clamp(0, 60_000) as u64;
        let cancellation = context.get_cancellation_token();

        flow_like_types::tokio::task::spawn_blocking(move || {
            perform_natural_move(
                &mut enigo,
                start,
                end,
                dur,
                curve_intensity,
                overshoot,
                cancellation.as_ref(),
            )
        })
        .await
        .map_err(|e| flow_like_types::anyhow!("Natural mouse move task failed: {}", e))??;

        session.apply_delay(context).await?;
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

/// A target resolved from current pixels, accessibility data, or explicit coordinates.
#[cfg(feature = "execute")]
struct ResolvedTarget {
    x: i32,
    y: i32,
    via: ResolvedVia,
}

/// Resolve visual and accessibility targets against the current desktop before input.
#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
async fn resolve_click_target(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    x: i64,
    y: i64,
    use_template: bool,
    template_bytes: Option<Vec<u8>>,
    confidence: f64,
    use_fingerprint: bool,
    fingerprint: Option<&crate::types::fingerprints::ElementFingerprint>,
) -> flow_like_types::Result<ResolvedTarget> {
    context.check_cancelled()?;
    session.ensure_active(context).await?;
    if use_template {
        let bytes = template_bytes.ok_or_else(|| {
            flow_like_types::anyhow!("Template matching requires a template image")
        })?;
        let matches =
            crate::types::screen_match::match_desktop_async(bytes, confidence, -2).await?;
        let &(x,y,_)=matches.first().ok_or_else(||flow_like_types::anyhow!("The visual target could not be verified. Capture a new template or explicitly use coordinate mode."))?;
        if matches
            .get(1)
            .is_some_and(|other| (matches[0].2 - other.2).abs() < 0.01)
        {
            return Err(flow_like_types::anyhow!(
                "The visual target is ambiguous across the desktop. Use a more specific template."
            ));
        }
        return Ok(ResolvedTarget {
            x,
            y,
            via: ResolvedVia::Template,
        });
    }
    if use_fingerprint {
        let fingerprint = fingerprint.ok_or_else(|| {
            flow_like_types::anyhow!("Fingerprint matching requires a fingerprint")
        })?;
        let role = fingerprint.role.as_deref().unwrap_or_default();
        let name = fingerprint
            .name
            .as_deref()
            .or(fingerprint.text.as_deref())
            .unwrap_or_default();
        if role.is_empty() && name.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Fingerprint needs an accessible role or name"
            ));
        }
        let title = fingerprint
            .attributes
            .get("window_title")
            .map(String::as_str)
            .unwrap_or_default();
        let tree = super::accessibility::load_tree(title, 32).await?;
        let mut matches = Vec::new();
        super::accessibility::find_elements(&tree, role, name, &mut matches);
        if matches.len() != 1 {
            return Err(flow_like_types::anyhow!(
                "Fingerprint matched {} current elements; locate a unique element before clicking",
                matches.len()
            ));
        }
        let bounds = matches[0]
            .bounds
            .as_ref()
            .filter(|b| b.width > 0 && b.height > 0)
            .ok_or_else(|| flow_like_types::anyhow!("Accessible target has no visible bounds"))?;
        return Ok(ResolvedTarget {
            x: bounds
                .x
                .checked_add(bounds.width / 2)
                .ok_or_else(|| flow_like_types::anyhow!("Element coordinate overflow"))?,
            y: bounds
                .y
                .checked_add(bounds.height / 2)
                .ok_or_else(|| flow_like_types::anyhow!("Element coordinate overflow"))?,
            via: ResolvedVia::Fingerprint,
        });
    }
    Ok(ResolvedTarget {
        x: i32::try_from(x)?,
        y: i32::try_from(y)?,
        via: ResolvedVia::FallbackCoordinates,
    })
}

/// Performs natural mouse movement using Bezier curves
#[cfg(feature = "execute")]
fn perform_natural_move(
    enigo: &mut super::native::input::DesktopInput,
    start: (i32, i32),
    end: (i32, i32),
    duration_ms: u64,
    curve_intensity: f64,
    overshoot: bool,
    cancellation: Option<&flow_like_types::tokio_util::sync::CancellationToken>,
) -> flow_like_types::Result<()> {
    use enigo::{Coordinate, Mouse};
    use rand::Rng;

    if !curve_intensity.is_finite() || !(0.0..=1.0).contains(&curve_intensity) {
        return Err(flow_like_types::anyhow!(
            "Curve intensity must be between 0 and 1"
        ));
    }
    check_cancellation(cancellation)?;
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

    let steps = ((distance / 10.0) as i32).clamp(10, 100);
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

    let offset1 = if curve_offset > 0.0 {
        rng.random_range(-curve_offset..curve_offset)
    } else {
        0.0
    };
    let offset2 = if curve_offset > 0.0 {
        rng.random_range(-curve_offset..curve_offset)
    } else {
        0.0
    };

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
        if cancellation.is_some_and(|token| token.is_cancelled()) {
            return Err(flow_like_types::anyhow!("Automation cancelled"));
        }
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
        interruptible_sleep((step_delay_ms as f64 * delay_variance) as u64, cancellation)?;
    }

    // Correct from overshoot
    if overshoot && (overshoot_amount.0.abs() > 0.1 || overshoot_amount.1.abs() > 0.1) {
        interruptible_sleep(rng.random_range(30..80), cancellation)?;

        let correction_steps = 5;
        for i in 1..=correction_steps {
            let t = i as f64 / correction_steps as f64;
            let x = overshoot_end.0 + (end_f.0 - overshoot_end.0) * t;
            let y = overshoot_end.1 + (end_f.1 - overshoot_end.1) * t;

            enigo
                .move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;

            interruptible_sleep(rng.random_range(10..20), cancellation)?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "mouseClick");
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
            "Resolve a unique accessible element from the current desktop before clicking",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Optional element fingerprint for pre-click validation",
            VariableType::Struct,
        )
        .set_schema::<crate::types::fingerprints::ElementFingerprint>();

        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Comma-separated ctrl, shift, alt, meta",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
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
        session.ensure_active(context).await?;
        let modifiers: String = context.evaluate_pin("modifiers").await.unwrap_or_default();
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
            .unwrap_or(false);
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

        if session.debug_mode {
            context.log_message(
                &format!(
                    "Click target resolved to ({}, {}) via {:?}",
                    target.x, target.y, target.via
                ),
                flow_like::flow::execution::LogLevel::Debug,
            );
        }

        let button = parse_button(&button_str)?;

        let mut enigo = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        let click_delay_ms = session.click_delay_ms.min(5000);
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            check_cancellation(cancellation.as_ref())?;
            enigo.modifiers(&modifiers)?;

            if natural_move && !super::native::input::wayland() {
                use rand::Rng;
                let start = enigo.location().map_err(|error| {
                    flow_like_types::anyhow!(
                        "Cannot read cursor position for natural movement: {}",
                        error
                    )
                })?;
                let overshoot = rand::rng().random_bool(0.3);
                perform_natural_move(
                    &mut enigo,
                    start,
                    (target.x, target.y),
                    move_duration_ms.clamp(0, 60_000) as u64,
                    0.3,
                    overshoot,
                    cancellation.as_ref(),
                )?;
            } else {
                enigo
                    .move_mouse(target.x, target.y, Coordinate::Abs)
                    .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            }

            interruptible_sleep(click_delay_ms, cancellation.as_ref())?;
            check_cancellation(cancellation.as_ref())?;

            enigo
                .button(button, enigo::Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to click mouse: {}", e))?;

            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "mouseDoubleClick");
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
            "Resolve a unique accessible element from the current desktop before clicking",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Optional element fingerprint for pre-click validation",
            VariableType::Struct,
        )
        .set_schema::<crate::types::fingerprints::ElementFingerprint>();

        node.add_input_pin("button", "Button", "Mouse button", VariableType::String)
            .set_default_value(Some(json!("left")))
            .set_options(
                flow_like::flow::pin::PinOptions::new()
                    .set_valid_values(vec!["left".into(), "right".into(), "middle".into()])
                    .build(),
            );
        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Comma-separated ctrl, shift, alt, meta",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
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
        session.ensure_active(context).await?;
        let modifiers: String = context.evaluate_pin("modifiers").await.unwrap_or_default();
        let button: String = context
            .evaluate_pin("button")
            .await
            .unwrap_or_else(|_| "left".into());
        let button = parse_button(&button)?;
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
            .unwrap_or(false);
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

        if session.debug_mode {
            context.log_message(
                &format!(
                    "DoubleClick target resolved to ({}, {}) via {:?}",
                    target.x, target.y, target.via
                ),
                flow_like::flow::execution::LogLevel::Debug,
            );
        }

        let mut enigo = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        let click_delay_ms = session.click_delay_ms.min(5000);
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            check_cancellation(cancellation.as_ref())?;
            enigo.modifiers(&modifiers)?;

            if natural_move && !super::native::input::wayland() {
                use rand::Rng;
                let start = enigo.location().map_err(|error| {
                    flow_like_types::anyhow!(
                        "Cannot read cursor position for natural movement: {}",
                        error
                    )
                })?;
                let overshoot = rand::rng().random_bool(0.3);
                perform_natural_move(
                    &mut enigo,
                    start,
                    (target.x, target.y),
                    move_duration_ms.clamp(0, 60_000) as u64,
                    0.3,
                    overshoot,
                    cancellation.as_ref(),
                )?;
            } else {
                enigo
                    .move_mouse(target.x, target.y, Coordinate::Abs)
                    .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            }

            interruptible_sleep(click_delay_ms, cancellation.as_ref())?;
            check_cancellation(cancellation.as_ref())?;

            enigo
                .button(button, enigo::Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to click mouse: {}", e))?;
            interruptible_sleep(80, cancellation.as_ref())?;
            enigo
                .button(button, enigo::Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to double-click mouse: {}", e))?;

            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "mouseDrag");
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

        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Comma-separated ctrl, shift, alt, meta",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
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
        session.ensure_active(context).await?;
        let modifiers: String = context.evaluate_pin("modifiers").await.unwrap_or_default();
        let from_x: i64 = context.evaluate_pin("from_x").await?;
        let from_y: i64 = context.evaluate_pin("from_y").await?;
        let to_x: i64 = context.evaluate_pin("to_x").await?;
        let to_y: i64 = context.evaluate_pin("to_y").await?;
        let button_str: String = context.evaluate_pin("button").await?;

        let button = parse_button(&button_str)?;

        let mut enigo = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            check_cancellation(cancellation.as_ref())?;
            enigo.modifiers(&modifiers)?;
            enigo
                .move_mouse(
                    i32::try_from(from_x)?,
                    i32::try_from(from_y)?,
                    Coordinate::Abs,
                )
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            enigo
                .button(button, enigo::Direction::Press)
                .map_err(|e| flow_like_types::anyhow!("Failed to press mouse: {}", e))?;
            enigo
                .move_mouse(i32::try_from(to_x)?, i32::try_from(to_y)?, Coordinate::Abs)
                .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;
            enigo
                .button(button, enigo::Direction::Release)
                .map_err(|e| flow_like_types::anyhow!("Failed to release mouse: {}", e))?;

            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "scroll");
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
        session.ensure_active(context).await?;
        let dx: i64 = context.evaluate_pin("dx").await?;
        let dy: i64 = context.evaluate_pin("dy").await?;
        if dx.unsigned_abs() > 1000 || dy.unsigned_abs() > 1000 {
            return Err(flow_like_types::anyhow!(
                "Scroll amount must be between -1000 and 1000 ticks"
            ));
        }

        // Send individual scroll ticks with small delays to ensure
        // browsers and other apps process each event correctly.
        // A single large scroll event is often ignored or misinterpreted.
        let tick_delay = std::time::Duration::from_millis(15);
        let mut enigo = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();

        flow_like_types::tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            let dy_dir: i32 = if dy > 0 { 1 } else { -1 };
            let dx_dir: i32 = if dx > 0 { 1 } else { -1 };

            for _ in 0..dy.unsigned_abs() {
                if cancellation
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
                {
                    return Err(flow_like_types::anyhow!("Automation cancelled"));
                }
                enigo
                    .scroll(dy_dir, Axis::Vertical)
                    .map_err(|e| flow_like_types::anyhow!("Failed to scroll vertically: {}", e))?;
                std::thread::sleep(tick_delay);
            }
            for _ in 0..dx.unsigned_abs() {
                if cancellation
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
                {
                    return Err(flow_like_types::anyhow!("Automation cancelled"));
                }
                enigo.scroll(dx_dir, Axis::Horizontal).map_err(|e| {
                    flow_like_types::anyhow!("Failed to scroll horizontally: {}", e)
                })?;
                std::thread::sleep(tick_delay);
            }
            Ok(())
        })
        .await
        .map_err(|e| flow_like_types::anyhow!("Scroll task failed: {}", e))??;

        session.apply_delay(context).await?;
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

#[cfg(feature = "execute")]
fn parse_button(value: &str) -> flow_like_types::Result<enigo::Button> {
    match value.to_lowercase().as_str() {
        "left" => Ok(enigo::Button::Left),
        "right" => Ok(enigo::Button::Right),
        "middle" => Ok(enigo::Button::Middle),
        _ => Err(flow_like_types::anyhow!("Unknown mouse button: {}", value)),
    }
}

#[cfg(feature = "execute")]
pub(crate) fn check_cancellation(
    token: Option<&flow_like_types::tokio_util::sync::CancellationToken>,
) -> flow_like_types::Result<()> {
    if token.is_some_and(|token| token.is_cancelled()) {
        return Err(flow_like_types::anyhow!("Automation cancelled"));
    }
    Ok(())
}
#[cfg(feature = "execute")]
pub(crate) fn interruptible_sleep(
    ms: u64,
    token: Option<&flow_like_types::tokio_util::sync::CancellationToken>,
) -> flow_like_types::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    loop {
        check_cancellation(token)?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
    }
}
