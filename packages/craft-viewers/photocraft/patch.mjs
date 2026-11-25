import { readFileSync } from "node:fs";
import { join } from "node:path";
import { prepareBrowserControl } from "./control-patch.mjs";
import { preparePhotoMcp } from "./mcp-patch.mjs";

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
	const app = replaceOnce(
		original(appPath),
		"        prefs_ui::tick(self, ctx, std::time::Instant::now());",
		'        #[cfg(not(target_arch = "wasm32"))]\n        prefs_ui::tick(self, ctx, std::time::Instant::now());\n        #[cfg(target_arch = "wasm32")]\n        prefs_ui::tick(self, ctx);',
		"browser preferences clock",
	);
	write(
		appPath,
		replaceOnce(
			app,
			"            .title_bar_if(menu_bar, |ui, _cx, app| panels::region_versions::show_title_bar(app, ui))",
			"            .title_bar_if(menu_bar, |ui, _cx, app| crate::menus::menu_bar(app, ui))",
			"embedded menu bar",
		),
	);
	const prefsPath = "crates/ui-egui/src/prefs/prefs_ui/save.rs";
	let prefs = replaceOnce(
		original(prefsPath),
		"pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context, now: Instant) {",
		'pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context, #[cfg(not(target_arch = "wasm32"))] now: Instant) {',
		"browser preferences tick",
	);
	prefs = replaceOnce(
		prefs,
		"    let save_at = store_preferences(app, now);",
		'    #[cfg(not(target_arch = "wasm32"))]\n    let save_at = store_preferences(app, now);\n    #[cfg(target_arch = "wasm32")]\n    store_preferences(app);',
		"browser preferences storage",
	);
	prefs = replaceOnce(
		prefs,
		"    autosave(app, now);\n    if let Some(at) = [save_at, app.autosave.deadline()].into_iter().flatten().min() {\n        ctx.request_repaint_after(at.saturating_duration_since(now));\n    }",
		'    // The browser host owns persistence and provides no disk autosave service.\n    #[cfg(not(target_arch = "wasm32"))]\n    {\n        autosave(app, now);\n        if let Some(at) = [save_at, app.autosave.deadline()].into_iter().flatten().min() {\n            ctx.request_repaint_after(at.saturating_duration_since(now));\n        }\n    }',
		"native autosave clock",
	);
	prefs = replaceOnce(
		prefs,
		"fn store_preferences(app: &mut PhotocraftApp, _now: Instant) -> Option<Instant> {",
		"fn store_preferences(app: &mut PhotocraftApp) -> Option<Instant> {",
		"browser storage clock",
	);
	write(prefsPath, prefs);
	for (const dialog of ["caf_ui", "select_mask_ui", "vp_ui"]) {
		const path = `crates/ui-egui/src/dialogs/${dialog}.rs`;
		write(
			path,
			replaceOnce(
				original(path),
				"use std::time::Instant;",
				"use web_time::Instant;",
				`${dialog} browser preview clock`,
			),
		);
	}
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
			'    let home = BarHome::new(tl!("Home").to_owned(), Bind::given(props.home_on)).enabled(props.home_enabled).show(ui);\n    if home.events.value().is_some() {\n        props.out.send(LeadEvent::Home);\n    }\n',
			"",
			"embedded options bar",
		).replaceAll("PhotoCraft", "Flows Photo"),
	);
	prepareBrowserControl({ original, write, replaceOnce });
	// The embedded host supplies the app identity, including in hidden dialogs.
	write(
		"crates/ui-egui/src/panels/brand.rs",
		'pub(crate) fn mark() -> craft::ui::display::Picture<\'static> { craft::ui::display::Picture::new("", 0x464c_4f57_5350, &|| egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT])) }\n',
	);
	const cargoPath = "apps/photocraft-web/Cargo.toml";
	write(
		cargoPath,
		replaceOnce(
			original(cargoPath),
			"[dependencies]\n",
			"[dependencies]\nphotocraft-compose = { workspace = true }\n",
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
	preparePhotoMcp({ source, write, original, replaceOnce });
	return { reconcileLock: true };
}
