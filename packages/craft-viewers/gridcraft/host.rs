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
                let app = create_app(cc);
                Ok(Box::new(Host {
                    app,
                    opened: false,
                    read_only: true,
                    name: String::new(),
                    reported_dirty: false,
                    unsaved_new: false,
                    last_exported: None,
                }))
            }),
        )
        .await
        .map_err(|e| format!("The document editor could not start: {e:?}"))
}
struct Host {
    app: gridcraft_ui_egui::SheetApp,
    opened: bool,
    read_only: bool,
    name: String,
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
                        match gridcraft_ui_egui::control::handle(
                            &mut self.app,
                            &ctx,
                            method,
                            &params,
                        ) {
                            gridcraft_ui_egui::control::Outcome::Done(reply) => {
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
                if !name.to_ascii_lowercase().ends_with(".xlsx") {
                    return Err("New documents use the .xlsx format.".into());
                }
                self.app.session.new_workbook();
                if let Some(doc) = self.app.session.active_mut() {
                    doc.title = name.clone();
                }
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
                if !matches!(
                    extension(&name).as_str(),
                    "xlsx"
                        | "xlsm"
                        | "xltx"
                        | "xltm"
                        | "xls"
                        | "xlt"
                        | "xlsb"
                        | "ods"
                        | "csv"
                        | "tsv"
                        | "tab"
                ) {
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
                let (workbook, warnings) = gridcraft_engine::io::open_bytes(&name, &bytes.to_vec())
                    .map_err(|e| e.to_string())?;
                self.app
                    .session
                    .add_document(gridcraft_engine::DocState::new(
                        workbook,
                        Some(name.clone()),
                        name.clone(),
                        gridcraft_engine::Trust::FromFile,
                    ));
                self.app.show_open_warnings(&json!({"warnings": warnings}));
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
                if !matches!(
                    format.as_str(),
                    "xlsx" | "xlsm" | "xltx" | "xltm" | "csv" | "tsv" | "tab"
                ) {
                    return Err("This format is supported for import only. Choose a supported export format.".into());
                }
                if !self.read_only {
                    if self
                        .app
                        .macros
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.dirty)
                    {
                        return Err(
                            "Save or close the macro editor before saving the workbook.".into()
                        );
                    }
                    if self.app.editor.is_some() && !self.app.commit_edit(0, 0, false, false) {
                        return Err("Finish editing the current cell before saving.".into());
                    }
                    self.app.commit_text_box_edit()?;
                    self.app.store_view_state();
                }
                let doc = self.app.session.doc().map_err(|e| e.to_string())?;
                let bytes =
                    gridcraft_engine::io::save_bytes(&doc.wb, &format!("workbook.{format}"))
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
                    if !self
                        .app
                        .macros
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.dirty)
                        && self.app.editor.is_none()
                        && self.app.text_box_editor.is_none()
                        && self.last_exported == Some(revision as u64)
                        && self.revision() as f64 == revision
                    {
                        if let Some(doc) = self.app.session.active_mut() {
                            doc.saved = doc.wb.clone();
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
        let dirty = self.unsaved_new
            || self
                .app
                .macros
                .editor
                .as_ref()
                .is_some_and(|editor| editor.dirty)
            || self.app.editor.is_some()
            || self.app.text_box_editor.is_some()
            || self.app.session.active().is_some_and(|d| d.is_dirty());
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            event(
                "dirty",
                json!({"dirty": dirty, "revision": self.revision()}),
            );
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        if !self.app.fonts_ready {
            ui.ctx().request_repaint();
            return;
        }
        let sheets: Vec<_> = self
            .app
            .session
            .active()
            .map(|d| {
                d.wb.sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, sheet)| {
                        sheet.visibility == gridcraft_engine::model::Visibility::Visible
                    })
                    .map(|(index, sheet)| (index, sheet.name.clone(), index == d.wb.active_sheet()))
                    .collect()
            })
            .unwrap_or_default();
        egui::Panel::bottom("preview-sheets")
            .exact_size(34.0)
            .show(ui, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, name, active) in sheets {
                            if ui.selectable_label(active, name).clicked() {
                                let _ = self
                                    .app
                                    .session
                                    .run("sheet.activate", json!({"sheet": index}));
                            }
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                gridcraft_ui_egui::grid::show_read_only(&mut self.app, ui);
            });
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

fn create_app(cc: &eframe::CreationContext<'_>) -> gridcraft_ui_egui::SheetApp {
    let mut session = gridcraft_engine::Session::new();
    session.hosted = true;
    session.prefs.user_name = "Flows Sheets User".into();
    let ctx = cc.egui_ctx.clone();
    let services = gridcraft_ui_egui::Services {
        host_command: Some(Box::new(|id| {
            if matches!(id, "file.save" | "file.saveAs" | "file.saveBytes") {
                event("save-request", json!({}));
                return Some(Ok(json!({"pending": "hostSave"})));
            }
            if id.starts_with("file.")
                || matches!(
                    id,
                    "ui.backstage" | "ui.open" | "window.close" | "window.activate"
                )
            {
                return Some(Err("Use the document controls above the editor.".into()));
            }
            None
        })),
        pick_picture_async: Some(Box::new(move |inbox| {
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("Pictures", &["png", "jpg", "jpeg"])
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
        ..Default::default()
    };
    let mut app = gridcraft_ui_egui::SheetApp::new(session, services);
    app.chrome = gridcraft_ui_egui::Chrome {
        title_bar: false,
        file_tab: false,
    };
    app.ui.ribbon_collapsed = true;
    app.ui.system_theme = true;
    app.ui.autosave = false;
    app
}
