#[path = "live_mcp.rs"]
mod live_mcp;
use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::json;
use wasm_bindgen::prelude::*;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

const MAX_BYTES: usize = 50 * 1024 * 1024;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeWordcraftEmit)]
    fn emit(kind: &str, detail: &str, bytes: &[u8]);
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeWordcraftTake)]
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

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Warn).ok();
    wasm_bindgen_futures::spawn_local(async {
        let result = start_app().await;
        match result {
            Ok(()) => event("ready", json!({})),
            Err(message) => event("error", json!({"error": message})),
        }
    });
}

async fn start_app() -> Result<(), String> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("The document is unavailable.")?;
    let canvas = document
        .get_element_by_id("wordcraft_canvas")
        .ok_or("The document canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid document canvas.")?;
    let options = eframe::WebOptions::default();
    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                let region = wordcraft_ui_egui::i18n::system_region();
                let mut session = Session::new(wordcraft_engine::blank_document(&region));
                session.author = "Flows Word User".into();
                let inbox = wordcraft_ui_egui::Inbox::default();
                let open_inbox = inbox.clone();
                let ctx = cc.egui_ctx.clone();
                let services = Services {
                    host_command: Some(Box::new(|id| {
                        if matches!(id, "file.save" | "file.saveAs") {
                            event("save-request", json!({}));
                            return Some(Ok(json!({"pending": "hostSave"})));
                        }
                        if id.starts_with("file.")
                            && !matches!(
                                id,
                                "file.properties"
                                    | "file.info"
                                    | "file.setAuthor"
                                    | "file.accessibility"
                                    | "file.printMarkup"
                            )
                            || matches!(
                                id,
                                "ui.backstage"
                                    | "ui.openFileDialog"
                                    | "ui.saveAs"
                                    | "ui.export"
                                    | "ui.discord"
                            )
                        {
                            return Some(Err("Use the document controls above the editor.".into()));
                        }
                        None
                    })),
                    open_async: Some(Box::new(move |purpose| {
                        if purpose != "picture" {
                            return;
                        }
                        let inbox = open_inbox.clone();
                        let ctx = ctx.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .add_filter(
                                    "Pictures",
                                    &["png", "jpg", "jpeg", "gif", "webp", "bmp"],
                                )
                                .pick_file()
                                .await
                            else {
                                return;
                            };
                            let bytes = file.read().await;
                            if bytes.len() <= MAX_BYTES {
                                inbox
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .push((file.file_name(), bytes));
                                ctx.request_repaint();
                            }
                        });
                    })),
                    inbox: Some(inbox),
                    ..Default::default()
                };
                let mut app = WordApp::new(session, services);
                app.autosave = false;
                app.session.autosave = false;
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    name: "document.docx".into(),
                    reported_dirty: false,
                    unsaved_new: false,
                    last_exported: None,
                    preview_revision: 0,
                    textures: HashMap::new(),
                }))
            }),
        )
        .await
        .map_err(|e| format!("The document editor could not start: {e:?}"))
}

struct Host {
    app: WordApp,
    opened: bool,
    read_only: bool,
    name: String,
    reported_dirty: bool,
    unsaved_new: bool,
    last_exported: Option<u64>,
    preview_revision: u64,
    textures: HashMap<usize, (f32, egui::TextureHandle)>,
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
                            wordcraft_ui_egui::ControlRequest::new(method, params);
                        match wordcraft_ui_egui::control::handle(&mut self.app, &ctx, &request) {
                            wordcraft_ui_egui::control::Outcome::Done(reply) => {
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
                if !name.to_ascii_lowercase().ends_with(".docx") {
                    return Err("New documents use the .docx format.".into());
                }
                let region = wordcraft_ui_egui::i18n::system_region();
                self.app
                    .session
                    .set_document(wordcraft_engine::blank_document(&region));
                self.app.session.path = Some(name.clone().into());
                self.app.session.autosave = false;
                self.app.ui.backstage = false;
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                self.unsaved_new = true;
                event(
                    "opened",
                    json!({"revision": self.app.session.rev(), "readOnly": self.read_only}),
                );
            }
            Some("open") => {
                if self.opened {
                    return Err("This editor already has a document open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A document name is required.")?;
                if !name.to_ascii_lowercase().ends_with(".docx") {
                    return Err("Only DOCX documents are supported.".into());
                }
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("Document bytes are required.".into());
                }
                let bytes = js_sys::Uint8Array::new(&data);
                if bytes.length() == 0 || bytes.length() as usize > MAX_BYTES {
                    return Err("Documents must be between 1 byte and 50 MiB.".into());
                }
                let doc = wordcraft_engine::io::open_bytes(&name, &bytes.to_vec())?;
                self.app.session.set_document(doc);
                self.app.session.path = Some(name.clone().into());
                self.app.session.autosave = false;
                self.app.ui.backstage = false;
                self.name = name;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"revision": self.app.session.rev(), "readOnly": self.read_only}),
                );
            }
            Some("set-read-only") => {
                if !self.opened {
                    return Err("Open a document first.".into());
                }
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.app.dialog = None;
                self.app.ui.backstage = false;
                self.app.canvas.want_focus = !self.read_only;
                // Preview always preserves printed page layout, independent of the editor view.
                let _ = self.app.session.run("view.printLayout", &json!({}));
                self.app.session.relayout();
            }
            Some("export") => {
                if !self.opened
                    || (self.read_only
                        && field(&message, "purpose").as_string().as_deref() != Some("download"))
                {
                    return Err("Enable editing before saving.".into());
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("A save request identifier is required.")?;
                if !self.read_only {
                    self.app.host_flush_drafts()?;
                }
                let mut copy = Session::new(self.app.session.doc.clone());
                copy.author = self.app.session.author.clone();
                copy.stamp_save();
                let bytes = wordcraft_engine::io::save_bytes("document.docx", &copy.doc)?;
                if bytes.len() > MAX_BYTES {
                    return Err("The exported document exceeds 50 MiB.".into());
                }
                let revision = self.app.session.rev();
                if field(&message, "purpose").as_string().as_deref() != Some("download") {
                    self.last_exported = Some(revision);
                }
                emit(
                    "exported",
                    &json!({"requestId": request, "name": self.name, "revision": revision})
                        .to_string(),
                    &bytes,
                );
            }
            Some("saved") => {
                if let Some(revision) = field(&message, "revision").as_f64() {
                    if !self.app.host_has_drafts()
                        && self.last_exported == Some(revision as u64)
                        && self.app.session.rev() as f64 == revision
                    {
                        self.app.session.dirty = false;
                        self.last_exported = None;
                        self.unsaved_new = false;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn report_dirty(&mut self) {
        let dirty = self.unsaved_new || self.app.session.dirty || self.app.host_has_drafts();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event(
                "dirty",
                json!({"dirty": dirty, "revision": self.app.session.rev()}),
            );
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        if self.preview_revision != self.app.session.rev() {
            self.textures.clear();
            self.preview_revision = self.app.session.rev();
        }
        let layout = self.app.session.layout();
        let doc = &self.app.session.doc;
        let textures = &mut self.textures;
        let mut visible = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(224)))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let viewport_width = ui.available_width();
                        for (index, page) in layout.pages.iter().enumerate() {
                            let scale =
                                ((viewport_width - 32.0) / page.w.max(1.0)).clamp(0.25, 1.5);
                            let size = egui::vec2(page.w * scale, page.h * scale);
                            ui.add_space(16.0);
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(viewport_width.max(size.x + 32.0), size.y),
                                egui::Sense::hover(),
                            );
                            let rect = egui::Rect::from_center_size(row.center(), size);
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            visible.push(index);
                            let pixel_scale = (scale * ui.ctx().pixels_per_point())
                                .min(2048.0 / page.w.max(page.h).max(1.0));
                            let entry = textures.entry(index).or_insert_with(|| {
                                (
                                    0.0,
                                    ui.ctx().load_texture(
                                        format!("page-{index}"),
                                        egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                                        Default::default(),
                                    ),
                                )
                            });
                            if (entry.0 - pixel_scale).abs() > 0.01 {
                                let image = wordcraft_render::render_page(
                                    doc,
                                    page,
                                    pixel_scale,
                                    &Default::default(),
                                );
                                let color = egui::ColorImage::from_rgba_premultiplied(
                                    [image.width as usize, image.height as usize],
                                    &image.pixels,
                                );
                                entry.1.set(color, egui::TextureOptions::LINEAR);
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
            if let Err(message) = self.receive(message) {
                event("error", json!({"requestId": request_id, "error": message}));
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
        if self.read_only {
            raw.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut
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
