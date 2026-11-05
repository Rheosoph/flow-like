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
		"apps/cadcraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod host;\n#[cfg(target_arch = "wasm32")]\nfn main() { host::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	write(
		"apps/cadcraft-web/src/host.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	let engine = original("crates/engine/src/lib.rs");
	engine = replaceOnce(
		engine,
		"pub struct Session {",
		"pub struct Session {\n    pub host_command: Option<fn(&str) -> Option<std::result::Result<Value, String>>>,",
		"host commands",
	);
	engine = replaceOnce(
		engine,
		"            docs: Vec::new(),",
		"            host_command: None,\n            docs: Vec::new(),",
		"host command default",
	);
	write("crates/engine/src/lib.rs", engine);
	write(
		"crates/engine/src/session/execute.rs",
		replaceOnce(
			original("crates/engine/src/session/execute.rs"),
			"        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;",
			"        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;\n        if let Some(result) = self.host_command.and_then(|hook| hook(spec.id)) { return result.map_err(EngineError::Other); }",
			"programmatic commands",
		),
	);
	write(
		"crates/engine/src/session/interactive.rs",
		replaceOnce(
			original("crates/engine/src/session/interactive.rs"),
			"        if transparent && spec.transparent && self.running.is_some() {",
			"        if let Some(result) = self.host_command.and_then(|hook| hook(spec.id)) { return result.map(|_| ()).map_err(EngineError::Other); }\n        if transparent && spec.transparent && self.running.is_some() {",
			"interactive commands",
		),
	);
	let menus = original("crates/ui-egui/src/menus.rs");
	menus = replaceOnce(
		menus,
		"pub fn run_ui_command(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {",
		"pub fn run_ui_command(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {\n    if let Some(result) = app.session.host_command.and_then(|hook| hook(id)) { return Some(result); }",
		"UI commands",
	);
	write(
		"crates/ui-egui/src/menus.rs",
		menus.replaceAll("CADCraft", "Flows CAD"),
	);
	write(
		"crates/ui-egui/src/chrome.rs",
		original("crates/ui-egui/src/chrome.rs").replaceAll(
			"CADCraft",
			"Flows CAD",
		),
	);
	write(
		"crates/mcp/Cargo.toml",
		replaceOnce(
			original("crates/mcp/Cargo.toml"),
			'getrandom = "0.4"',
			'getrandom = { version = "0.4", features = ["wasm_js"] }',
			"browser randomness",
		),
	);
	prepareLiveMcp({ id: "cadcraft", source, write, original, replaceOnce });
}
