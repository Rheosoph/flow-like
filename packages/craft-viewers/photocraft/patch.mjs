import { prepareRmcpLive } from "../rmcp-live.mjs";
import { readFileSync } from "node:fs";
import { join } from "node:path";

export default function prepare({
	directory,
	source,
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
	const appPath = "crates/ui-egui/src/app/frame.rs";
	write(
		appPath,
		replaceOnce(
			original(appPath),
			"            panels::title_bar(self, ui);",
			'            egui::Panel::top("flows-photo-menu").show(ui, |ui| { ui.horizontal(|ui| { crate::menus::menu_bar(self, ui); }); });',
			"embedded menu bar",
		),
	);
	const menusPath = "crates/ui-egui/src/menus/invoke.rs";
	let menus = original(menusPath);
	const invoke =
		"pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {";
	menus = replaceOnce(
		menus,
		invoke,
		`${invoke}\n    if let Some(result) = app.services.host_command.as_mut().and_then(|hook| hook(id)) { return result; }`,
		"menu command guard",
	);
	write(menusPath, menus);
	const menuRoot = "crates/ui-egui/src/menus.rs";
	write(
		menuRoot,
		replaceOnce(
			original(menuRoot),
			'pub const TOP_MENUS: [&str; 10] = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];',
			'pub const TOP_MENUS: [&str; 8] = ["Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window"];',
			"embedded menus",
		),
	);
	const optionsPath = "crates/ui-egui/src/panels/options_bar.rs";
	write(
		optionsPath,
		replaceOnce(
			original(optionsPath),
			"                        if t.pro {\n                            crate::chrome_ui::home_button(app, ui);\n                            widgets::vline(ui, 22.0);\n                        }\n",
			"",
			"embedded options bar",
		).replaceAll("PhotoCraft", "Flows Photo"),
	);
	const inputPath = "crates/ui-egui/src/app/input.rs";
	write(
		inputPath,
		replaceOnce(
			original(inputPath),
			"    fn cancel_synthetic(&mut self, reason: &str) {",
			"    pub fn cancel_synthetic(&mut self, reason: &str) {",
			"host input cancellation",
		),
	);
	// The embedded host supplies the app identity, including in hidden dialogs.
	write(
		"crates/ui-egui/src/panels/brand.rs",
		"pub fn paint_mark(_ui: &egui::Ui, _rect: egui::Rect) {}\n",
	);
	const cargoPath = "apps/photocraft-web/Cargo.toml";
	write(
		cargoPath,
		replaceOnce(
			original(cargoPath),
			"[dependencies]\n",
			"[dependencies]\nserde_json = { workspace = true }\nphotocraft-compose = { workspace = true }\n",
			"host dependencies",
		),
	);
	let lock = original("Cargo.lock");
	const block = lock.match(
		/\[\[package\]\]\nname = "photocraft-web"\n[\s\S]*?(?=\n\[\[package\]\]|$)/,
	)?.[0];
	if (!block) throw new Error("The PhotoCraft web lock entry is missing.");
	lock = replaceOnce(
		lock,
		block,
		replaceOnce(
			block,
			"dependencies = [\n",
			'dependencies = [\n "photocraft-compose",\n',
			"locked host dependencies",
		),
		"web lock entry",
	);
	write("Cargo.lock", lock);
	write(
		"apps/photocraft-web/src/web.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	write(
		"apps/photocraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod web;\n#[cfg(target_arch = "wasm32")]\nfn main() { web::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	prepareRmcpLive({ id: "photocraft", source, write, original, replaceOnce });
	return { reconcileLock: true };
}
