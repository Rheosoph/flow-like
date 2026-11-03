#[path = "live_mcp.rs"]
mod live_mcp;
use std::cell::RefCell;

use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services, theme::ThemeKind};
use serde_json::json;
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 50 * 1024 * 1024;
const INPUTS: &[&str] = &[
    "pcraft", "ora", "psd", "psb", "psdt", "png", "apng", "jpg", "jpeg", "jpe", "jfif", "tif",
    "tiff", "webp", "gif", "bmp", "dib", "tga", "icb", "vda", "vst", "ico", "qoi", "exr", "hdr",
    "pnm", "pbm", "pgm", "ppm", "pam", "pfm", "fits", "fit", "fts", "pdn", "pcx", "kra", "dng",
    "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "afphoto", "xcf", "xpm", "dds",
    "dpx", "cin", "dcm", "dicom", "dc3", "iff", "lbm", "ilbm",
];

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
    if matches!(id, "file.save" | "file.saveAs") {
        event("save-request", json!({}));
        return Some(Ok(json!({"pending": "hostSave"})));
    }
    if id.starts_with("file.")
        || id.starts_with("app.")
        || id.starts_with("help.")
        || matches!(
            id,
            "layer.smartObjects.editContents" | "window.newWindowForDocument"
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
        .ok_or("The photo canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid photo canvas.")?;
    let mut options = eframe::WebOptions::default();
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
                let services = Services {
                    host_command: Some(Box::new(host_command)),
                    automation_command: Some(Box::new(|id, params| {
                        live_mcp::authorize_command(id, params, false)
                    })),
                    automation_authorize: Some(|id, params| {
                        live_mcp::authorize_command(id, params, false)
                            .map_err(photocraft_engine::EngineError::Other)
                    }),
                    ..Default::default()
                };
                let mut app = PhotocraftApp::new(Session::new(), services);
                app.background_jobs = false;
                app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
                if let Some(state) = cc.wgpu_render_state.clone() {
                    app.set_wgpu(state);
                }
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    name: String::new(),
                    document_id: None,
                    reported_dirty: false,
                    new_document: false,
                    last_exported: None,
                    preview_revision: 0,
                    texture: None,
                }))
            }),
        )
        .await
        .map_err(|e| format!("Flows Photo could not start: {e:?}"))
}

struct Host {
    app: PhotocraftApp,
    opened: bool,
    read_only: bool,
    name: String,
    document_id: Option<photocraft_doc::DocId>,
    reported_dirty: bool,
    new_document: bool,
    last_exported: Option<u64>,
    preview_revision: u64,
    texture: Option<egui::TextureHandle>,
}

impl Host {
    fn commit(&mut self) {
        photocraft_ui_egui::type_tool::commit(&mut self.app);
        photocraft_ui_egui::transform_tool::commit(&mut self.app);
    }

    fn receive(&mut self, message: JsValue) -> Result<(), String> {
        match field(&message, "type").as_string().as_deref() {
            Some("mcp-request") => {
                if !self.opened {
                    return Err("Open a document first.".into());
                }
                let ctx = CONTEXT
                    .with(|context| context.borrow().clone())
                    .ok_or("The editor is unavailable.")?;
                let detail = live_mcp::receive(&message, self.read_only, &mut |method, params| {
                    let (request, _) =
                        photocraft_ui_egui::control::ControlRequest::new(method, params);
                    match photocraft_ui_egui::control::handle(&mut self.app, &ctx, &request) {
                        photocraft_ui_egui::control::Outcome::Done(reply) => {
                            live_mcp::unwrap_control(reply)
                        }
                        _ => Err("This operation needs a native editor window.".into()),
                    }
                })
                .ok_or("Invalid MCP request.")?;
                event("mcp-response", detail);
            }
            Some("create") => {
                if self.opened {
                    return Err("This editor already has a file open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A file name is required.")?;
                if !INPUTS.contains(&extension(&name).as_str()) {
                    return Err("This image format is unsupported.".into());
                }
                self.app
                    .session
                    .execute("file.new", json!({"name": name}))
                    .map_err(|e| e.to_string())?;
                self.app.sync_views();
                self.document_id = self.app.session.active().map(|state| state.doc.id);
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                self.new_document = true;
                event("opened", json!({"readOnly": self.read_only}));
            }
            Some("open") => {
                if self.opened {
                    return Err("This editor already has a file open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A file name is required.")?;
                if !INPUTS.contains(&extension(&name).as_str()) {
                    return Err("This image format is unsupported.".into());
                }
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("File bytes are required.".into());
                }
                let bytes = js_sys::Uint8Array::new(&data);
                if bytes.length() == 0 || bytes.length() as usize > MAX_BYTES {
                    return Err("Files must be between 1 byte and 50 MiB.".into());
                }
                let imported =
                    photocraft_io::import(&name, &bytes.to_vec()).map_err(|e| e.to_string())?;
                self.app.session.add_document(imported.document, None);
                self.app.sync_views();
                self.document_id = self.app.session.active().map(|state| state.doc.id);
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"readOnly": self.read_only, "warnings": imported.warnings, "previewOnly": imported.preview_only}),
                );
            }
            Some("set-read-only") => {
                if !self.opened {
                    return Err("Open a file first.".into());
                }
                self.commit();
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
            }
            Some("export") => {
                let download =
                    field(&message, "purpose").as_string().as_deref() == Some("download");
                if !self.opened || (self.read_only && !download) {
                    return Err("Enable editing before saving.".into());
                }
                if self.app.session.active().map(|state| state.doc.id) != self.document_id {
                    return Err("Switch back to the original file's tab before saving.".into());
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("A save request identifier is required.")?;
                let format = field(&message, "format")
                    .as_string()
                    .unwrap_or_else(|| extension(&self.name));
                if !matches!(
                    format.as_str(),
                    "pcraft"
                        | "ora"
                        | "psd"
                        | "psb"
                        | "png"
                        | "jpg"
                        | "jpeg"
                        | "jpe"
                        | "jfif"
                        | "tif"
                        | "tiff"
                        | "webp"
                        | "gif"
                        | "bmp"
                        | "dib"
                        | "tga"
                        | "icb"
                        | "vda"
                        | "vst"
                        | "ico"
                        | "qoi"
                        | "exr"
                        | "hdr"
                        | "pnm"
                        | "fits"
                        | "fit"
                        | "fts"
                        | "dds"
                        | "dpx"
                        | "cin"
                        | "iff"
                        | "lbm"
                        | "ilbm"
                ) {
                    return Err(
                        "This image format is import-only. Save a new PCraft, PSD, or PNG file."
                            .into(),
                    );
                }
                self.commit();
                let state = self.app.session.active().ok_or("No file is open.")?;
                let mut options = photocraft_io::ExportOptions {
                    tiff_layers: true,
                    ..Default::default()
                };
                photocraft_io::apply_file_encoding(&state.doc, &mut options.encode);
                let exported = photocraft_io::export(&state.doc, &format, &options)
                    .map_err(|e| e.to_string())?;
                if exported.bytes.is_empty() || exported.bytes.len() > MAX_BYTES {
                    return Err("The exported file exceeds 50 MiB or is empty.".into());
                }
                let revision = state.revision;
                if !download {
                    self.last_exported = Some(revision);
                }
                emit("exported", &json!({"requestId": request, "revision": revision, "warnings": exported.warnings}).to_string(), &exported.bytes);
            }
            Some("saved") => {
                if let Some(revision) = field(&message, "revision").as_f64() {
                    if let Some(state) = self.app.session.active_mut() {
                        if Some(state.doc.id) == self.document_id
                            && self.last_exported == Some(state.revision)
                            && state.revision as f64 == revision
                        {
                            state.saved_revision = state.revision;
                            self.new_document = false;
                            self.last_exported = None;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn report_dirty(&mut self) {
        let dirty = self.new_document
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
        if self.texture.is_none() || self.preview_revision != state.revision {
            let image = photocraft_compose::thumbnail(&state.doc, 2048);
            self.texture = Some(ui.ctx().load_texture(
                "photo-preview",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width as usize, image.height as usize],
                    &image.pixels,
                ),
                egui::TextureOptions::LINEAR,
            ));
            self.preview_revision = state.revision;
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let available = ui.available_size();
                let scale = (available.x / size.x)
                    .min(available.y / size.y)
                    .min(1.5)
                    .max(0.01);
                let rect = egui::Rect::from_center_size(ui.max_rect().center(), size * scale);
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        });
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
            self.app.logic(ctx, frame);
        }
        self.report_dirty();
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
        if self.read_only {
            self.app.cancel_synthetic("The editor is read-only.");
        }
        if !self.read_only {
            self.app.raw_input_hook(ctx, raw);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.opened {
            return;
        }
        if self.read_only {
            self.preview(ui);
        } else {
            self.app.ui(ui, frame);
        }
        self.report_dirty();
    }
}
