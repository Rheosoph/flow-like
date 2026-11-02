import { readFileSync } from "node:fs";
import { join } from "node:path";
import { prepareLiveMcp } from "../live-mcp.mjs";

export default function prepare({
	source,
	directory,
	original,
	write,
	replaceOnce,
}) {
	const patch = (path, search, replacement, label) =>
		write(path, replaceOnce(original(path), search, replacement, label));
	patch(
		"crates/ui-egui/src/services.rs",
		"pub struct Services {",
		"pub struct Services {\n    pub host_command: Option<Box<dyn Fn(&str) -> Option<Result<serde_json::Value, String>>>>,",
		"host services",
	);
	const run =
		"pub fn run(&mut self, id: &str, params: Json) -> Result<Json, String> {";
	patch(
		"crates/ui-egui/src/app/commands.rs",
		run,
		`${run}\n        if let Some(result) = self.services.host_command.as_ref().and_then(|filter| filter(id)) { return result; }`,
		"command dispatch",
	);
	const enginePath = "crates/engine/src/lib.rs";
	let engine = original(enginePath);
	engine = replaceOnce(
		engine,
		"pub struct Session {",
		"pub struct Session {\n    pub hosted: bool,",
		"hosted session",
	);
	const execute =
		"pub fn execute(&mut self, id: &str, params: Json) -> Result<Json> {";
	engine = replaceOnce(
		engine,
		execute,
		`${execute}\n        if self.hosted && (id.starts_with("file.") || matches!(id, "window.close" | "window.activate" | "view.newWindow")) { return Err(EngineError::Other("Use the document controls above the editor.".into())); }`,
		"hosted file commands",
	);
	write(enginePath, engine);
	patch(
		"crates/ui-egui/src/ribbon/automate.rs",
		'"About\\nGridCraft"',
		'"About\\nFlows Sheets"',
		"About button",
	);
	const gridPath = "crates/ui-egui/src/grid/show.rs";
	let grid = original(gridPath);
	grid = replaceOnce(
		grid,
		"pub fn show(app: &mut SheetApp, ui: &mut egui::Ui) {",
		"pub fn show(app: &mut SheetApp, ui: &mut egui::Ui) { show_impl(app, ui, false); }\npub fn show_read_only(app: &mut SheetApp, ui: &mut egui::Ui) { show_impl(app, ui, true); }\nfn show_impl(app: &mut SheetApp, ui: &mut egui::Ui, read_only: bool) {",
		"read-only grid entry",
	);
	grid = replaceOnce(
		grid,
		"        if zoom_delta != 1.0 {",
		"        if zoom_delta != 1.0 && !read_only {",
		"preview zoom",
	);
	grid = replaceOnce(
		grid,
		"    show_grid_interactions(app, ui, &resp, &geo, sh, &wb);",
		"    if !read_only { show_grid_interactions(app, ui, &resp, &geo, sh, &wb); }",
		"read-only interaction boundary",
	);
	patch(
		"crates/ui-egui/src/grid.rs",
		"pub use show::show;",
		"pub use show::{show, show_read_only};",
		"read-only grid export",
	);
	write(gridPath, grid);
	patch(
		"crates/ui-egui/src/dialogs.rs",
		'Dialog::custom("about", "About GridCraft",',
		'Dialog::custom("about", "About Flows Sheets",',
		"About title",
	);
	let about = original("crates/ui-egui/src/dialogs/about.rs");
	about = replaceOnce(
		about,
		'ui.heading("GridCraft");',
		'ui.heading("Flows Sheets");',
		"product heading",
	);
	about = replaceOnce(
		about,
		'ui.label("A clean-room, open-source, Rust-native spreadsheet. Part of ArtCraft.");',
		'ui.label("Spreadsheet viewing and editing in Flow-Like, powered by GridCraft.");',
		"product identity",
	);
	write("crates/ui-egui/src/dialogs/about.rs", about);
	write("Cargo.lock", original("Cargo.lock"));
	write(
		"apps/gridcraft-web/src/web.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	write(
		"apps/gridcraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod web;\n#[cfg(target_arch = "wasm32")]\nfn main() { web::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	prepareLiveMcp({ id: "gridcraft", source, write, original, replaceOnce });
}
