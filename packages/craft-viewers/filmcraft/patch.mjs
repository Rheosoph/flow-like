import { readFileSync } from "node:fs";
import { join } from "node:path";
import { prepareRmcpLive } from "../rmcp-live.mjs";
import prepareBrowserCodecs from "./codec-patch.mjs";

export default function prepare({
	directory,
	source,
	original,
	write,
	replaceOnce,
}) {
	prepareBrowserCodecs({ directory, original, write, replaceOnce });
	let lib = original("apps/filmcraft-web/src/lib.rs");
	lib = replaceOnce(
		lib,
		"pub mod api;",
		"mod host;\nmod browser_export;\npub mod api;",
		"host module",
	);
	lib = replaceOnce(
		lib,
		"        self.autosave.tick(&self.app.session, ctx);",
		"",
		"browser autosave isolation",
	);
	write("apps/filmcraft-web/src/lib.rs", lib);
	write(
		"apps/filmcraft-web/src/host.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	let engine = original("crates/engine/src/lib.rs");
	engine = replaceOnce(
		engine,
		"pub struct Session {",
		"pub struct Session {\n    pub host_command: Option<fn(&str) -> Option<std::result::Result<Value, String>>>,",
		"session host command",
	);
	engine = replaceOnce(
		engine,
		'            project: Arc::new(Project::new("Untitled")),',
		'            host_command: None,\n            project: Arc::new(Project::new("Untitled")),',
		"session host default",
	);
	engine = replaceOnce(
		engine,
		"pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {",
		"pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {\n        if let Some(result) = self.host_command.and_then(|hook| hook(id)) { return result.map_err(EngineError::Other); }",
		"host save interception",
	);
	write("crates/engine/src/lib.rs", engine);
	let menus = original("crates/ui-egui/src/menus.rs");
	menus = replaceOnce(
		menus,
		"pub fn invoke(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {",
		"pub fn invoke(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {\n    if let Some(result) = app.session.host_command.and_then(|hook| hook(id)) { return result; }",
		"menu host interception",
	);
	menus = menus.replaceAll('"About FilmCraft"', '"About Flows Video"');
	write("crates/ui-egui/src/menus.rs", menus);
	let exportPanel = original("crates/ui-egui/src/panels/export_mode.rs");
	exportPanel = replaceOnce(
		exportPanel,
		"pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {",
		`pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    if app.session.host_command.is_some() {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        child.vertical_centered(|ui| {
            ui.add_space(32.0);
            ui.heading("Save document");
            ui.label("Use Save above the editor to export this video or project and run its save workflow.");
            if ui.button("Save").clicked() {
                if let Err(error) = app.session.execute("file.save", json!({})) {
                    app.ui.status = error.to_string();
                }
            }
        });
        return;
    }`,
		"host export format controls",
	);
	write("crates/ui-egui/src/panels/export_mode.rs", exportPanel);
	let quickExport = original("crates/ui-egui/src/panels/export_mode/quick.rs");
	quickExport = replaceOnce(
		quickExport,
		"pub fn quick_export(app: &mut FilmcraftApp, ctx: &egui::Context, anchor: egui::Pos2) {",
		`pub fn quick_export(app: &mut FilmcraftApp, ctx: &egui::Context, anchor: egui::Pos2) {
    if app.session.host_command.is_some() {
        if app.ui.export.quick_open {
            app.ui.export.quick_open = false;
            if let Err(error) = app.session.execute("file.save", json!({})) {
                app.ui.status = error.to_string();
            }
        }
        return;
    }`,
		"host quick export controls",
	);
	write("crates/ui-egui/src/panels/export_mode/quick.rs", quickExport);
	write(
		"crates/ui-egui/src/brand.rs",
		`pub fn paint_wordmark(ui: &egui::Ui, left_center: egui::Pos2, height: f32, _dark: bool) -> Option<egui::Rect> {
    Some(ui.painter().text(left_center, egui::Align2::LEFT_CENTER, "Flows Video", egui::FontId::proportional(height), ui.visuals().text_color()))
}
`,
	);
	let header = original("crates/ui-egui/src/header.rs");
	header = replaceOnce(
		header,
		'let label = "Discord";',
		'let label = "Flows Video";',
		"header brand",
	);
	header = header
		.replaceAll(
			'"Join the ArtCraft Discord (discord.gg/artcraft)"',
			'"Flows Video"',
		)
		.replaceAll('"Join the ArtCraft Discord"', '"Flows Video"');
	header = replaceOnce(
		header,
		"            crate::links::open(app, ui.ctx(), crate::links::DISCORD);",
		'            app.ui.status = "Flows Video".into();',
		"header external link",
	);
	write("crates/ui-egui/src/header.rs", header);
	prepareRmcpLive({ id: "filmcraft", source, write, original, replaceOnce });
	return {
		reconcileLock: true,
		assets: [
			{
				from: "apps/filmcraft-web/web/audio-worklet.js",
				to: "audio-worklet.js",
			},
		],
	};
}
