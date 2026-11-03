import { readFileSync } from "node:fs";
import { join } from "node:path";

export default function prepare({ directory, original, write, replaceOnce }) {
	const uiPath = "crates/ui-egui/src/app.rs";
	let ui = original(uiPath);
	ui = replaceOnce(
		ui,
		"        chrome::tab_strip(self, ui);",
		"        if self.host_save.is_none() { chrome::tab_strip(self, ui); }",
		"document tabs",
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
	const open =
		"pub(super) fn open_bytes_with_focus(&mut self, name: &str, path: Option<String>, bytes: Vec<u8>, activate: bool) -> Result<(), String> {";
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
	const execute = "pub fn execute(&mut self, id: &str) -> bool {";
	write(
		commandsPath,
		replaceOnce(
			original(commandsPath),
			execute,
			`${execute}\n        if self.host_save.is_some() && ((id.starts_with("file.") && !matches!(id, "file.save" | "file.save_as" | "file.properties")) || id.starts_with("create.") || id == "page.combine") {\n            self.notify("Use the document controls above the editor.");\n            return true;\n        }`,
			"host commands",
		),
	);
	const editingPath = "crates/ui-egui/src/editing/save.rs";
	const createPath = "crates/ui-egui/src/workflows/create_ui.rs";
	write(
		createPath,
		replaceOnce(
			original(createPath),
			"    fn open_created_bytes(",
			"    pub(crate) fn open_created_bytes(",
			"host document creation",
		),
	);
	let editing = original(editingPath);
	const save =
		"pub(crate) fn save_then(&mut self, index: usize, target: SaveTarget, after: impl FnOnce(&mut Self) + Send + 'static) -> bool {";
	editing = replaceOnce(
		editing,
		save,
		`${save}\n        if let Some(save) = &self.host_save {\n            let _ = save("document.pdf", &[], matches!(target, SaveTarget::As));\n            return false;\n        }`,
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
	const dialogsPath = "crates/ui-egui/src/dialogs.rs";
	let dialogs = original(dialogsPath);
	for (const [search, replacement] of [
		["widgets::artcraft_mark(ui, 40.0);", "ui.add_space(8.0);"],
		[
			'egui::RichText::new("PdfCraft").font(theme::semibold(20.0))',
			'egui::RichText::new("Flows PDF").font(theme::semibold(20.0))',
		],
		[
			"widgets::artcraft_logo(ui, 16.0);",
			'ui.label("Powered by PdfCraft. © ArtCraft contributors.");',
		],
	])
		dialogs = replaceOnce(dialogs, search, replacement, "product identity");
	write(dialogsPath, dialogs);
	const homePath = "crates/ui-egui/src/home.rs";
	write(
		homePath,
		replaceOnce(
			original(homePath),
			"                        widgets::artcraft_mark(ui, 28.0);\n",
			"",
			"home brand artwork",
		),
	);
	const widgetsPath = "crates/ui-egui/src/widgets.rs";
	const widgets = original(widgetsPath);
	const brandWidgets = widgets.match(
		/^\/\/\/ The ArtCraft wordmark[^\n]*\n[\s\S]*?(?=^\/\/\/ Buttons for every community link)/m,
	)?.[0];
	if (!brandWidgets?.includes("pub fn artcraft_mark("))
		throw new Error(
			"The PdfCraft brand widgets changed; review artwork removal.",
		);
	// Remove the asset loaders too, so hidden UI cannot keep restricted logos in WASM.
	write(
		widgetsPath,
		replaceOnce(widgets, brandWidgets, "", "brand artwork loaders"),
	);
	const manifest = "apps/pdfcraft-web/Cargo.toml";
	write(
		manifest,
		replaceOnce(
			original(manifest),
			'\negui = { workspace = true, features = ["default_fonts", "monochrome_emoji_fonts"] }',
			'\negui = { workspace = true, features = ["default_fonts", "monochrome_emoji_fonts"] }\nserde_json = { workspace = true }\nwasm-bindgen = "0.2"',
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
		' "pdfcraft-ui-egui",\n "serde_json",\n "wasm-bindgen",',
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
