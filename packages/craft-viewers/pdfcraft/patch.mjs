import { readFileSync } from "node:fs";
import { join } from "node:path";

export default function prepare({ directory, original, write, replaceOnce }) {
	const uiPath = "crates/ui-egui/src/app.rs";
	let ui = original(uiPath);
	ui = replaceOnce(
		ui,
		"            host_dirty: None,",
		"            host_dirty: None,\n            host_save: None,",
		"host save initialization",
	);
	write(
		"crates/ui-egui/src/lib.rs",
		replaceOnce(
			replaceOnce(
				original("crates/ui-egui/src/lib.rs"),
				"pub(crate) mod brand;\n",
				"",
				"brand artwork module",
			),
			"    pub host_dirty: Option<HostDirtyFn>,",
			"    pub host_dirty: Option<HostDirtyFn>,\n    pub host_save: Option<Box<dyn Fn()>> ,",
			"host save callback",
		),
	);
	const chromePath = "crates/ui-egui/src/chrome.rs";
	write(
		chromePath,
		replaceOnce(
			original(chromePath),
			".title_bar(|ui, _, app: &mut PdfCraftApp| title_bar(app, ui))",
			".title_bar(|ui, _, app: &mut PdfCraftApp| { if app.host_save.is_none() { title_bar(app, ui); } })",
			"host title bar",
		),
	);
	ui = replaceOnce(
		ui,
		'.map_or_else(|| "PdfCraft".to_owned(), |name| format!("{name}{mark} — PdfCraft"))',
		'.map_or_else(|| "Flows PDF".to_owned(), |name| format!("{name}{mark} - Flows PDF"))',
		"window title",
	);
	write(uiPath, ui);
	const filesPath = "crates/ui-egui/src/app/files.rs";
	ui = original(filesPath);
	const open = `pub(super) fn open_bytes_with_focus(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        path: Option<String>,
        bytes: Vec<u8>,
        activate: bool,
    ) -> Result<(), String> {`;
	ui = replaceOnce(
		ui,
		open,
		`${open}\n        if self.host_save.is_some() && !self.views.is_empty() { return Err("Open another document from storage.".into()); }`,
		"single document",
	);
	for (const signature of [
		"pub fn open_dialog(&mut self) {",
		"pub fn close_tab(&mut self, index: usize) {",
	]) {
		ui = replaceOnce(
			ui,
			signature,
			`${signature}\n        if self.host_save.is_some() { return; }`,
			"host file controls",
		);
	}
	write(filesPath, ui);
	const commandsPath = "crates/ui-egui/src/commands.rs";
	const execute =
		"pub fn execute(&mut self, ctx: &egui::Context, id: &str) -> bool {";
	write(
		commandsPath,
		replaceOnce(
			original(commandsPath),
			execute,
			`${execute}\n        if self.host_save.is_some() && ((id.starts_with("file.") && !matches!(id, "file.save" | "file.save_as" | "file.properties")) || id.starts_with("create.") || id == "page.combine") {\n            self.notify("Use the document controls above the editor.");\n            return true;\n        }`,
			"host commands",
		),
	);
	const catalogPath = "crates/ui-egui/src/commands/catalog.rs";
	write(
		catalogPath,
		replaceOnce(
			original(catalogPath),
			"    let gated = menus::gated(app);",
			"    // The palette resolves commands beneath its own modal layer.\n    // Its overlay still blocks shortcuts and page input behind it.\n    let gated = app.modal_open();",
			"palette command gate",
		),
	);
	const editingPath = "crates/ui-egui/src/editing/save.rs";
	let editing = original(editingPath);
	const save = `pub(crate) fn save_then(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        target: SaveTarget,
        after: impl FnOnce(&mut Self, &egui::Context) + 'static,
        failed: impl FnOnce(&mut Self, &egui::Context) + 'static,
    ) -> bool {`;
	editing = replaceOnce(
		editing,
		save,
		`${save}\n        if let Some(save) = &self.host_save {\n            save();\n            return false;\n        }`,
		"asynchronous host save",
	);
	write(
		"crates/ui-egui/src/editing.rs",
		`${original("crates/ui-egui/src/editing.rs")}\n${readFileSync(join(directory, "host-methods.rs"), "utf8")}`,
	);
	write(editingPath, editing);
	const menusPath = "crates/ui-egui/src/menus.rs";
	write(
		menusPath,
		replaceOnce(
			original(menusPath),
			'        bar.tag_item(&CommandRef::new("help.about"), ItemRole::About);',
			'        if self.host_save.is_some() { bar.menus.remove(0); }\n        bar.tag_item(&CommandRef::new("help.about"), ItemRole::About);',
			"host File menu",
		),
	);
	const aboutPath = "crates/ui-egui/src/dialogs/about.rs";
	let about = original(aboutPath);
	const artwork = about.match(
		/\/\/\/ The ArtCraft mark[^\n]*\n[\s\S]*?(?=\/\/\/ What the About tab shows)/,
	)?.[0];
	if (!artwork?.includes("const MARK:"))
		throw new Error("Review PdfCraft About artwork.");
	about = replaceOnce(about, artwork, "", "About artwork loaders");
	about = replaceOnce(
		about,
		'        "PdfCraft",',
		'        "Flows PDF",',
		"About product name",
	);
	about = replaceOnce(
		about,
		'tl!("A clean-room, open-source PDF application written in Rust. MIT OR Apache-2.0.")',
		'"PDF viewing and editing in Flow-Like, powered by PdfCraft. MIT OR Apache-2.0."',
		"About product description",
	);
	about = replaceOnce(
		about,
		'    .mark(AboutImage::svg("ArtCraft", MARK))\n    .part_of(AboutImage::svg("ArtCraft", LOGO).dark(LOGO_WHITE))\n',
		"",
		"About artwork",
	);
	about = about.replace("AboutImage, ", "");
	write(aboutPath, about);
	const homePath = "crates/ui-egui/src/home.rs";
	let home = original(homePath);
	for (const [search, replacement] of [
		[
			"use craft::ui::display::{PictureSize, Text};",
			"use craft::ui::display::Text;",
		],
		["use craft::ui::list::ThumbSize;\n", ""],
		[
			"use crate::{PdfCraftApp, brand, panels::human_size};",
			"use crate::{PdfCraftApp, panels::human_size};",
		],
		[
			"                            brand::mark(ui, PictureSize::Thumb(ThumbSize::Medium));\n",
			"",
		],
	])
		home = replaceOnce(home, search, replacement, "home brand artwork");
	write(homePath, home);
	const manifest = "apps/pdfcraft-web/Cargo.toml";
	write(
		manifest,
		replaceOnce(
			original(manifest),
			"\nserde_json = { workspace = true }",
			'\nserde_json = { workspace = true }\nwasm-bindgen = "0.2"',
			"bridge dependencies",
		),
	);
	const lock = original("Cargo.lock");
	const packageBlock = lock.match(
		/\[\[package\]\]\nname = "pdfcraft-web"\n[\s\S]*?(?=\n\[\[package\]\]|$)/,
	)?.[0];
	if (!packageBlock)
		throw new Error("The PdfCraft web package changed in Cargo.lock.");
	const dependencies = replaceOnce(
		packageBlock,
		' "pdfcraft-ui-egui",',
		' "pdfcraft-ui-egui",\n "wasm-bindgen",',
		"locked bridge dependencies",
	);
	write(
		"Cargo.lock",
		replaceOnce(lock, packageBlock, dependencies, "web package"),
	);
	write(
		"apps/pdfcraft-web/src/web.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	write(
		"apps/pdfcraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod web;\n#[cfg(target_arch = "wasm32")]\nfn main() { web::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	const automationPath = "crates/automation/src/lib.rs";
	let automation = original(automationPath);
	automation = replaceOnce(
		automation,
		"pub struct Automation {",
		"pub struct Automation {\n    live_read_only: Option<bool>,",
		"live MCP scope",
	);
	automation = replaceOnce(
		automation,
		"            session: Session::new(),",
		"            live_read_only: None,\n            session: Session::new(),",
		"live MCP initialization",
	);
	write(
		automationPath,
		`${automation}\n${readFileSync(join(directory, "mcp-policy.rs"), "utf8")}`,
	);
	const call =
		"pub fn call(&mut self, name: &str, args: &Value) -> Result<Vec<Content>> {";
	write(
		"crates/automation/src/dispatch.rs",
		replaceOnce(
			original("crates/automation/src/dispatch.rs"),
			call,
			`${call}\n        if let Some(read_only) = self.live_read_only {\n            if !crate::live_tool_allowed(name) { return Err(failed("Use the document controls above the editor for file operations.")); }\n            if read_only && !tools::find(name).is_some_and(|tool| tool.read_only) { return Err(failed("Enable editing before changing the document.")); }\n        }`,
			"live MCP tool guard",
		),
	);
	const resolve =
		"pub(crate) fn resolve(&self, path: &str, for_write: bool) -> Result<PathBuf> {";
	write(
		"crates/automation/src/paths.rs",
		replaceOnce(
			original("crates/automation/src/paths.rs"),
			resolve,
			`${resolve}\n        if self.live_read_only.is_some() { return Err(failed("File paths are unavailable in the live editor.")); }`,
			"live MCP file guard",
		),
	);
	write(
		"crates/automation/src/mcp.rs",
		`${original("crates/automation/src/mcp.rs")}\n${readFileSync(join(directory, "mcp.rs"), "utf8")}`,
	);
	const uiCargo = "crates/ui-egui/Cargo.toml";
	write(
		uiCargo,
		replaceOnce(
			original(uiCargo),
			"[dependencies]",
			'[dependencies]\npdfcraft-automation = { path = "../automation" }',
			"live MCP dependency",
		),
	);
	return { reconcileLock: true };
}
