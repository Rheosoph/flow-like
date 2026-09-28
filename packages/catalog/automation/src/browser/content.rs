#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
#[cfg(any(feature = "execute", test))]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};

/// Session-bound browser node with exec pins, filed under `Automation/Browser/<category>`.
pub(crate) fn page_node(id: &str, title: &str, description: &str, category: &str) -> Node {
    let mut node = super::manage::base_node(id, title, description);
    node.category = format!("Automation/Browser/{category}");
    node
}

#[cfg(any(feature = "execute", test))]
fn truncate_chars(text: String, max_chars: usize) -> (String, bool) {
    if max_chars == 0 {
        return (text, false);
    }
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => {
            let mut text = text;
            text.truncate(end);
            (text, true)
        }
        None => (text, false),
    }
}

#[cfg(feature = "execute")]
fn html_to_markdown(html: &str) -> flow_like_types::Result<String> {
    let converter = htmd::HtmlToMarkdownBuilder::new()
        .skip_tags(vec![
            "head", "script", "style", "noscript", "template", "iframe", "svg", "canvas", "video",
            "audio", "object", "embed",
        ])
        .build();
    let markdown = converter
        .convert(html)
        .map_err(|e| flow_like_types::anyhow!("Failed to convert page HTML to Markdown: {e}"))?;
    Ok(markdown.trim().to_owned())
}

#[cfg(feature = "execute")]
const PAGE_CONTENT_SCRIPT: &str = "const root = arguments[0] || document.body || document.documentElement; return { text: root.innerText ?? root.textContent ?? '', html: root.outerHTML ?? '' };";

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetPageTextNode {}

impl BrowserGetPageTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetPageTextNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_get_page_text",
            "Get Page Text",
            "Reads the rendered text of the page, or of one scoped element, and a Markdown version of the same content for prompts and summaries. Hidden elements are excluded from the text; scripts, styles and embedded media are dropped from the Markdown.",
            "Extract",
        );
        node.set_flowscript_name("browser", "getPageText");
        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(6)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "scope",
            "Scope",
            "CSS selector of the element to read; empty reads the whole page body",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        super::selector::add_locator_pin(&mut node);
        node.add_input_pin(
            "max_chars",
            "Max Characters",
            "Maximum characters returned in Text and in Markdown; 0 returns everything",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50000)));
        node.add_output_pin(
            "text",
            "Text",
            "Rendered text (innerText) with the page's line breaks",
            VariableType::String,
        );
        node.add_output_pin(
            "markdown",
            "Markdown",
            "Markdown converted from the same content, keeping headings, links, lists and tables",
            VariableType::String,
        );
        node.add_output_pin(
            "truncated",
            "Truncated",
            "True when Text or Markdown was cut to Max Characters",
            VariableType::Boolean,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let scope: String = context.evaluate_pin("scope").await?;
        let locator = super::selector::evaluate_locator(context, &scope).await?;
        let max_chars: i64 = context.evaluate_pin("max_chars").await?;
        let max_chars = usize::try_from(max_chars).map_err(|_| {
            flow_like_types::anyhow!("Max characters must be nonnegative, got {max_chars}")
        })?;

        let driver = session.get_browser_driver_and_switch(context).await?;
        let root = if locator.value.is_empty() {
            Value::Null
        } else {
            super::selector::find(&driver, &locator)
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!(
                        "Failed to find page text scope '{}': {e}",
                        locator.value
                    )
                })?
                .to_json()?
        };
        let content = driver
            .execute(PAGE_CONTENT_SCRIPT, vec![root])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to read page content: {e}"))?;
        drop(driver);
        let text = content.json()["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let html = content.json()["html"]
            .as_str()
            .unwrap_or_default()
            .to_owned();

        let (text, markdown, truncated) = tokio::task::spawn_blocking(move || {
            let markdown = html_to_markdown(&html)?;
            let (text, text_truncated) = truncate_chars(text, max_chars);
            let (markdown, markdown_truncated) = truncate_chars(markdown, max_chars);
            Ok::<_, flow_like_types::Error>((text, markdown, text_truncated || markdown_truncated))
        })
        .await??;

        context.set_pin_value("text", json!(text)).await?;
        context.set_pin_value("markdown", json!(markdown)).await?;
        context.set_pin_value("truncated", json!(truncated)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

/// Largest rendered side Chrome can rasterize in one capture.
#[cfg(any(feature = "execute", test))]
const MAX_CAPTURE_SIDE: f64 = 16384.0;

/// Converts a viewport-relative CSS region into a `Page.captureScreenshot` clip, which is
/// expressed in document coordinates.
#[cfg(any(feature = "execute", test))]
fn zoom_clip(
    region: [f64; 4],
    scale: f64,
    layout_metrics: &Value,
) -> flow_like_types::Result<Value> {
    let [x, y, width, height] = region;
    if !region.iter().all(|value| value.is_finite()) || x < 0.0 || y < 0.0 {
        return Err(flow_like_types::anyhow!(
            "Zoom region must use finite, nonnegative coordinates, got x={x}, y={y}"
        ));
    }
    if width < 1.0 || height < 1.0 {
        return Err(flow_like_types::anyhow!(
            "Zoom region must be at least 1×1 CSS pixels, got {width}×{height}"
        ));
    }
    if !scale.is_finite() || scale <= 0.0 || scale > 10.0 {
        return Err(flow_like_types::anyhow!(
            "Zoom scale must be greater than 0 and at most 10, got {scale}"
        ));
    }
    if width * scale > MAX_CAPTURE_SIDE || height * scale > MAX_CAPTURE_SIDE {
        return Err(flow_like_types::anyhow!(
            "Zoomed region {width}×{height} at scale {scale} exceeds {MAX_CAPTURE_SIDE} pixels per side"
        ));
    }
    let viewport = &layout_metrics["cssVisualViewport"];
    let page_x = viewport["pageX"].as_f64().unwrap_or_default();
    let page_y = viewport["pageY"].as_f64().unwrap_or_default();
    Ok(json!({
        "x": x + page_x,
        "y": y + page_y,
        "width": width,
        "height": height,
        "scale": scale,
    }))
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserZoomScreenshotNode {}

impl BrowserZoomScreenshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserZoomScreenshotNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_zoom_screenshot",
            "Zoom Screenshot",
            "Captures a region of the visible page at a higher device scale, so small text and icons stay legible for vision models. Coordinates are viewport CSS pixels, the same space as element bounding boxes and Click At Point. Chrome and Edge only.",
            "Capture",
        );
        node.set_flowscript_name("browser", "zoomScreenshot");
        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        for (name, title, description, default) in [
            (
                "x",
                "X",
                "Left edge of the region in viewport CSS pixels",
                0.0,
            ),
            (
                "y",
                "Y",
                "Top edge of the region in viewport CSS pixels",
                0.0,
            ),
            ("width", "Width", "Region width in CSS pixels", 400.0),
            ("height", "Height", "Region height in CSS pixels", 300.0),
        ] {
            node.add_input_pin(name, title, description, VariableType::Float)
                .set_default_value(Some(json!(default)));
        }
        node.add_input_pin(
            "scale",
            "Scale",
            "Render scale: 2 doubles the region's pixel size (on top of the display's pixel ratio); up to 10",
            VariableType::Float,
        )
        .set_default_value(Some(json!(2.0)));
        node.add_output_pin(
            "screenshot",
            "Screenshot",
            "Region as base64 PNG data",
            VariableType::String,
        );
        node.add_output_pin(
            "image",
            "Image",
            "Region as NodeImage",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        super::emulation::require_chromium(&session, "Zoom screenshots")?;
        let region = [
            context.evaluate_pin::<f64>("x").await?,
            context.evaluate_pin::<f64>("y").await?,
            context.evaluate_pin::<f64>("width").await?,
            context.evaluate_pin::<f64>("height").await?,
        ];
        let scale: f64 = context.evaluate_pin("scale").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;
        let metrics = super::cdp::cdp(&driver, "Page.getLayoutMetrics", json!({})).await?;
        let clip = zoom_clip(region, scale, &metrics)?;
        let png = super::capture::capture_png_via_cdp(&driver, clip, false).await?;
        drop(driver);

        super::capture::set_screenshot_outputs(context, png).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

/// Paper sizes in millimetres (portrait).
#[cfg(any(feature = "execute", test))]
const PAPER_FORMATS: [(&str, f64, f64); 7] = [
    ("A3", 297.0, 420.0),
    ("A4", 210.0, 297.0),
    ("A5", 148.0, 210.0),
    ("Letter", 215.9, 279.4),
    ("Legal", 215.9, 355.6),
    ("Tabloid", 279.4, 431.8),
    ("Ledger", 431.8, 279.4),
];

#[cfg(any(feature = "execute", test))]
#[derive(Debug, Clone, PartialEq)]
struct PdfOptions {
    width_mm: f64,
    height_mm: f64,
    landscape: bool,
    print_background: bool,
    scale: f64,
}

#[cfg(any(feature = "execute", test))]
impl PdfOptions {
    fn new(
        paper_format: &str,
        landscape: bool,
        print_background: bool,
        scale: f64,
    ) -> flow_like_types::Result<Self> {
        let (_, width_mm, height_mm) = PAPER_FORMATS
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(paper_format))
            .copied()
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Unknown PDF paper format '{paper_format}'; use one of {}",
                    PAPER_FORMATS.map(|(name, _, _)| name).join(", ")
                )
            })?;
        if !scale.is_finite() || !(0.1..=2.0).contains(&scale) {
            return Err(flow_like_types::anyhow!(
                "PDF scale must be between 0.1 and 2, got {scale}"
            ));
        }
        Ok(Self {
            width_mm,
            height_mm,
            landscape,
            print_background,
            scale,
        })
    }

    fn cdp_params(&self) -> Value {
        json!({
            "landscape": self.landscape,
            "printBackground": self.print_background,
            "scale": self.scale,
            "paperWidth": self.width_mm / 25.4,
            "paperHeight": self.height_mm / 25.4,
            "preferCSSPageSize": false,
            "transferMode": "ReturnAsBase64",
        })
    }

    #[cfg(feature = "execute")]
    fn webdriver_params(&self) -> thirtyfour::common::print::PrintParameters {
        use thirtyfour::common::print::{PrintOrientation, PrintPage, PrintParameters};
        PrintParameters {
            orientation: if self.landscape {
                PrintOrientation::Landscape
            } else {
                PrintOrientation::Portrait
            },
            scale: self.scale,
            background: self.print_background,
            page: PrintPage {
                width: self.width_mm / 10.0,
                height: self.height_mm / 10.0,
            },
            ..Default::default()
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserPrintPdfNode {}

impl BrowserPrintPdfNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserPrintPdfNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_print_pdf",
            "Save Page as PDF",
            "Prints the current page to a PDF file, the way the browser's print dialog would. Chrome and Edge print through DevTools (headless Chrome is the most reliable); other browsers use WebDriver printing.",
            "Capture",
        );
        node.set_flowscript_name("browser", "printPdf");
        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.add_input_pin(
            "file_path",
            "File Path",
            "Where to write the PDF; an existing file is replaced",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "paper_format",
            "Paper Format",
            "Paper size of each PDF page",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    PAPER_FORMAT_NAMES
                        .iter()
                        .map(|name| name.to_string())
                        .collect(),
                )
                .build(),
        )
        .set_default_value(Some(json!("A4")));
        node.add_input_pin(
            "landscape",
            "Landscape",
            "Print in landscape orientation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "print_background",
            "Print Background",
            "Include background colors and images",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "scale",
            "Scale",
            "Content scale between 0.1 and 2",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));
        node.add_output_pin(
            "pdf_path",
            "PDF",
            "The written PDF file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::base64::Engine;
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;
        let paper_format: String = context.evaluate_pin("paper_format").await?;
        let landscape: bool = context.evaluate_pin("landscape").await?;
        let print_background: bool = context.evaluate_pin("print_background").await?;
        let scale: f64 = context.evaluate_pin("scale").await?;
        let options = PdfOptions::new(&paper_format, landscape, print_background, scale)?;

        let driver = session.get_browser_driver_and_switch(context).await?;
        let pdf = if super::emulation::is_chromium(&session) {
            let printed = super::cdp::cdp(&driver, "Page.printToPDF", options.cdp_params()).await?;
            let data = printed["data"]
                .as_str()
                .ok_or_else(|| flow_like_types::anyhow!("Page.printToPDF returned no PDF data"))?;
            flow_like_types::base64::engine::general_purpose::STANDARD.decode(data)?
        } else {
            driver
                .print_page(options.webdriver_params())
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to print page to PDF: {e}"))?
        };
        drop(driver);

        file_path.put(context, pdf, false).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to write PDF to '{}': {e}", file_path.path)
        })?;
        context.set_pin_value("pdf_path", json!(file_path)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

const PAPER_FORMAT_NAMES: [&str; 7] = ["A3", "A4", "A5", "Letter", "Legal", "Tabloid", "Ledger"];

const MOUSE_BUTTONS: [&str; 3] = ["left", "middle", "right"];

/// W3C WebDriver key codepoints for modifier keys.
#[cfg(any(feature = "execute", test))]
fn modifier_key(name: &str) -> flow_like_types::Result<char> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok('\u{E009}'),
        "shift" => Ok('\u{E008}'),
        "alt" | "option" => Ok('\u{E00A}'),
        "meta" | "cmd" | "command" | "win" => Ok('\u{E03D}'),
        _ => Err(flow_like_types::anyhow!(
            "Unknown click modifier '{name}'; use Control, Shift, Alt, or Meta"
        )),
    }
}

/// WebDriver action sequence that clicks at a viewport point while holding modifiers.
#[cfg(any(feature = "execute", test))]
fn point_click_actions(
    x: f64,
    y: f64,
    button: &str,
    modifiers: &[String],
    click_count: i64,
) -> flow_like_types::Result<Value> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return Err(flow_like_types::anyhow!(
            "Click point must be finite, nonnegative viewport coordinates, got ({x}, {y})"
        ));
    }
    let button = MOUSE_BUTTONS
        .iter()
        .position(|candidate| *candidate == button)
        .ok_or_else(|| {
            flow_like_types::anyhow!("Click button must be left, middle, or right, got '{button}'")
        })?;
    if !(1..=3).contains(&click_count) {
        return Err(flow_like_types::anyhow!(
            "Click count must be 1, 2, or 3, got {click_count}"
        ));
    }
    let keys = modifiers
        .iter()
        .map(|name| modifier_key(name))
        .collect::<flow_like_types::Result<Vec<_>>>()?;
    let pause = json!({"type": "pause", "duration": 0});
    let mut key_actions = Vec::new();
    let mut pointer_actions = Vec::new();
    for key in &keys {
        key_actions.push(json!({"type": "keyDown", "value": key.to_string()}));
        pointer_actions.push(pause.clone());
    }
    key_actions.push(pause.clone());
    pointer_actions.push(json!({
        "type": "pointerMove",
        "duration": 0,
        "origin": "viewport",
        "x": x.round() as i64,
        "y": y.round() as i64,
    }));
    for _ in 0..click_count {
        pointer_actions.push(json!({"type": "pointerDown", "button": button}));
        pointer_actions.push(json!({"type": "pointerUp", "button": button}));
        key_actions.extend([pause.clone(), pause.clone()]);
    }
    for key in keys.iter().rev() {
        key_actions.push(json!({"type": "keyUp", "value": key.to_string()}));
        pointer_actions.push(pause.clone());
    }
    Ok(json!([
        {"type": "key", "id": "automation-keyboard", "actions": key_actions},
        {"type": "pointer", "id": "automation-mouse", "parameters": {"pointerType": "mouse"}, "actions": pointer_actions},
    ]))
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserClickAtPointNode {}

impl BrowserClickAtPointNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserClickAtPointNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_click_at_point",
            "Click At Point",
            "Clicks at viewport CSS coordinates instead of an element, for canvas apps and coordinates read off a screenshot by a vision model. Works in every WebDriver browser.",
            "Interact",
        );
        node.set_flowscript_name("browser", "clickAtPoint");
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(9)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "x",
            "X",
            "Horizontal position in viewport CSS pixels",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "y",
            "Y",
            "Vertical position in viewport CSS pixels",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "button",
            "Button",
            "Mouse button to press",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(MOUSE_BUTTONS.map(str::to_string).to_vec())
                .build(),
        )
        .set_default_value(Some(json!("left")));
        node.add_input_pin(
            "click_count",
            "Click Count",
            "1 for a click, 2 for a double click, 3 for a triple click",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_range((1.0, 3.0))
                .set_step(1.0)
                .build(),
        )
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Keys held during the click: Control, Shift, Alt, or Meta",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::common::command::{Actions, Command};
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let x: f64 = context.evaluate_pin("x").await?;
        let y: f64 = context.evaluate_pin("y").await?;
        let button: String = context.evaluate_pin("button").await?;
        let click_count: i64 = context.evaluate_pin("click_count").await?;
        let modifiers: Vec<String> = context.evaluate_pin("modifiers").await?;
        let actions = point_click_actions(x, y, &button, &modifiers, click_count)?;

        let driver = session.get_browser_driver_and_switch(context).await?;
        let result = driver
            .handle
            .cmd(Command::PerformActions(Actions::from(actions)))
            .await;
        let reset = driver.action_chain().reset_actions().await;
        result.map_err(|e| flow_like_types::anyhow!("Failed to click at ({x}, {y}): {e}"))?;
        reset?;
        drop(driver);

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

const SCROLL_MODES: [&str; 3] = ["by", "top", "bottom"];

#[cfg(feature = "execute")]
const SCROLL_SCRIPT: &str = "const [scope, mode, dx, dy] = arguments; const target = scope || document.scrollingElement || document.documentElement; if (mode === 'top') { target.scrollTo({ left: target.scrollLeft, top: 0, behavior: 'instant' }); } else if (mode === 'bottom') { target.scrollTo({ left: target.scrollLeft, top: target.scrollHeight, behavior: 'instant' }); } else { target.scrollBy({ left: dx, top: dy, behavior: 'instant' }); } const maxTop = Math.max(0, target.scrollHeight - target.clientHeight); return { x: target.scrollLeft, y: target.scrollTop, atBottom: target.scrollTop >= maxTop - 1 };";

#[crate::register_node]
#[derive(Default)]
pub struct BrowserScrollPageNode {}

impl BrowserScrollPageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserScrollPageNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_scroll_page",
            "Scroll Page",
            "Scrolls the page, or a scrollable container, by a distance or to its top or bottom, and reports where it ended up. Use At Bottom to stop infinite-scroll loops.",
            "Interact",
        );
        node.set_flowscript_name("browser", "scrollPage");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "mode",
            "Mode",
            "by scrolls by Delta X/Y; top and bottom jump to the vertical ends",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(SCROLL_MODES.map(str::to_string).to_vec())
                .build(),
        )
        .set_default_value(Some(json!("by")));
        node.add_input_pin(
            "delta_x",
            "Delta X",
            "Horizontal distance in CSS pixels for mode 'by'; negative scrolls left",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "delta_y",
            "Delta Y",
            "Vertical distance in CSS pixels for mode 'by'; negative scrolls up",
            VariableType::Float,
        )
        .set_default_value(Some(json!(600.0)));
        node.add_input_pin(
            "scope",
            "Container",
            "CSS selector of a scrollable container; empty scrolls the page",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        super::selector::add_locator_pin(&mut node);
        node.add_output_pin(
            "scroll_x",
            "Scroll X",
            "Horizontal scroll offset after scrolling, in CSS pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "scroll_y",
            "Scroll Y",
            "Vertical scroll offset after scrolling, in CSS pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "at_bottom",
            "At Bottom",
            "True when the page or container cannot scroll further down",
            VariableType::Boolean,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let mode: String = context.evaluate_pin("mode").await?;
        if !SCROLL_MODES.contains(&mode.as_str()) {
            return Err(flow_like_types::anyhow!(
                "Scroll mode must be by, top, or bottom, got '{mode}'"
            ));
        }
        let delta_x: f64 = context.evaluate_pin("delta_x").await?;
        let delta_y: f64 = context.evaluate_pin("delta_y").await?;
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return Err(flow_like_types::anyhow!(
                "Scroll distances must be finite, got ({delta_x}, {delta_y})"
            ));
        }
        let scope: String = context.evaluate_pin("scope").await?;
        let locator = super::selector::evaluate_locator(context, &scope).await?;

        let driver = session.get_browser_driver_and_switch(context).await?;
        let container = if locator.value.is_empty() {
            Value::Null
        } else {
            super::selector::find(&driver, &locator)
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!(
                        "Failed to find scroll container '{}': {e}",
                        locator.value
                    )
                })?
                .to_json()?
        };
        let position = driver
            .execute(
                SCROLL_SCRIPT,
                vec![container, json!(mode), json!(delta_x), json!(delta_y)],
            )
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to scroll ({mode}): {e}"))?;
        drop(driver);
        let position = position.json();
        let offset = |name: &str| position[name].as_f64().unwrap_or_default().round() as i64;

        context
            .set_pin_value("scroll_x", json!(offset("x")))
            .await?;
        context
            .set_pin_value("scroll_y", json!(offset("y")))
            .await?;
        context
            .set_pin_value("at_bottom", json!(position["atBottom"] == true))
            .await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(
            truncate_chars("héllo wörld".into(), 5),
            ("héllo".into(), true)
        );
        assert_eq!(truncate_chars("short".into(), 5), ("short".into(), false));
        assert_eq!(
            truncate_chars("unbounded".into(), 0),
            ("unbounded".into(), false)
        );
        assert_eq!(truncate_chars("🙂🙂🙂".into(), 2), ("🙂🙂".into(), true));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn markdown_keeps_structure_and_drops_scripts() {
        let markdown = html_to_markdown(
            "<body><h1>Title</h1><script>steal()</script><p>See <a href=\"/docs\">docs</a></p><ul><li>one</li></ul></body>",
        )
        .unwrap();
        assert!(markdown.contains("# Title"));
        assert!(markdown.contains("[docs](/docs)"));
        assert!(markdown.contains("one"));
        assert!(!markdown.contains("steal"));
    }

    #[test]
    fn zoom_clip_moves_viewport_region_into_document_space() {
        let metrics = json!({"cssVisualViewport": {"pageX": 10.0, "pageY": 1200.0}});
        assert_eq!(
            zoom_clip([5.0, 20.0, 100.0, 50.0], 3.0, &metrics).unwrap(),
            json!({"x": 15.0, "y": 1220.0, "width": 100.0, "height": 50.0, "scale": 3.0})
        );
        assert!(zoom_clip([0.0, 0.0, 0.0, 10.0], 2.0, &metrics).is_err());
        assert!(zoom_clip([-1.0, 0.0, 10.0, 10.0], 2.0, &metrics).is_err());
        assert!(zoom_clip([0.0, 0.0, 10.0, 10.0], f64::NAN, &metrics).is_err());
        assert!(zoom_clip([0.0, 0.0, 4000.0, 10.0], 5.0, &metrics).is_err());
    }

    #[test]
    fn pdf_options_map_paper_formats_to_inches() {
        let options = PdfOptions::new("letter", true, false, 1.5).unwrap();
        let params = options.cdp_params();
        assert_eq!(params["paperWidth"], json!(8.5));
        assert_eq!(params["paperHeight"], json!(11.0));
        assert_eq!(params["landscape"], json!(true));
        assert_eq!(params["printBackground"], json!(false));
        assert_eq!(params["scale"], json!(1.5));
        let a4 = PdfOptions::new("A4", false, true, 1.0)
            .unwrap()
            .cdp_params();
        assert!((a4["paperWidth"].as_f64().unwrap() - 8.2677).abs() < 1e-3);
        assert!((a4["paperHeight"].as_f64().unwrap() - 11.6929).abs() < 1e-3);
        assert!(PdfOptions::new("B5", false, true, 1.0).is_err());
        assert!(PdfOptions::new("A4", false, true, 2.5).is_err());
        assert!(PdfOptions::new("A4", false, true, f64::NAN).is_err());
        assert_eq!(PAPER_FORMATS.map(|(name, _, _)| name), PAPER_FORMAT_NAMES);
    }

    #[cfg(feature = "execute")]
    #[test]
    fn pdf_options_map_to_webdriver_centimetres() {
        let params = PdfOptions::new("A4", true, true, 0.5)
            .unwrap()
            .webdriver_params();
        assert_eq!(params.page.width, 21.0);
        assert_eq!(params.page.height, 29.7);
        assert_eq!(
            params.orientation,
            thirtyfour::common::print::PrintOrientation::Landscape
        );
        assert!(params.background);
        assert_eq!(params.scale, 0.5);
    }

    #[test]
    fn point_click_uses_viewport_origin_and_releases_modifiers() {
        let actions =
            point_click_actions(10.4, 20.6, "right", &["Shift".into(), "ctrl".into()], 2).unwrap();
        let keys = actions[0]["actions"].as_array().unwrap();
        let pointer = actions[1]["actions"].as_array().unwrap();
        assert_eq!(keys.len(), pointer.len());
        assert_eq!(keys[0], json!({"type": "keyDown", "value": "\u{E008}"}));
        assert_eq!(keys[1], json!({"type": "keyDown", "value": "\u{E009}"}));
        assert_eq!(
            keys[keys.len() - 1],
            json!({"type": "keyUp", "value": "\u{E008}"})
        );
        let moves: Vec<_> = pointer
            .iter()
            .filter(|action| action["type"] == "pointerMove")
            .collect();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0]["origin"], json!("viewport"));
        assert_eq!(
            (moves[0]["x"].clone(), moves[0]["y"].clone()),
            (json!(10), json!(21))
        );
        let downs = pointer
            .iter()
            .filter(|action| action["type"] == "pointerDown")
            .count();
        assert_eq!(downs, 2);
        assert!(pointer.iter().any(|action| action["button"] == json!(2)));
        assert!(point_click_actions(-1.0, 0.0, "left", &[], 1).is_err());
        assert!(point_click_actions(0.0, 0.0, "back", &[], 1).is_err());
        assert!(point_click_actions(0.0, 0.0, "left", &[], 4).is_err());
        assert!(point_click_actions(0.0, 0.0, "left", &["hyper".into()], 1).is_err());
    }
}
