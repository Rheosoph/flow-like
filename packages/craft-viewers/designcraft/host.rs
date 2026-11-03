#[path = "live_mcp.rs"]
mod live_mcp;
use std::cell::RefCell;
use std::collections::HashMap;

use designcraft_engine::Session;
use designcraft_ui_egui::{DesignApp, Services};
use serde_json::json;
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 50 * 1024 * 1024;
const INPUTS: &[&str] = &["designcraft", "idml", "indd", "indt"];
const IDML_WARNING: &str = "InDesign import and IDML export may change unsupported layout features. Review the import warnings before saving. Linked assets outside this file are unavailable; use embedded graphics.";

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

fn host_command(id: &str) -> Option<Result<serde_json::Value, String>> {
    if matches!(
        id,
        "file.save" | "file.saveAs" | "document.save" | "document.saveAs"
    ) {
        event("save-request", json!({}));
        return Some(Ok(json!({"pending": "hostSave"})));
    }
    if id.starts_with("file.")
        || id.starts_with("help.")
        || id.starts_with("document.open")
        || id.starts_with("document.save")
        || id.starts_with("document.export")
        || id.starts_with("document.close")
        || matches!(
            id,
            "document.new"
                | "document.revert"
                | "ui.home"
                | "ui.discord"
                | "app.home"
                | "app.about"
                | "app.quit"
        )
    {
        return Some(Err("Use the file controls above the editor.".into()));
    }
    None
}

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Warn).ok();
    wasm_bindgen_futures::spawn_local(async {
        match start_app().await {
            Ok(()) => event("ready", json!({})),
            Err(error) => event("error", json!({"error": error})),
        }
    });
}

async fn start_app() -> Result<(), String> {
    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("craft_canvas"))
        .ok_or("The layout canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid layout canvas.")?;
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                let services = Services {
                    host_command: Some(Box::new(host_command)),
                    ..Default::default()
                };
                Ok(Box::new(Host {
                    app: DesignApp::new(Session::new(), services),
                    opened: false,
                    read_only: true,
                    name: String::new(),
                    document_id: None,
                    reported_dirty: false,
                    unsaved_new: false,
                    last_exported: None,
                    preview_revision: 0,
                    textures: HashMap::new(),
                    renderer: designcraft_render::Renderer::new(),
                }))
            }),
        )
        .await
        .map_err(|e| format!("Flows Design could not start: {e:?}"))
}

struct Host {
    app: DesignApp,
    opened: bool,
    read_only: bool,
    name: String,
    document_id: Option<u64>,
    reported_dirty: bool,
    unsaved_new: bool,
    last_exported: Option<u64>,
    preview_revision: u64,
    textures: HashMap<usize, egui::TextureHandle>,
    renderer: designcraft_render::Renderer,
}

impl Host {
    fn receive(&mut self, message: JsValue) -> Result<(), String> {
        match field(&message, "type").as_string().as_deref() {
            Some("mcp-request") => {
                let ctx = CONTEXT
                    .with(|context| context.borrow().clone())
                    .ok_or("The editor is not ready.")?;
                let read_only = self.read_only || !self.opened;
                if let Some(response) =
                    live_mcp::receive(&message, read_only, &mut |method, params| {
                        let (request, _reply) =
                            designcraft_ui_egui::ControlRequest::new(method, params);
                        match designcraft_ui_egui::control::handle(&mut self.app, &ctx, &request) {
                            designcraft_ui_egui::control::Outcome::Done(reply) => {
                                live_mcp::unwrap_control(reply)
                            }
                            _ => Err("This operation requires a later editor frame.".into()),
                        }
                    })
                {
                    event("mcp-response", response);
                }
                ctx.request_repaint();
            }

            Some("create") => {
                if self.opened {
                    return Err("This editor already has a document open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A document name is required.")?;
                if !name.to_ascii_lowercase().ends_with(".designcraft") {
                    return Err("New documents use the .designcraft format.".into());
                }
                self.app
                    .session
                    .execute("file.new", &json!({"title": name}))
                    .map_err(|error| error.to_string())?;
                self.app.after_engine();
                self.document_id = self.app.session.active().map(|state| state.uid);
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                self.unsaved_new = true;
                event(
                    "opened",
                    json!({"revision": self.app.session.active().map(|state| state.revision).unwrap_or(0), "readOnly": self.read_only}),
                );
            }
            Some("open") => {
                if self.opened {
                    return Err("This editor already has a file open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A file name is required.")?;
                if !INPUTS.contains(&extension(&name).as_str()) {
                    return Err("This layout format is unsupported.".into());
                }
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("File bytes are required.".into());
                }
                let bytes = js_sys::Uint8Array::new(&data);
                if bytes.length() == 0 || bytes.length() as usize > MAX_BYTES {
                    return Err("Files must be between 1 byte and 50 MiB.".into());
                }
                let bytes = bytes.to_vec();
                let opened = self
                    .app
                    .session
                    .execute(
                        "file.openBytes",
                        &json!({
                            "name": name,
                            "base64": designcraft_engine::cmd::base64_encode(&bytes),
                        }),
                    )
                    .map_err(|error| error.to_string())?;
                let mut warnings: Vec<String> = opened
                    .get("warnings")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect();
                if extension(&name) != "designcraft" {
                    warnings.insert(0, IDML_WARNING.into());
                }
                self.app.after_engine();

                self.document_id = self.app.session.active().map(|state| state.uid);
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"readOnly": self.read_only, "warnings": warnings}),
                );
            }
            Some("set-read-only") => {
                if !self.opened {
                    return Err("Open a file first.".into());
                }
                self.app
                    .session
                    .commit_interaction()
                    .map_err(|e| e.to_string())?;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.app.ui.dialog = None;
            }
            Some("export") => {
                if !self.opened
                    || (self.read_only
                        && field(&message, "purpose").as_string().as_deref() != Some("download"))
                {
                    return Err("Enable editing before saving.".into());
                }
                if self.app.session.active().map(|state| state.uid) != self.document_id {
                    return Err("Switch back to the original file's tab before saving.".into());
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("A save request identifier is required.")?;
                let format = field(&message, "format").as_string().unwrap_or_else(|| {
                    match extension(&self.name).as_str() {
                        "indd" | "indt" => "designcraft".into(),
                        format => format.to_owned(),
                    }
                });
                if !matches!(format.as_str(), "designcraft" | "idml") {
                    return Err("This layout export format is unsupported.".into());
                }
                if !self.read_only {
                    self.app
                        .session
                        .commit_interaction()
                        .map_err(|e| e.to_string())?;
                }
                let state = self.app.session.active().ok_or("No file is open.")?;
                let (bytes, warnings) = if format == "idml" {
                    (
                        designcraft_idml::export_idml(&state.doc),
                        vec![IDML_WARNING],
                    )
                } else {
                    (
                        designcraft_format::save(&state.doc).map_err(|e| e.to_string())?,
                        Vec::new(),
                    )
                };
                if bytes.is_empty() || bytes.len() > MAX_BYTES {
                    return Err("The exported file exceeds 50 MiB or is empty.".into());
                }
                let revision = state.revision;
                if field(&message, "purpose").as_string().as_deref() != Some("download") {
                    self.last_exported = Some(revision);
                }
                emit(
                    "exported",
                    &json!({"requestId": request, "revision": revision, "warnings": warnings})
                        .to_string(),
                    &bytes,
                );
            }
            Some("saved") => {
                if let Some(revision) = field(&message, "revision").as_f64() {
                    if let Some(state) = self.app.session.active_mut() {
                        if Some(state.uid) == self.document_id
                            && self.last_exported == Some(state.revision)
                            && state.revision as f64 == revision
                        {
                            state.saved_doc = state.doc.clone();
                            state.saved_revision = state.revision;
                            self.last_exported = None;
                            self.unsaved_new = false;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn report_dirty(&mut self) {
        let dirty = self.unsaved_new
            || self
                .app
                .session
                .documents()
                .iter()
                .any(|state| state.is_dirty());
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event("dirty", json!({"dirty": dirty}));
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.app.session.active() else {
            return;
        };
        if self.preview_revision != state.revision {
            self.textures.clear();
            self.preview_revision = state.revision;
        }
        let mut visible = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for index in 0..state.doc.page_count() {
                        let Some((spread, page)) = state.doc.page_loc(index) else {
                            continue;
                        };
                        let bounds = state.doc.spreads[spread].pages[page].bounds();
                        let longest = bounds.width().max(bounds.height()).max(1.0);
                        let scale = (2048.0 / longest).min(2.0);
                        let width = (bounds.width() as f32).max(1.0);
                        let height = (bounds.height() as f32).max(1.0);
                        let fit = ((ui.available_width() - 32.0).max(1.0) / width).min(1.5);
                        ui.add_space(16.0);
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(width * fit, height * fit),
                            egui::Sense::hover(),
                        );
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        visible.push(index);
                        if !self.textures.contains_key(&index) {
                            if let Some(image) = self.renderer.render_page(
                                &state.doc,
                                &self.app.session.cache,
                                index,
                                scale,
                                false,
                                &Default::default(),
                            ) {
                                self.textures.insert(
                                    index,
                                    ui.ctx().load_texture(
                                        format!("page-{index}"),
                                        egui::ColorImage::from_rgba_premultiplied(
                                            [image.width as usize, image.height as usize],
                                            &image.pixels,
                                        ),
                                        egui::TextureOptions::LINEAR,
                                    ),
                                );
                            }
                        }
                        if let Some(texture) = self.textures.get(&index) {
                            ui.painter().image(
                                texture.id(),
                                rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                });
        });
        self.textures.retain(|index, _| visible.contains(index));
    }
}

impl eframe::App for Host {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            self.app.logic(ctx);
        }
        self.report_dirty();
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
        if self.read_only {
            self.app.host_cancel_synthetic();
        }
        if !self.read_only {
            self.app.raw_input_hook(ctx, raw);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.opened {
            return;
        }
        if self.read_only {
            self.preview(ui);
        } else {
            self.app.ui(ui);
        }
        self.report_dirty();
    }
}
