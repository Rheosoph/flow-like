use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 200 * 1024 * 1024;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftEmit)]
    fn emit(kind: &str, detail: &str, bytes: &[u8]);
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftTake)]
    fn take() -> JsValue;
}
thread_local! {
    static CONTEXT: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}
#[wasm_bindgen]
pub fn request_repaint() {
    CONTEXT.with(|ctx| {
        if let Some(ctx) = ctx.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}
fn event(kind: &str, detail: serde_json::Value) {
    emit(kind, &detail.to_string(), &[]);
}
fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}
fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        match start_app().await {
            Ok(()) => event("ready", json!({})),
            Err(error) => event("error", json!({"error": error})),
        }
    });
}
async fn start_app() -> Result<(), String> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("The document is unavailable.")?;
    let canvas = document
        .get_element_by_id("craft_canvas")
        .ok_or("The document canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid document canvas.")?;
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions {
                renderer: eframe::Renderer::Glow,
                ..Default::default()
            },
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                let app = create_app(cc)?;
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    name: String::new(),
                    reported_dirty: false,
                    last_exported: None,
                    exported_bytes: None,
                }))
            }),
        )
        .await
        .map_err(|e| format!("The document editor could not start: {e:?}"))
}
struct Host {
    app: pdfcraft_ui_egui::PdfCraftApp,
    opened: bool,
    read_only: bool,
    name: String,
    reported_dirty: bool,
    last_exported: Option<u64>,
    exported_bytes: Option<std::sync::Arc<Vec<u8>>>,
}
impl Host {
    fn receive(&mut self, message: JsValue) -> Result<(), String> {
        let ctx = CONTEXT
            .with(|context| context.borrow().clone())
            .ok_or("The editor is not ready.")?;
        match field(&message, "type").as_string().as_deref() {
            Some("mcp-request") => {
                let request_id = field(&message, "requestId")
                    .as_string()
                    .ok_or("Missing MCP request ID.")?;
                let text = js_sys::JSON::stringify(&field(&message, "message"))
                    .map_err(|_| "Invalid MCP message.")?
                    .as_string()
                    .ok_or("Invalid MCP message.")?;
                let request: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let response = if self.opened {
                    self.app.host_mcp(&ctx, &request, self.read_only)
                } else {
                    json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"Open a PDF first."}})
                };
                event(
                    "mcp-response",
                    json!({"requestId":request_id,"response":response}),
                );
            }
            Some("create") => {
                if self.opened {
                    return Err("This editor already has a document open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A document name is required.")?;
                if extension(&name) != "pdf" {
                    return Err("Only PDF documents are supported.".into());
                }
                self.app.host_create(&name)?;
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"revision": self.revision(), "readOnly": self.read_only}),
                );
            }
            Some("open") => {
                if self.opened {
                    return Err("This editor already has a document open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A document name is required.")?;
                if extension(&name) != "pdf" {
                    return Err("Only PDF documents are supported.".into());
                }
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("Document bytes are required.".into());
                }
                let bytes = js_sys::Uint8Array::new(&data);
                if bytes.length() == 0 || bytes.length() as usize > MAX_BYTES {
                    return Err("Documents must be between 1 byte and 200 MiB.".into());
                }
                self.app.open_bytes(&ctx, &name, None, bytes.to_vec())?;
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"revision": self.revision(), "readOnly": self.read_only}),
                );
            }
            Some("set-read-only") => {
                if !self.opened {
                    return Err("Open a document first.".into());
                }
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
            }
            Some("export") => {
                let download =
                    field(&message, "purpose").as_string().as_deref() == Some("download");
                if !self.opened {
                    return Err("Open a document first.".into());
                }
                if self.read_only && !download {
                    return Err("Enable editing before saving.".into());
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("An export request ID is required.")?;
                let format = field(&message, "format")
                    .as_string()
                    .unwrap_or_else(|| extension(&self.name));
                if format != "pdf" {
                    return Err("Only PDF export is supported.".into());
                }
                let (revision, bytes) = self.app.host_export(&ctx)?;
                if bytes.is_empty() || bytes.len() > MAX_BYTES {
                    return Err("The saved document exceeds 200 MiB.".into());
                }
                if !download {
                    self.last_exported = Some(revision);
                    self.exported_bytes = Some(bytes.clone());
                }
                emit(
                    "exported",
                    &json!({"requestId": request, "revision": revision}).to_string(),
                    &bytes,
                );
            }
            Some("saved") => {
                if let Some(revision) = field(&message, "revision").as_f64() {
                    if self.last_exported == Some(revision as u64)
                        && self.revision() as f64 == revision
                    {
                        if let Some(bytes) = self.exported_bytes.clone() {
                            if self.app.host_saved(&ctx, revision as u64, bytes)? {
                                self.last_exported = None;
                                self.exported_bytes = None;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn revision(&self) -> u64 {
        self.app.host_revision()
    }
    fn report_dirty(&mut self) {
        let dirty = self.app.first_dirty().is_some();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event(
                "dirty",
                json!({"dirty": dirty, "revision": self.revision()}),
            );
        }
    }
}
impl eframe::App for Host {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for _ in 0..8 {
            let message = take();
            if message.is_null() || message.is_undefined() {
                break;
            }
            let request_id = field(&message, "requestId").as_string();
            if let Err(error) = self.receive(message) {
                event("error", json!({"requestId": request_id, "error": error}));
            }
        }
        if self.opened && !self.read_only {
            eframe::App::logic(&mut self.app, ctx, frame);
        }
        self.report_dirty();
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
        if self.read_only {
            raw.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Key { .. }
                )
            });
        } else {
            eframe::App::raw_input_hook(&mut self.app, ctx, raw);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.opened {
            return;
        }
        if self.read_only {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("Enable editing to use PDF tools.");
            });
        } else {
            eframe::App::ui(&mut self.app, ui, frame);
        }
        self.report_dirty();
    }
}

fn create_app(cc: &eframe::CreationContext<'_>) -> Result<pdfcraft_ui_egui::PdfCraftApp, String> {
    use std::sync::Arc;
    let ctx = cc.egui_ctx.clone();
    let system = craft::ui::runtime::env::SystemReader::start(move || ctx.request_repaint());
    let spec = craft::ui::InstallSpec::new(
        craft::host::target::ShellInfo::default(),
        Arc::new(craft::window::system::SystemWindows),
    )
    .system(system);
    let lang = pdfcraft_ui_egui::i18n::Lang::from_pref(pdfcraft_ui_egui::i18n::AUTO);
    craft::ui::shell::embed::install(&cc.egui_ctx, spec, &pdfcraft_ui_egui::fonts::ui_fonts(lang))
        .map_err(|error| format!("The PDF runtime could not start: {error}"))?;
    let ctx = cc.egui_ctx.clone();
    let wake = craft::host::Wake::new(move || ctx.request_repaint());
    let (intake, _) = craft::platform::open::FileIntake::new(wake.clone());
    let files = craft::platform::browser_files::BrowserFiles::new(move || wake.wake()).limits(
        craft::platform::browser_files::PickLimits {
            pending_bytes: MAX_BYTES,
            ..craft::platform::browser_files::PickLimits::DEFAULT
        },
    );
    let framework_host = craft::host::Host::builder(pdfcraft_ui_egui::APP_INFO)
        .files(Arc::new(files))
        .build();
    let mut app = pdfcraft_ui_egui::PdfCraftApp::with_host(
        Arc::new(framework_host),
        pdfcraft_ui_egui::AppServices::system(&cc.egui_ctx, intake),
    );
    app.comment_prefs.author = "Flows PDF User".into();
    app.host_save = Some(Box::new(|| event("save-request", json!({}))));
    app.mode = pdfcraft_ui_egui::Mode::Edit;
    Ok(app)
}
