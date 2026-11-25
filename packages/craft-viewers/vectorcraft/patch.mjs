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
	const uiPath = "crates/ui-egui/src/services.rs";
	let ui = original(uiPath);
	ui = replaceOnce(
		ui,
		"pub struct Services {",
		"pub struct Services {\n    pub host_command: Option<Box<dyn FnMut(&str) -> Option<Result<serde_json::Value, String>>>>,",
		"host services",
	);
	write(uiPath, ui);
	const commandsPath = "crates/ui-egui/src/app/commands.rs";
	const run =
		"    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {";
	write(
		commandsPath,
		replaceOnce(
			original(commandsPath),
			run,
			`${run}\n        if let Some(result) = self.services.host_command.as_mut().and_then(|hook| hook(id)) { return result; }`,
			"command guard",
		),
	);
	const appPath = "crates/ui-egui/src/app.rs";
	write(
		appPath,
		replaceOnce(
			original(appPath),
			"            .title_bar_if(true, chrome::app_bar)",
			"            .title_bar_if(true, |ui, _cx, app: &mut VectorcraftApp| { craft::menu::MenuBarUi::new(&app.menus.laid).show(ui); })",
			"embedded menu bar",
		),
	);
	const menusPath = "crates/ui-egui/src/menus/model.rs";
	let menus = original(menusPath);
	menus = replaceOnce(
		menus,
		"    let menus: Vec<Menu> = menu_tree_named(app.session.prefs.font_names_in_english, &listed)\n        .into_iter()",
		'    let menus: Vec<Menu> = menu_tree_named(app.session.prefs.font_names_in_english, &listed)\n        .into_iter()\n        .filter(|(title, _)| !matches!(*title, "File" | "Help"))',
		"embedded menus",
	);
	write(menusPath, menus);
	const unsavedPath = "crates/ui-egui/src/unsaved.rs";
	let unsaved = original(unsavedPath);
	const close =
		"pub fn close(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {";
	unsaved = replaceOnce(
		unsaved,
		close,
		`${close}\n    if let Some(result) = app.services.host_command.as_mut().and_then(|hook| hook("file.close")) { return result; }`,
		"tab close guard",
	);
	write(unsavedPath, unsaved);
	write(
		"crates/ui-egui/src/chrome.rs",
		original("crates/ui-egui/src/chrome.rs").replaceAll(
			"VectorCraft",
			"Flows Vector",
		),
	);
	// The embedded host supplies the app identity, including in hidden dialogs.
	write(
		"crates/ui-egui/src/brand.rs",
		"pub(crate) const BRAND_KEY: u64 = 0x464c_4f57_5356;\npub(crate) fn image() -> std::sync::Arc<egui::ColorImage> { std::sync::Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT])) }\n",
	);
	const cargoPath = "apps/vectorcraft-web/Cargo.toml";
	write(
		cargoPath,
		replaceOnce(
			original(cargoPath),
			"[dependencies]\n",
			"[dependencies]\nvectorcraft-render = { workspace = true }\n",
			"host dependencies",
		),
	);
	let lock = original("Cargo.lock");
	const block = lock.match(
		/\[\[package\]\]\nname = "vectorcraft-web"\n[\s\S]*?(?=\n\[\[package\]\]|$)/,
	)?.[0];
	if (!block) throw new Error("The VectorCraft web lock entry is missing.");
	lock = replaceOnce(
		lock,
		block,
		replaceOnce(
			block,
			"dependencies = [\n",
			'dependencies = [\n "vectorcraft-render",\n',
			"locked host dependencies",
		),
		"web lock entry",
	);
	write("Cargo.lock", lock);
	write(
		"apps/vectorcraft-web/src/web.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	write(
		"apps/vectorcraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod web;\n#[cfg(target_arch = "wasm32")]\nfn main() { web::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	prepareLiveMcp({ id: "vectorcraft", source, write, original, replaceOnce });
	const backendPath = "crates/mcp/src/backend.rs";
	let backend = original(backendPath);
	for (const item of [
		"use std::net::{SocketAddr, ToSocketAddrs};",
		"use craft::automation::args::{ControlOptions, EnvNames};",
		"use craft::automation::client::ControlClient;",
		"pub fn control_token() -> Result<String, String> {",
		"pub struct Remote {",
		"impl Remote {",
		"impl Backend for Remote {",
	]) {
		backend = replaceOnce(
			backend,
			item,
			`#[cfg(not(target_arch = "wasm32"))]\n${item}`,
			"native MCP backend",
		);
	}
	write(backendPath, backend);
	const mcpLibPath = "crates/mcp/src/lib.rs";
	write(
		mcpLibPath,
		replaceOnce(
			readFileSync(join(source, mcpLibPath), "utf8"),
			"pub use backend::{Backend, Remote, control_token};",
			'pub use backend::Backend;\n#[cfg(not(target_arch = "wasm32"))]\npub use backend::{Remote, control_token};',
			"native MCP exports",
		),
	);
	const libPath = "crates/ui-egui/src/lib.rs";
	write(
		libPath,
		replaceOnce(
			readFileSync(join(source, libPath), "utf8"),
			"pub fn host_cancel_synthetic(&mut self) { self.synthetic.clear(); }",
			"pub fn host_cancel_synthetic(&mut self, ctx: &egui::Context) { craft::ui::runtime::windows::WindowHost::of(ctx).withdraw_injected(egui::ViewportId::ROOT); }",
			"shared input cancellation",
		),
	);
}
