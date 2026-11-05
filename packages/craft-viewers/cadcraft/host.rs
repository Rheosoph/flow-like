#[path = "live_mcp.rs"]
mod live_mcp;
use cadcraft_engine::Session;
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::{Value, json};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use web_sys::js_sys;

const MAX_BYTES: usize = 100 * 1024 * 1024;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftEmit)]
    fn emit(kind: &str, detail: &str, bytes: &[u8]);
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftTake)]
    fn take() -> JsValue;
}

thread_local! { static CONTEXT: RefCell<Option<egui::Context>> = const { RefCell::new(None) }; }

#[wasm_bindgen]
pub fn request_repaint() {
    CONTEXT.with(|ctx| {
        if let Some(ctx) = ctx.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}
fn event(kind: &str, detail: Value) {
    emit(kind, &detail.to_string(), &[]);
}
fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

fn host_command(id: &str) -> Option<Result<Value, String>> {
    if matches!(id, "qsave" | "save" | "saveas" | "ui.saveas") {
        event("save-request", json!({}));
        return Some(Ok(json!({"pending": "hostSave"})));
    }
    if matches!(
        id,
        "new"
            | "qnew"
            | "open"
            | "close"
            | "closeall"
            | "ui.open"
            | "ui.newtemplate"
            | "ui.sample"
            | "ui.start"
            | "ui.quit"
            | "ui.dialog.about"
            | "document.switch"
            | "recover"
            | "export"
            | "exportpdf"
            | "plot"
            | "wblock"
    ) {
        return Some(Err("Use the drawing controls above the editor.".into()));
    }
    None
}

pub fn start() {
    cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|e| e.to_string()),
        write: |doc, name| cadcraft_io::write(doc, name).map_err(|e| e.to_string()),
        plot: Some(|doc, space, opts| {
            cadcraft_io::plotter::plot(doc, space, opts).map_err(|e| e.to_string())
        }),
    });
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
        .ok_or("The drawing canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid drawing canvas.")?;
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                let mut session = Session::empty();
                session.host_command = Some(host_command);
                let mut app = CadApp::new(session, Services::default());
                if let Some(rs) = &cc.wgpu_render_state {
                    app.set_wgpu(rs, None);
                }
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    reported_dirty: false,
                    unsaved_new: false,
                    last_exported: None,
                }))
            }),
        )
        .await
        .map_err(|e| format!("The drawing editor could not start: {e:?}"))?;
    std::mem::forget(runner);
    Ok(())
}

struct Host {
    app: CadApp,
    opened: bool,
    read_only: bool,
    reported_dirty: bool,
    unsaved_new: bool,
    last_exported: Option<u64>,
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
                            cadcraft_ui_egui::ControlRequest::new(method, params);
                        match cadcraft_ui_egui::control::handle(&mut self.app, &ctx, &request) {
                            cadcraft_ui_egui::control::Outcome::Done(reply) => {
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
                if !name.to_ascii_lowercase().ends_with(".dxf") {
                    return Err("New documents use the .dxf format.".into());
                }
                self.app.session.new_drawing(false);
                self.app.ui.start_tab = false;
                self.app.canvas.zoom_pending = true;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                self.unsaved_new = true;
                event(
                    "opened",
                    json!({"revision": self.app.session.state().map(|state| state.revision).unwrap_or(0), "readOnly": self.read_only}),
                );
            }
            Some("open") => {
                if self.opened {
                    return Err("A drawing is already open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A drawing name is required.")?;
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("Drawing bytes are required.".into());
                }
                let data = js_sys::Uint8Array::new(&data);
                if data.length() == 0 || data.length() as usize > MAX_BYTES {
                    return Err("Drawings must be between 1 byte and 100 MiB.".into());
                }
                let bytes = data.to_vec();
                let doc = cadcraft_io::read(&bytes, &name).map_err(|e| e.to_string())?;
                let warnings = doc.read_notes.clone();
                self.app
                    .session
                    .open_drawing(doc, &name, Some(name.clone()));
                if let Ok(state) = self.app.session.state_mut() {
                    state.binary_dxf = bytes.starts_with(b"AutoCAD Binary DXF");
                }
                self.app.ui.start_tab = false;
                self.app.canvas.zoom_pending = true;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"revision": self.app.session.state().map(|s| s.revision).unwrap_or(0), "readOnly": self.read_only, "warnings": warnings}),
                );
            }
            Some("set-read-only") => {
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                if self.read_only {
                    self.app.session.cancel();
                }
                self.app.ui.dialog = None;
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
                let format = field(&message, "format")
                    .as_string()
                    .ok_or("An export format is required.")?;
                if format != "dxf" {
                    return Err(
                        "Drawing exports use DXF. DWG requires the native CAD application.".into(),
                    );
                }
                let state = self.app.session.state().map_err(|e| e.to_string())?;
                let bytes = cadcraft_io::write_with(
                    &state.doc,
                    &format!("drawing.{format}"),
                    &cadcraft_io::SaveOptions {
                        version: None,
                        binary: format == "dxf" && state.binary_dxf,
                    },
                )
                .map_err(|e| e.to_string())?;
                if bytes.len() > MAX_BYTES {
                    return Err("The exported drawing exceeds 100 MiB.".into());
                }
                let revision = state.revision;
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
                    if let Ok(state) = self.app.session.state_mut() {
                        if self.last_exported == Some(revision as u64)
                            && state.revision as f64 == revision
                        {
                            state.saved = state.doc.clone();
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
}
impl eframe::App for Host {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
        if self.read_only {
            self.app.host_cancel_synthetic();
        }
        if self.read_only {
            raw.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                )
            });
        }
        self.app.raw_input_hook(ctx, raw);
        if !self.read_only {
            cadcraft_ui_egui::cmdline::capture_tab(ctx, raw);
        }
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        loop {
            let message = take();
            if message.is_null() || message.is_undefined() {
                break;
            }
            let request = field(&message, "requestId").as_string();
            if let Err(error) = self.receive(message) {
                event("error", json!({"error": error, "requestId": request}));
            }
        }
        self.app.logic(ctx);
        let dirty = self.unsaved_new || self.app.session.docs.iter().any(|doc| doc.is_dirty());
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event("dirty", json!({"dirty": dirty}));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.read_only {
            ui.horizontal(|ui| {
                ui.label("Flows CAD preview");
                if ui.button("Fit drawing").clicked() {
                    self.app.canvas.zoom_pending = true;
                }
                if ui.button("Zoom in").clicked() {
                    let _ = self
                        .app
                        .session
                        .execute("zoom", &json!({"mode": "scale", "factor": 1.5}));
                }
                if ui.button("Zoom out").clicked() {
                    let _ = self
                        .app
                        .session
                        .execute("zoom", &json!({"mode": "scale", "factor": 1.0 / 1.5}));
                }
            });
            ui.ctx().input_mut(|input| {
                input.events.clear();
                input.pointer = Default::default();
            });
        }
        ui.add_enabled_ui(self.opened && !self.read_only, |ui| self.app.ui(ui));
    }
}
