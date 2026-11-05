#[path = "live_mcp.rs"]
mod live_mcp;
use js_sys::{ArrayBuffer, Reflect, Uint8Array};
use serde_json::{Value, json};
use solidcraft_engine::{Session, io};
use solidcraft_ui_egui::{Services, SolveApp};
use std::cell::{Cell, RefCell};
use std::sync::mpsc::{Receiver, TryRecvError};
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 100 * 1024 * 1024;
const EXTENSIONS: &[&str] = &[
    "solidcraft",
    "fcstd",
    "3dm",
    "step",
    "stp",
    "iges",
    "igs",
    "stl",
    "obj",
    "3mf",
    "glb",
    "gltf",
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
    static READ_ONLY: Cell<bool> = const { Cell::new(true) };
}

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
    Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

fn host_command(id: &str) -> Option<Result<Value, String>> {
    let id = solidcraft_engine::find_command(id).map_or(id, |spec| spec.id);
    if READ_ONLY.get() && !live_mcp::policy::command_read_only(id) {
        return Some(Err("Enable editing before changing this model.".into()));
    }
    if matches!(id, "file.save" | "file.save_as") {
        event("save-request", json!({}));
        return Some(Ok(json!({"pending": "hostSave"})));
    }
    if id.starts_with("file.")
        || id.starts_with("doc.recover")
        || matches!(id, "doc.open" | "host.close")
        || id.contains("export")
        || id.starts_with("script.")
    {
        return Some(Err(
            "Use the file controls above the editor. Models save as .solidcraft files.".into(),
        ));
    }
    None
}

// A missing glTF companion can otherwise produce a successful partial import.
fn check_gltf(bytes: &[u8], extension: &str) -> Result<(), String> {
    let json_bytes = if extension == "gltf" {
        bytes
    } else if extension == "glb" {
        if bytes.len() < 20 || &bytes[..4] != b"glTF" {
            return Err("Invalid GLB header.".into());
        }
        let word = |offset: usize| {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default()) as usize
        };
        if word(4) != 2 || word(8) != bytes.len() || &bytes[16..20] != b"JSON" {
            return Err("Invalid GLB container.".into());
        }
        bytes
            .get(20..20_usize.saturating_add(word(12)))
            .ok_or("Invalid GLB JSON chunk.")?
    } else {
        return Ok(());
    };
    let model: Value =
        serde_json::from_slice(json_bytes).map_err(|e| format!("Invalid glTF: {e}"))?;
    if model
        .get("buffers")
        .and_then(Value::as_array)
        .is_some_and(|buffers| {
            buffers.iter().any(|buffer| {
                buffer
                    .get("uri")
                    .and_then(Value::as_str)
                    .is_some_and(|uri| !uri.starts_with("data:"))
            })
        })
    {
        return Err("This model references a separate buffer file. Export a GLB with embedded buffers or a self-contained glTF and upload it again.".into());
    }
    Ok(())
}

pub fn start() {
    solidcraft_engine::set_host_command(Some(host_command));
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
        .ok_or("The model canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid model canvas.")?;
    let runner = eframe::WebRunner::new();
    let mut options = eframe::WebOptions {
        depth_buffer: 32,
        ..Default::default()
    };
    if web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.contains("webgl"))
        && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) =
            &mut options.wgpu_options.wgpu_setup
    {
        create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
    }
    runner
        .start(
            canvas,
            options,
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                solidcraft_ui_egui::fonts::setup_fonts(&cc.egui_ctx);
                let mut app = SolveApp::new(Session::default(), Services::default());
                if let Some(rs) = &cc.wgpu_render_state {
                    app.set_wgpu(rs, 32, 0, 1);
                }
                app.home.open = false;
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    reported_dirty: false,
                    last_exported: None,
                    unsaved_new: false,
                    pending_open: None,
                }))
            }),
        )
        .await
        .map_err(|e| {
            format!("The model editor could not start. WebGPU or WebGL2 is required: {e:?}")
        })?;
    std::mem::forget(runner);
    Ok(())
}

struct Host {
    app: SolveApp,
    opened: bool,
    read_only: bool,
    reported_dirty: bool,
    last_exported: Option<u64>,
    unsaved_new: bool,
    pending_open: Option<PendingOpen>,
}

struct PendingOpen {
    path: String,
    extension: String,
    request_id: Option<String>,
    reply: Receiver<Value>,
    progress: Option<(usize, usize)>,
    reported_at: f64,
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
                            solidcraft_ui_egui::ControlRequest::new(method, params);
                        match solidcraft_ui_egui::control::handle(&mut self.app, &ctx, &request) {
                            solidcraft_ui_egui::control::Outcome::Done(reply) => {
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
                if self.opened || self.pending_open.is_some() {
                    return Err("A model is already open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A model name is required.")?;
                let basename = name.rsplit(['/', '\\']).next().unwrap_or("");
                let (title, extension) = basename
                    .rsplit_once('.')
                    .ok_or("Use a .solidcraft model name.")?;
                if title.trim().is_empty() || !extension.eq_ignore_ascii_case("solidcraft") {
                    return Err("New models use the .solidcraft format.".into());
                }
                self.app
                    .session
                    .replace_design(Session::new(solidcraft_engine::doc::Document::new(title)));
                self.app.home.open = false;
                self.app.fit_view();
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                READ_ONLY.set(self.read_only);
                self.opened = true;
                self.unsaved_new = true;
                self.reported_dirty = true;
                event(
                    "opened",
                    json!({"revision": self.app.session.revision, "readOnly": self.read_only, "warnings": []}),
                );
                event("dirty", json!({"dirty": true}));
            }
            Some("open") => {
                if self.opened || self.pending_open.is_some() {
                    return Err("A model is already open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A model name is required.")?;
                let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                if !EXTENSIONS.contains(&extension.as_str()) {
                    return Err("This model format is not supported.".into());
                }
                let data = field(&message, "bytes");
                if !data.is_instance_of::<ArrayBuffer>() {
                    return Err("Model bytes are required.".into());
                }
                let data = Uint8Array::new(&data);
                if data.length() == 0 || data.length() as usize > MAX_BYTES {
                    return Err("Models must be between 1 byte and 100 MiB.".into());
                }
                let bytes = data.to_vec();
                check_gltf(&bytes, &extension)?;
                let basename = name.rsplit(['/', '\\']).next().unwrap_or("model");
                let path = format!("/host/{basename}");
                io::vfs::insert(&path, bytes);
                if matches!(extension.as_str(), "solidcraft" | "step" | "stp") {
                    solidcraft_engine::set_host_command(None);
                    let result = self.app.host_open_staged(&path);
                    solidcraft_engine::set_host_command(Some(host_command));
                    let reply = match result {
                        Ok(reply) => reply,
                        Err(error) => {
                            let _ = io::vfs::remove(&path);
                            return Err(error);
                        }
                    };
                    self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                    self.pending_open = Some(PendingOpen {
                        path,
                        extension,
                        request_id: field(&message, "requestId").as_string(),
                        reply,
                        progress: None,
                        reported_at: 0.0,
                    });
                    return Ok(());
                }
                solidcraft_engine::set_host_command(None);
                let result = self.app.session.execute("doc.open", &json!({"path": path}));
                solidcraft_engine::set_host_command(Some(host_command));
                let _ = io::vfs::remove(&path);
                let result = result.map_err(|e| e.to_string())?;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.finish_open(result, &extension);
            }
            Some("set-read-only") => {
                let read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                if read_only && !self.read_only {
                    solidcraft_ui_egui::dialogs::cancel(&mut self.app);
                    if self.app.session.active_sketch.is_some() {
                        self.app.finish_sketch();
                    }
                    self.app.preview.reset();
                    solidcraft_ui_egui::dim_view::cancel();
                    self.app.tool = None;
                    self.app.params = None;
                    self.app.design_properties = None;
                    self.app.menu = Default::default();
                    self.app.ui.palette_open = false;
                    self.app.viewport.boxsel = None;
                    self.app.viewport.point_drag = None;
                }
                self.read_only = read_only;
                READ_ONLY.set(read_only || !self.opened);
            }
            Some("export") => {
                let download =
                    field(&message, "purpose").as_string().as_deref() == Some("download");
                if !self.opened || (self.read_only && !download) {
                    return Err("Enable editing before saving.".into());
                }
                if !self.app.session.pending_moves.is_empty() {
                    return Err(
                        "Capture or revert component positions in the editor before saving.".into(),
                    );
                }
                if self.app.dialog.is_some() || self.app.applying.is_some() {
                    return Err(
                        "Finish or cancel the current operation in the editor before saving."
                            .into(),
                    );
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("A save request identifier is required.")?;
                if field(&message, "format").as_string().as_deref() != Some("solidcraft") {
                    return Err(
                        "Models save as .solidcraft files to preserve their editing history."
                            .into(),
                    );
                }
                let bytes = self
                    .app
                    .session
                    .design_container(io::DesignCompression::Fast, false, false)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > MAX_BYTES {
                    return Err("The exported model exceeds 100 MiB.".into());
                }
                let revision = self.app.session.revision;
                if !download {
                    self.last_exported = Some(revision);
                }
                emit(
                    "exported",
                    &json!({"requestId": request, "revision": revision}).to_string(),
                    &bytes,
                );
            }
            Some("saved") => {
                if let Some(revision) = field(&message, "revision").as_f64()
                    && self.last_exported == Some(revision as u64)
                    && self.app.session.revision as f64 == revision
                {
                    self.app.session.mark_saved();
                    self.last_exported = None;
                    self.unsaved_new = false;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish_open(&mut self, result: Value, extension: &str) {
        let mut warnings: Vec<String> = result
            .get("warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if matches!(extension, "fcstd" | "3dm") {
            warnings.insert(0, "Imported geometry does not include the original design history. Edits save as a .solidcraft model.".into());
        }
        if result.get("errors").and_then(Value::as_u64).unwrap_or(0) > 0 {
            warnings.push(
                "Some features could not rebuild. Review the model's timeline before saving."
                    .into(),
            );
        }
        self.app.session.mark_saved();
        self.app.home.open = false;
        self.app.fit_view();
        self.opened = true;
        READ_ONLY.set(self.read_only);
        event(
            "opened",
            json!({"revision": self.app.session.revision, "readOnly": self.read_only, "warnings": warnings}),
        );
    }

    fn poll_open(&mut self, ctx: &egui::Context) {
        let Some(pending) = &mut self.pending_open else {
            return;
        };
        // Only this host-owned loader can temporarily execute file commands.
        solidcraft_engine::set_host_command(None);
        let progress = self.app.host_tick_load(ctx);
        solidcraft_engine::set_host_command(Some(host_command));
        let result = match pending.reply.try_recv() {
            Ok(reply) => Some(live_mcp::unwrap_control(reply)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("The model load stopped.".into())),
        };
        if let Some(result) = result {
            let pending = self.pending_open.take().unwrap();
            let _ = io::vfs::remove(&pending.path);
            match result {
                Ok(result) => self.finish_open(result, &pending.extension),
                Err(error) => event(
                    "error",
                    json!({"error": error, "requestId": pending.request_id}),
                ),
            }
        } else {
            let now = ctx.input(|input| input.time);
            if pending.progress != Some(progress) || now - pending.reported_at >= 1.0 {
                pending.progress = Some(progress);
                pending.reported_at = now;
                event(
                    "loading",
                    json!({"interactive": true, "completed": progress.0.min(progress.1), "total": progress.1}),
                );
            }
            ctx.request_repaint();
        }
    }
}

impl eframe::App for Host {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
        if self.read_only || !self.opened {
            self.app.host_cancel_synthetic();
        }
        raw.hovered_files.clear();
        if self.read_only || !self.opened {
            raw.events.retain(|event| {
                if self.pending_open.is_some()
                    && matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            ..
                        }
                    )
                {
                    return true;
                }
                !matches!(
                    event,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                )
            });
        } else {
            self.app.raw_input_hook(raw);
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
        self.app.home.open = false;
        self.poll_open(ctx);
        self.app.logic(ctx);
        let dirty = self.unsaved_new
            || self.app.session.is_dirty()
            || !self.app.session.pending_moves.is_empty();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event("dirty", json!({"dirty": dirty}));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.pending_open.is_some() {
            self.app.host_loading(ui);
        } else if self.read_only || !self.opened {
            self.app.host_preview(ui);
        } else {
            self.app.ui(ui);
        }
    }
}
