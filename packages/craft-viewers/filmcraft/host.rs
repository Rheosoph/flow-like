#[path = "live_mcp.rs"]
mod live_mcp;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 200 * 1024 * 1024;
thread_local! { static OPEN_RESULT: RefCell<Option<Result<(), String>>> = const { RefCell::new(None) }; }

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftEmit)]
    fn emit(kind: &str, detail: &str, bytes: &[u8]);
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftTake)]
    fn take() -> JsValue;
}

#[wasm_bindgen]
pub fn request_repaint() {
    crate::repaint();
}

fn event(kind: &str, detail: Value) {
    emit(kind, &detail.to_string(), &[]);
}
fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

fn host_command(id: &str) -> Option<Result<Value, String>> {
    if matches!(
        id,
        "file.save" | "file.saveAs" | "file.exportMedia" | "export.quick"
    ) {
        event("save-request", json!({}));
        return Some(Ok(json!({"pending": "hostSave"})));
    }
    if id.starts_with("help.")
        || id == "app.about"
        || id == "app.quit"
        || id.starts_with("export.")
            && !matches!(
                id,
                "export.resolve" | "export.presets.list" | "export.presets.get"
            )
        || id.starts_with("sequence.render") && id != "sequence.renderBar"
        || id.starts_with("file.")
            && !matches!(
                id,
                "file.newSequence"
                    | "file.newBin"
                    | "file.newAdjustmentLayer"
                    | "file.newColorMatte"
                    | "file.newBlackVideo"
                    | "file.newTransparentVideo"
                    | "file.newBarsAndTone"
                    | "file.newCountingLeader"
            )
    {
        return Some(Err("Use the file controls above the editor.".into()));
    }
    None
}

#[wasm_bindgen(start)]
pub fn start_host() {
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
        .ok_or("The video canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid video canvas.")?;
    crate::webcodecs::probe().await;
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                filmcraft_ui_egui::install_ime_reset_callbacks(&cc.egui_ctx);
                crate::set_context(cc.egui_ctx.clone());
                let ctx = cc.egui_ctx.clone();
                crate::fs::set_on_data(move || ctx.request_repaint());
                crate::webcodecs::install();
                crate::browser_export::install();
                let mut session = Session::new(Arc::new(crate::fs::WebServices));
                session.host_command = Some(host_command);
                let mut app = FilmcraftApp::new(session);
                app.ui.mode = filmcraft_ui_egui::state::Mode::Edit;
                if let Some(rs) = cc.wgpu_render_state.clone() {
                    if rs.adapter.get_info().backend == eframe::wgpu::Backend::BrowserWebGpu {
                        app.set_wgpu(rs);
                    }
                }
                let audio = crate::audio::Handle::new();
                if let Some(audio) = &audio {
                    app.audio = Some(Box::new(audio.clone()));
                }
                Ok(Box::new(Host {
                    web: crate::WebApp {
                        app,
                        audio,
                        autosave: crate::recovery::Autosave::default(),
                        host: None,
                    },
                    opened: false,
                    read_only: true,
                    reported_dirty: false,
                    last_exported: None,
                    pending: None,
                    opening: None,
                }))
            }),
        )
        .await
        .map_err(|e| format!("The video editor could not start: {e:?}"))?;
    std::mem::forget(runner);
    Ok(())
}

struct Pending {
    request: String,
    revision: u64,
    download: bool,
}
struct Opening {
    name: String,
    path: String,
    read_only: bool,
}
struct Host {
    web: crate::WebApp,
    opened: bool,
    read_only: bool,
    reported_dirty: bool,
    last_exported: Option<u64>,
    pending: Option<Pending>,
    opening: Option<Opening>,
}

impl Host {
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let hook = self.web.app.session.host_command.take();
        let result = self
            .web
            .app
            .session
            .execute(id, params)
            .map_err(|e| e.to_string());
        self.web.app.session.host_command = hook;
        result
    }

    fn receive(&mut self, message: JsValue) -> Result<(), String> {
        match field(&message, "type").as_string().as_deref() {
            Some("mcp-request") => {
                if !self.opened {
                    return Err("Open a document first.".into());
                }
                let ctx = crate::CTX
                    .with(|context| context.borrow().clone())
                    .ok_or("The editor is unavailable.")?;
                let detail = live_mcp::receive(&message, self.read_only, &mut |method, params| {
                    let (request, _) =
                        filmcraft_ui_egui::control::ControlRequest::new(method, params);
                    match filmcraft_ui_egui::control::handle(&mut self.web.app, &ctx, &request) {
                        filmcraft_ui_egui::control::Outcome::Done(reply) => {
                            live_mcp::unwrap_control(reply)
                        }
                        _ => Err("This operation needs a native editor window.".into()),
                    }
                })
                .ok_or("Invalid MCP request.")?;
                event("mcp-response", detail);
            }
            Some("create") => {
                if self.opened || self.opening.is_some() {
                    return Err("A video project is already open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A project name is required.")?;
                if !name.to_ascii_lowercase().ends_with(".fcproj") {
                    return Err("New video projects use the .fcproj format.".into());
                }
                self.execute(
                    "file.newProject",
                    json!({"name": name.trim_end_matches(".fcproj")}),
                )?;
                self.execute("file.newSequence", json!({"name": "Sequence 1"}))?;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                event(
                    "opened",
                    json!({"revision": self.web.app.session.revision, "readOnly": self.read_only}),
                );
            }
            Some("open") => {
                if self.opened || self.opening.is_some() {
                    return Err("A video file is already open.".into());
                }
                let name = field(&message, "name")
                    .as_string()
                    .ok_or("A file name is required.")?;
                let data = field(&message, "bytes");
                if !data.is_instance_of::<js_sys::ArrayBuffer>() {
                    return Err("File bytes are required.".into());
                }
                let data = js_sys::Uint8Array::new(&data);
                if data.length() == 0 || data.length() as usize > MAX_BYTES {
                    return Err("Video files must be between 1 byte and 200 MiB.".into());
                }
                let name: String = name
                    .chars()
                    .map(|c| if c == '/' || c == '\\' { '_' } else { c })
                    .collect();
                let path = format!("/files/{name}");
                let bytes = data.to_vec();
                crate::fs::put(&path, bytes.clone());
                if name.to_ascii_lowercase().ends_with(".fcproj") {
                    let result = self.execute("file.open", json!({"path": path}))?;
                    if result["missingMedia"]
                        .as_u64()
                        .is_some_and(|count| count > 0)
                    {
                        return Err("This video project references separate media files. Open a self-contained project or a rendered video.".into());
                    }
                    self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                    self.opened = true;
                    event(
                        "opened",
                        json!({"revision": self.web.app.session.revision, "readOnly": self.read_only}),
                    );
                    return Ok(());
                }
                self.opening = Some(Opening {
                    name: name.clone(),
                    path,
                    read_only: field(&message, "readOnly").as_bool().unwrap_or(true),
                });
                wasm_bindgen_futures::spawn_local(async move {
                    let result = crate::webcodecs::prepare(&name, &bytes).await;
                    OPEN_RESULT.with(|slot| *slot.borrow_mut() = Some(result));
                    crate::repaint();
                });
            }
            Some("set-read-only") => {
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true)
            }
            Some("export") => {
                let download =
                    field(&message, "purpose").as_string().as_deref() == Some("download");
                if !self.opened || (self.read_only && !download) {
                    return Err("Enable editing before saving.".into());
                }
                if self.pending.is_some() {
                    return Err("A video export is already in progress.".into());
                }
                let request = field(&message, "requestId")
                    .as_string()
                    .ok_or("A save request identifier is required.")?;
                self.web.app.stop();
                let revision = self.web.app.session.revision;
                let session = &self.web.app.session;
                if field(&message, "format").as_string().as_deref() == Some("fcproj") {
                    let bytes = filmcraft_format::encode_with_view(
                        &session.project,
                        Some(&session.project_view()),
                        false,
                    );
                    if bytes.is_empty() || bytes.len() > MAX_BYTES {
                        return Err("The exported project is empty or exceeds 200 MiB.".into());
                    }
                    if !download {
                        self.last_exported = Some(revision);
                    }
                    emit(
                        "exported",
                        &json!({"requestId": request, "revision": revision}).to_string(),
                        &bytes,
                    );
                    return Ok(());
                }
                if field(&message, "format").as_string().as_deref() != Some("mp4") {
                    return Err(
                        "Video exports use MP4 or the native .fcproj project format.".into(),
                    );
                }
                let sequence = session
                    .state
                    .active_sequence
                    .ok_or("There is no active video sequence.")?;
                let progress_request = request.clone();
                crate::browser_export::start(
                    session.project.clone(),
                    sequence,
                    session.media.clone(),
                    session.services.clone(),
                    move |progress, stage| {
                        event(
                            "export-progress",
                            json!({"requestId": progress_request, "progress": progress, "stage": stage}),
                        );
                    },
                );
                self.pending = Some(Pending {
                    request,
                    revision,
                    download,
                });
            }
            Some("saved") => {
                if let Some(revision) = field(&message, "revision").as_f64() {
                    if self.last_exported == Some(revision as u64)
                        && self.web.app.session.revision as f64 == revision
                    {
                        self.web.app.session.saved_revision = revision as u64;
                        self.last_exported = None;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn poll_export(&mut self) {
        if self.pending.is_none() {
            return;
        }
        let Some(bytes) = crate::browser_export::take_result() else {
            return;
        };
        let Some(pending) = self.pending.take() else {
            return;
        };
        match bytes {
            Ok(bytes) if bytes.len() <= MAX_BYTES && !bytes.is_empty() => {
                if !pending.download {
                    self.last_exported = Some(pending.revision);
                }
                emit(
                    "exported",
                    &json!({"requestId": pending.request, "revision": pending.revision})
                        .to_string(),
                    &bytes,
                );
            }
            Ok(_) => event(
                "error",
                json!({"error": "The exported video is empty or exceeds 200 MiB.", "requestId": pending.request}),
            ),
            Err(error) => event(
                "error",
                json!({"error": error, "requestId": pending.request}),
            ),
        }
    }

    fn poll_open(&mut self) -> Result<(), String> {
        let Some(result) = OPEN_RESULT.with(|slot| slot.borrow_mut().take()) else {
            return Ok(());
        };
        let Some(opening) = self.opening.take() else {
            return Ok(());
        };
        result?;
        let imported = self.execute("file.import", json!({"paths": [opening.path]}))?;
        if let Some(errors) = imported
            .get("errors")
            .and_then(Value::as_array)
            .filter(|errors| !errors.is_empty())
        {
            return Err(errors
                .iter()
                .map(|e| e.as_str().unwrap_or("Video import failed."))
                .collect::<Vec<_>>()
                .join("; "));
        }
        let item = imported
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_u64)
            .ok_or("The video has no supported media stream.")?;
        self.execute(
            "file.newSequence",
            json!({"name": opening.name, "fromItem": item}),
        )?;
        self.web.app.session.saved_revision = self.web.app.session.revision;
        self.read_only = opening.read_only;
        self.opened = true;
        event(
            "opened",
            json!({"revision": self.web.app.session.revision, "readOnly": self.read_only}),
        );
        Ok(())
    }
}

impl eframe::App for Host {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.dropped_files.clear();
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
        self.web.raw_input_hook(ctx, raw);
    }
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
        self.web.logic(ctx, frame);
        if let Err(error) = self.poll_open() {
            event("error", json!({"error": error}));
        }
        self.poll_export();
        let dirty = self.opened && self.web.app.session.is_dirty();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event(
                "dirty",
                json!({"dirty": dirty, "revision": self.web.app.session.revision}),
            );
        }
        if self.pending.is_some() {
            ctx.request_repaint();
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.read_only {
            ui.horizontal(|ui| {
                ui.label("Flows Video preview");
                if ui
                    .button(if self.web.app.playback.playing {
                        "Stop"
                    } else {
                        "Play"
                    })
                    .clicked()
                {
                    self.web.app.toggle_play(1.0);
                }
            });
            ui.ctx().input_mut(|input| {
                input.events.clear();
                input.pointer = Default::default();
            });
        }
        ui.add_enabled_ui(self.opened && !self.read_only, |ui| self.web.ui(ui, frame));
    }
}
