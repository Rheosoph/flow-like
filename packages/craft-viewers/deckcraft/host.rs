#[path = "live_mcp.rs"]
mod live_mcp;
use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 50 * 1024 * 1024;

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
    eframe::WebLogger::init(log::LevelFilter::Warn).ok();
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
            eframe::WebOptions::default(),
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                let ctx = cc.egui_ctx.clone();
                let repaint = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                    ctx.request_repaint()
                });
                deckcraft_media::browser::set_repaint(repaint.as_ref().unchecked_ref());
                repaint.forget();
                let app = create_app(cc);
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    name: String::new(),
                    reported_dirty: false,
                    unsaved_new: false,
                    last_exported: None,
                    preview_revision: 0,
                    textures: Default::default(),
                }))
            }),
        )
        .await
        .map_err(|e| format!("The document editor could not start: {e:?}"))
}
struct Host {
    app: deckcraft_ui_egui::SlideApp,
    opened: bool,
    read_only: bool,
    name: String,
    reported_dirty: bool,
    unsaved_new: bool,
    last_exported: Option<u64>,
    preview_revision: u64,
    textures: std::collections::HashMap<usize, (f32, egui::TextureHandle)>,
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
                            deckcraft_ui_egui::ControlRequest::new(method, params);
                        match deckcraft_ui_egui::control::handle(&mut self.app, &ctx, &request) {
                            deckcraft_ui_egui::control::Outcome::Done(reply) => {
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
                if !name.to_ascii_lowercase().ends_with(".pptx") {
                    return Err("New documents use the .pptx format.".into());
                }
                let hosted = self.app.session.hosted;
                self.app.session.hosted = false;
                let created = self.app.session.execute("file.new", &json!({}));
                self.app.session.hosted = hosted;
                created.map_err(|error| error.to_string())?;
                self.app.backstage = None;
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                self.unsaved_new = true;
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
                if !matches!(extension(&name).as_str(), "pptx" | "potx" | "ppsx") {
                    return Err("This document format is not supported.".into());
                }
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("Document bytes are required.".into());
                }
                let bytes = js_sys::Uint8Array::new(&data);
                if bytes.length() == 0 || bytes.length() as usize > MAX_BYTES {
                    return Err("Documents must be between 1 byte and 50 MiB.".into());
                }
                let doc = deckcraft_engine::cmd::file::open_presentation(&name, &bytes.to_vec())
                    .map_err(|e| e.to_string())?;
                deckcraft_engine::cmd::file::add_opened(
                    &mut self.app.session,
                    &name,
                    Some(name.clone()),
                    doc,
                );
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
                if !self.opened {
                    return Err("Open a document first.".into());
                }
                if self.read_only
                    && field(&message, "purpose").as_string().as_deref() != Some("download")
                {
                    return Err("Enable editing before saving.".into());
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("An export request ID is required.")?;
                let format = field(&message, "format")
                    .as_string()
                    .unwrap_or_else(|| extension(&self.name));
                if !matches!(format.as_str(), "pptx") {
                    return Err("This format is supported for import only. Choose a supported export format.".into());
                }
                let doc = self.app.session.doc().map_err(|e| e.to_string())?;
                let bytes = deckcraft_engine::cmd::file::save_bytes(&doc.doc, &format)
                    .map_err(|e| e.to_string())?;
                if bytes.is_empty() || bytes.len() > MAX_BYTES {
                    return Err("The saved document exceeds 50 MiB.".into());
                }
                let revision = self.revision();
                if field(&message, "purpose").as_string().as_deref() != Some("download") {
                    self.last_exported = Some(revision);
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
                        if let Some(doc) = self.app.session.active_mut() {
                            doc.saved_doc = doc.doc.clone();
                        }
                        self.last_exported = None;
                        self.unsaved_new = false;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn revision(&self) -> u64 {
        self.app.session.active().map(|d| d.revision).unwrap_or(0)
    }
    fn report_dirty(&mut self) {
        let dirty = self.unsaved_new || self.app.session.active().is_some_and(|d| d.is_dirty());
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event(
                "dirty",
                json!({"dirty": dirty, "revision": self.revision()}),
            );
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        if self.preview_revision != self.revision() {
            self.textures.clear();
            self.preview_revision = self.revision();
        }
        let Some(state) = self.app.session.active() else {
            return;
        };
        let doc = &state.doc;
        let textures = &mut self.textures;
        let mut visible = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(224)))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width();
                        for index in 0..doc.slides.len() {
                            let scale =
                                ((width - 32.0) / doc.slide_size.width as f32).clamp(0.1, 1.5);
                            let size = egui::vec2(
                                doc.slide_size.width as f32 * scale,
                                doc.slide_size.height as f32 * scale,
                            );
                            ui.add_space(16.0);
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(width.max(size.x + 32.0), size.y),
                                egui::Sense::hover(),
                            );
                            let rect = egui::Rect::from_center_size(row.center(), size);
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            visible.push(index);
                            let pixel_scale = (scale * ui.ctx().pixels_per_point())
                                .min(2048.0 / size.x.max(size.y) * scale);
                            let entry = textures.entry(index).or_insert_with(|| {
                                (
                                    0.0,
                                    ui.ctx().load_texture(
                                        format!("slide-{index}"),
                                        egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                                        Default::default(),
                                    ),
                                )
                            });
                            if (entry.0 - pixel_scale).abs() > 0.01 {
                                let image = deckcraft_render::render_slide(
                                    doc,
                                    index,
                                    &deckcraft_render::RenderOpts {
                                        scale: pixel_scale as f64,
                                        ..Default::default()
                                    },
                                );
                                entry.1.set(
                                    egui::ColorImage::from_rgba_premultiplied(
                                        [image.width as usize, image.height as usize],
                                        &image.pixels,
                                    ),
                                    egui::TextureOptions::LINEAR,
                                );
                                entry.0 = pixel_scale;
                            }
                            ui.painter().image(
                                entry.1.id(),
                                rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }
                    });
            });
        textures.retain(|index, _| visible.contains(index));
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
        if self.opened {
            if !self.read_only {
                self.app.logic(ctx);
            }
        }
        self.report_dirty();
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
        if self.read_only {
            self.app.host_cancel_synthetic();
        }
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

fn create_app(cc: &eframe::CreationContext<'_>) -> deckcraft_ui_egui::SlideApp {
    let mut session = deckcraft_engine::Session::new();
    session.hosted = true;
    session.prefs.author = "Flows Slides User".into();
    let inbox = deckcraft_ui_egui::Inbox::default();
    let picker_inbox = inbox.clone();
    let ctx = cc.egui_ctx.clone();
    let services = deckcraft_ui_egui::Services {
        host_command: Some(Box::new(|id| {
            if matches!(id, "file.save" | "file.saveAs" | "app.saveAsDialog") {
                event("save-request", json!({}));
                return Some(Ok(json!({"pending": "hostSave"})));
            }
            if id.starts_with("file.")
                || matches!(
                    id,
                    "app.home" | "app.openDialog" | "app.exportDialog" | "app.quit"
                )
            {
                return Some(Err("Use the document controls above the editor.".into()));
            }
            None
        })),
        open_async: Some(Box::new(move |purpose, request_id| {
            if !matches!(purpose, "picture" | "audio" | "video") {
                picker_inbox
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(deckcraft_ui_egui::InboxEvent::Cancelled { id: request_id });
                ctx.request_repaint();
                return;
            }
            let inbox = picker_inbox.clone();
            let ctx = ctx.clone();
            let prepare_media = matches!(purpose, "audio" | "video");
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
                    inbox
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(deckcraft_ui_egui::InboxEvent::Cancelled { id: request_id });
                    ctx.request_repaint();
                    return;
                };
                let bytes = file.read().await;
                if bytes.is_empty() || bytes.len() > MAX_BYTES {
                    inbox
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(deckcraft_ui_egui::InboxEvent::Cancelled { id: request_id });
                    ctx.request_repaint();
                } else {
                    let bytes = std::sync::Arc::new(bytes);
                    if prepare_media {
                        let _ = deckcraft_media::browser::prepare(&bytes).await;
                    }
                    let bytes =
                        std::sync::Arc::try_unwrap(bytes).unwrap_or_else(|bytes| (*bytes).clone());
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(
                        deckcraft_ui_egui::InboxEvent::File(deckcraft_ui_egui::InboxFile {
                            id: request_id,
                            name: file.file_name(),
                            bytes,
                        }),
                    );
                    ctx.request_repaint();
                }
            });
        })),
        inbox: Some(inbox),
        ..Default::default()
    };
    let mut app = deckcraft_ui_egui::SlideApp::new(session, services);
    app.ui.ribbon_collapsed = true;
    app
}
