#[path = "live_mcp.rs"]
mod live_mcp;
use serde_json::{Value, json};
use soundcraft_engine::{Engine, io};
use soundcraft_ui_egui::dialogs::Dialog;
use soundcraft_ui_egui::{Services, SoundApp};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::Arc,
};
use wasm_bindgen::prelude::*;

const MAX_BYTES: usize = 100 * 1024 * 1024;
const MAX_PCM_BYTES: usize = 256 * 1024 * 1024;

#[wasm_bindgen(module = "/src/audio.js")]
extern "C" {
    #[wasm_bindgen(js_name = installAudioUnlock)]
    fn install_audio_unlock();
    #[wasm_bindgen(catch, js_name = decodeAudio)]
    async fn decode_audio(bytes: &[u8]) -> Result<JsValue, JsValue>;
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftEmit)]
    fn emit(kind: &str, detail: &str, bytes: &[u8]);
    #[wasm_bindgen(js_namespace = window, js_name = flowLikeCraftTake)]
    fn take() -> JsValue;
}

thread_local! {
    static CONTEXT: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static OUTPUT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static IMPORT_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy)]
enum Purpose {
    Open,
    Import { at: i64 },
}
enum Prepared {
    Audio(Vec<u8>),
    Midi(Vec<u8>),
    Session {
        bytes: Vec<u8>,
        decoded: HashMap<String, Vec<u8>>,
    },
}
struct Pending {
    name: String,
    purpose: Purpose,
    result: Result<Option<Prepared>, String>,
}

async fn prepare_audio(name: &str, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    let ext = name.rsplit('.').next();
    // The WASM reader only handles PCM WAV/AIFF. Preserve their BWF metadata.
    if let Ok(size) = io::decoded_audio_byte_length(&bytes, ext) {
        if size > MAX_PCM_BYTES as u64 {
            return Err("The decoded audio exceeds the editor's 256 MiB PCM limit.".into());
        }
        return Ok(bytes);
    }
    let decoded = decode_audio(&bytes).await.map_err(|error| {
        field(&error, "message").as_string().unwrap_or_else(|| {
            "This browser could not decode the audio. Convert it to PCM WAV or AIFF.".into()
        })
    })?;
    if !decoded.is_instance_of::<js_sys::Uint8Array>() {
        return Err("The browser did not return decoded audio.".into());
    }
    let data = js_sys::Uint8Array::new(&decoded);
    if data.length() < 44 || data.length() as usize > MAX_PCM_BYTES + 44 {
        return Err("The decoded audio exceeds the editor's 256 MiB PCM limit.".into());
    }
    Ok(data.to_vec())
}

async fn prepare_file(name: &str, bytes: Vec<u8>, purpose: &Purpose) -> Result<Prepared, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Audio files must be between 1 byte and 100 MiB.".into());
    }
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match (purpose, ext.as_str()) {
        (Purpose::Open, "scraft" | "scraftz") => {
            let mut decoded = HashMap::new();
            if bytes.starts_with(b"PK\x03\x04") {
                let (text, mut media) = io::read_host_bundle(&bytes).map_err(|e| e.to_string())?;
                let session =
                    soundcraft_model::Session::from_json(&text).map_err(|e| e.to_string())?;
                let mut total = 0usize;
                for source in &session.sources {
                    if let Some(audio) = media.remove(&source.path) {
                        let audio = prepare_audio(&source.path, audio).await?;
                        total = total.saturating_add(audio.len());
                        if total > MAX_PCM_BYTES {
                            return Err(
                                "This session exceeds the editor's 256 MiB audio limit.".into()
                            );
                        }
                        decoded.insert(source.path.clone(), audio);
                    }
                }
            }
            Ok(Prepared::Session { bytes, decoded })
        }
        (Purpose::Open, "mid" | "midi" | "smf") => Ok(Prepared::Midi(bytes)),
        _ => prepare_audio(name, bytes).await.map(Prepared::Audio),
    }
}

fn complete_later(name: String, purpose: Purpose, result: Result<Option<Prepared>, String>) {
    PENDING.with(|pending| {
        pending.borrow_mut().push(Pending {
            name,
            purpose,
            result,
        })
    });
    request_repaint();
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
    js_sys::Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

pub fn start() {
    install_audio_unlock();
    io::set_deliver(|_, bytes| {
        if bytes.len() > MAX_BYTES {
            return Err("The exported audio exceeds 100 MiB.".into());
        }
        OUTPUT.with(|out| *out.borrow_mut() = Some(bytes.to_vec()));
        Ok(())
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
        .ok_or("The audio canvas is unavailable.")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Invalid audio canvas.")?;
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                let engine = Engine::default();
                let player = soundcraft_playback::Player::new(Arc::new(engine.session().clone()));
                let services = Services {
                    host_command: Some(Box::new(|id| {
                        if id == "file.import_audio" {
                            IMPORT_REQUESTED.with(|requested| requested.set(true));
                            return Some(Ok(json!({"pending": "browserImport"})));
                        }
                        if id.starts_with("session.save")
                            || id == "file.bounce_mix"
                            || id == "file.export_midi"
                        {
                            event("save-request", json!({}));
                            return Some(Ok(json!({"pending": "hostSave"})));
                        }
                        if id.starts_with("file.")
                            || matches!(
                                id,
                                "session.new"
                                    | "session.open"
                                    | "session.close"
                                    | "session.revert"
                                    | "app.quit"
                            )
                        {
                            return Some(Err("Use the file controls above the editor.".into()));
                        }
                        None
                    })),
                    ..Default::default()
                };
                Ok(Box::new(Host {
                    app: SoundApp::new(engine, Some(player), services),
                    opened: false,
                    busy: false,
                    read_only: true,
                    reported_dirty: false,
                    unsaved_new: false,
                    last_exported: None,
                }))
            }),
        )
        .await
        .map_err(|e| format!("The audio editor could not start: {e:?}"))?;
    std::mem::forget(runner);
    Ok(())
}

struct Host {
    app: SoundApp,
    opened: bool,
    busy: bool,
    read_only: bool,
    reported_dirty: bool,
    unsaved_new: bool,
    last_exported: Option<u64>,
}

impl Host {
    fn start_import_picker(&mut self) {
        if !self.opened || self.read_only || self.busy {
            return;
        }
        self.busy = true;
        let purpose = Purpose::Import {
            at: self.app.engine.session().edit.selection.start.max(0),
        };
        wasm_bindgen_futures::spawn_local(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter(
                    "Audio",
                    &[
                        "wav", "wave", "bwf", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga",
                        "m4a", "aac", "caf", "webm", "mp4",
                    ],
                )
                .pick_file()
                .await;
            let Some(file) = picked else {
                complete_later(String::new(), purpose, Ok(None));
                return;
            };
            let name = file.file_name();
            let result = prepare_file(&name, file.read().await, &purpose)
                .await
                .map(Some);
            complete_later(name, purpose, result);
        });
    }

    fn receive_prepared(&mut self, pending: Pending) -> Result<(), String> {
        self.busy = false;
        let Some(prepared) = pending.result? else {
            return Ok(());
        };
        let at = match pending.purpose {
            Purpose::Open => 0,
            Purpose::Import { at } => {
                if self.read_only {
                    return Err("Enable editing before importing audio.".into());
                }
                at
            }
        };
        let before = self.app.engine.session_arc();
        match prepared {
            Prepared::Audio(bytes) => {
                io::import_audio_bytes(&mut self.app.engine, &pending.name, &bytes, None, None, at)
                    .map_err(|e| e.to_string())?;
            }
            Prepared::Midi(bytes) => {
                io::import_midi_bytes(&mut self.app.engine, &bytes, 0, true)
                    .map_err(|e| e.to_string())?;
            }
            Prepared::Session { bytes, decoded } => {
                let missing = io::open_session_bytes_with_audio(
                    &mut self.app.engine,
                    &pending.name,
                    &bytes,
                    decoded,
                )
                .map_err(|e| e.to_string())?;
                if !missing.is_empty() {
                    return Err("This session references missing media. Open a bundled .scraftz session with its audio included.".into());
                }
            }
        }
        match pending.purpose {
            Purpose::Open => {
                self.app.engine.mark_clean();
                self.opened = true;
                event(
                    "opened",
                    json!({"revision": self.app.engine.revision, "readOnly": self.read_only}),
                );
            }
            Purpose::Import { .. } => {
                self.app.engine.push_undo("Import Audio", before);
                self.app.ui.status = format!("Imported {}", pending.name);
            }
        }
        Ok(())
    }

    fn take_import_prompt(&mut self) {
        if matches!(&self.app.dialogs.open, Some(Dialog::PathPrompt { cmd, .. }) if cmd == "file.import_audio")
        {
            self.app.dialogs.open = None;
            self.start_import_picker();
        }
        if IMPORT_REQUESTED.with(|requested| requested.replace(false)) {
            self.start_import_picker();
        }
    }

    fn receive(&mut self, message: JsValue) -> Result<(), String> {
        match field(&message, "type").as_string().as_deref() {
            Some("mcp-request") => {
                let ctx = CONTEXT
                    .with(|context| context.borrow().clone())
                    .ok_or("The editor is not ready.")?;
                let read_only = self.read_only || !self.opened || self.busy;
                if let Some(response) =
                    live_mcp::receive(&message, read_only, &mut |method, params| {
                        let (request, _reply) =
                            soundcraft_ui_egui::ControlRequest::new(method, params);
                        soundcraft_ui_egui::control::handle(&mut self.app, &ctx, &request)
                            .ok_or_else(|| {
                                "This operation requires a later editor frame.".to_owned()
                            })
                            .and_then(live_mcp::unwrap_control)
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
                if !name.to_ascii_lowercase().ends_with(".scraftz") {
                    return Err("New documents use the .scraftz format.".into());
                }
                if self.busy {
                    return Err("Wait for the current audio import to finish.".into());
                }
                self.app
                    .engine
                    .execute("session.new", &json!({"name": name, "template": "blank"}))
                    .map_err(|error| error.to_string())?;
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.opened = true;
                self.unsaved_new = true;
                event(
                    "opened",
                    json!({"revision": self.app.engine.revision, "readOnly": self.read_only}),
                );
            }
            Some("open") => {
                if self.opened || self.busy {
                    return Err("An audio file is already open.".into());
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
                    return Err("Audio files must be between 1 byte and 100 MiB.".into());
                }
                let bytes = data.to_vec();
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.busy = true;
                wasm_bindgen_futures::spawn_local(async move {
                    let result = prepare_file(&name, bytes, &Purpose::Open).await.map(Some);
                    complete_later(name, Purpose::Open, result);
                });
            }
            Some("set-read-only") => {
                self.read_only = field(&message, "readOnly").as_bool().unwrap_or(true);
                self.app.dialogs.open = None;
            }
            Some("export") => {
                if self.busy {
                    return Err("Wait for the audio import to finish before saving.".into());
                }
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
                let bytes = match format.as_str() {
                    "scraftz" => {
                        io::session_bundle(self.app.engine.session())
                            .map_err(|e| e.to_string())?
                            .0
                    }
                    "mid" | "midi" | "smf" => {
                        io::export_midi(&self.app.engine, None).map_err(|e| e.to_string())?
                    }
                    "wav" | "wave" | "bwf" | "aif" | "aiff" | "flac" => {
                        OUTPUT.with(|out| out.borrow_mut().take());
                        let encoding = match format.as_str() {
                            "aif" | "aiff" => "aiff",
                            "flac" => "flac",
                            _ => "wav",
                        };
                        let end = self.app.engine.session().content_end().max(1);
                        self.app.engine.execute("file.bounce_mix", &json!({"path": format!("output.{encoding}"), "format": encoding, "start": 0, "end": end, "bit_depth": "24", "dither": false})).map_err(|e| e.to_string())?;
                        OUTPUT
                            .with(|out| out.borrow_mut().take())
                            .ok_or("No audio was exported.")?
                    }
                    _ => return Err("This audio format cannot be exported.".into()),
                };
                if bytes.len() > MAX_BYTES {
                    return Err("The exported file exceeds 100 MiB.".into());
                }
                let revision = self.app.engine.revision;
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
                        && self.app.engine.revision as f64 == revision
                    {
                        self.app.engine.mark_clean();
                        self.last_exported = None;
                        self.unsaved_new = false;
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
        if self.read_only || self.busy {
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
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let pending = PENDING.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        for message in pending {
            if let Err(error) = self.receive_prepared(message) {
                self.app.ui.status = error.clone();
                event("error", json!({"error": error}));
            }
        }
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
        self.take_import_prompt();
        let dirty = self.unsaved_new || (self.opened && self.app.engine.is_dirty());
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event(
                "dirty",
                json!({"dirty": dirty, "revision": self.app.engine.revision}),
            );
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.busy {
            ui.label("Preparing audio…");
        }
        if self.read_only {
            ui.horizontal(|ui| {
                ui.label("Flows Audio preview");
                if ui
                    .button(if self.app.is_playing() {
                        "Stop"
                    } else {
                        "Play"
                    })
                    .clicked()
                {
                    let command = if self.app.is_playing() {
                        "transport.stop"
                    } else {
                        "transport.play"
                    };
                    let _ = self.app.run(command, json!({}));
                }
            });
            ui.ctx().input_mut(|input| {
                input.events.clear();
                input.pointer = Default::default();
            });
        }
        ui.add_enabled_ui(self.opened && !self.read_only && !self.busy, |ui| {
            self.app.ui(ui)
        });
    }
}
