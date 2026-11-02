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
	write(
		"apps/solidcraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod host;\n#[cfg(target_arch = "wasm32")]\nfn main() { host::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	write(
		"apps/solidcraft-web/src/host.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);

	let engine = original("crates/engine/src/lib.rs");
	engine += `
type HostCommand = fn(&str) -> Option<std::result::Result<Value, String>>;
thread_local! {
    static HOST_COMMAND: std::cell::Cell<Option<HostCommand>> = const { std::cell::Cell::new(None) };
}
pub fn set_host_command(hook: Option<HostCommand>) { HOST_COMMAND.with(|h| h.set(hook)); }
pub fn host_command(id: &str) -> Option<std::result::Result<Value, String>> {
    HOST_COMMAND.with(|h| h.get().and_then(|hook| hook(id)))
}
`;
	engine = replaceOnce(
		engine,
		"    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {",
		"    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {\n        if let Some(result) = host_command(id) { return result.map_err(EngineError::Other); }",
		"host command dispatch",
	);
	write("crates/engine/src/lib.rs", engine);

	let ui = original("crates/ui-egui/src/lib.rs");
	ui = replaceOnce(
		ui,
		"    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {",
		"    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {\n        if let Some(result) = solidcraft_engine::host_command(id) { return result; }",
		"UI command dispatch",
	);
	ui = replaceOnce(
		ui,
		"    pub fn start(&mut self, id: &str) {",
		"    pub fn start(&mut self, id: &str) {\n        if self.host_action(id) { return; }",
		"interactive command dispatch",
	);
	ui = replaceOnce(
		ui,
		"        file_load::tick(self, ctx);",
		"        // The storage host advances its trusted file load separately.",
		"host staged file loading",
	);
	ui += `
mod host_ui;
impl SolveApp {
    pub fn host_action(&mut self, id: &str) -> bool {
        if let Some(result) = solidcraft_engine::host_command(id) {
            if let Err(error) = result { self.set_status(error, true); }
            return true;
        }
        false
    }
}
`;
	write("crates/ui-egui/src/lib.rs", ui);
	write(
		"crates/ui-egui/src/host_ui.rs",
		readFileSync(join(directory, "host_ui.rs"), "utf8"),
	);
	let viewport = original("crates/ui-egui/src/viewport.rs");
	viewport = replaceOnce(
		viewport,
		"crate::navigation::handle(app, ui, rect, &resp, NavCaps::MODEL)",
		"crate::navigation::handle(app, ui, rect, &resp, if crate::host_ui::read_only() { NavCaps::RENDER } else { NavCaps::MODEL })",
		"preview camera capabilities",
	);
	viewport = replaceOnce(
		viewport,
		"crate::select_other::track_hold(app, ui.ctx(), &proj, inside, primary_pressed, primary_down, hover, navigating_from_press);",
		"!crate::host_ui::read_only() && crate::select_other::track_hold(app, ui.ctx(), &proj, inside, primary_pressed, primary_down, hover, navigating_from_press);",
		"preview selection hold",
	);
	viewport = replaceOnce(
		viewport,
		"    // ---- interaction ----",
		"    if !crate::host_ui::read_only() {\n    // ---- interaction ----",
		"preview document interactions",
	);
	viewport = replaceOnce(
		viewport,
		"    crate::preview::paint_sketches(app, &painter, &proj);",
		"    }\n    crate::host_ui::overlays(ui, |ui| {\n    crate::preview::paint_sketches(app, &painter, &proj);",
		"preview document overlays",
	);
	viewport = replaceOnce(
		viewport,
		"    axis_view(app, ui, rect);\n    crate::view_angles::show(app, ui, rect);",
		"    });\n    axis_view(app, ui, rect);\n    crate::view_angles::show(app, ui, rect);",
		"native preview navigation",
	);
	write("crates/ui-egui/src/viewport.rs", viewport);

	let documents = original("crates/ui-egui/src/documents.rs");
	for (const [signature, command] of [
		["pub fn new_design(app: &mut SolveApp) {", "file.new"],
		["pub fn new_tab(app: &mut SolveApp) {", "file.new"],
		["pub fn close(app: &mut SolveApp, i: usize, force: bool) {", "host.close"],
		["pub fn request_quit(app: &mut SolveApp) {", "host.close"],
	]) {
		documents = replaceOnce(
			documents,
			signature,
			`${signature}\n    if app.host_action("${command}") { return; }`,
			command,
		);
	}
	write("crates/ui-egui/src/documents.rs", documents);

	let toolbar = original("crates/ui-egui/src/toolbar.rs");
	for (const [signature, command] of [
		["pub fn save(app: &mut SolveApp) {", "file.save"],
		[
			"fn save_as_with_extension(app: &mut SolveApp, extension: &str) {",
			"file.save_as",
		],
		[
			"fn export_as(app: &mut SolveApp, ext: &str, format: Option<&str>) {",
			"file.export",
		],
	]) {
		toolbar = replaceOnce(
			toolbar,
			signature,
			`${signature}\n    if app.host_action("${command}") { return; }`,
			command,
		);
	}
	write("crates/ui-egui/src/toolbar.rs", toolbar);

	// The storage editor owns the open document. Home and its sample cards must
	// never replace it without the parent's unsaved-change handling.
	write(
		"crates/ui-egui/src/home.rs",
		replaceOnce(
			original("crates/ui-egui/src/home.rs"),
			"    pub fn show(&mut self, _rev: u64) {\n        self.open = true;",
			"    pub fn show(&mut self, _rev: u64) {\n        self.open = false;",
			"embedded home navigation",
		),
	);
	prepareLiveMcp({ id: "solidcraft", source, write, original, replaceOnce });
}
