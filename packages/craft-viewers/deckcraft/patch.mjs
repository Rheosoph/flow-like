import { readFileSync } from "node:fs";
import { join } from "node:path";
import { prepareLiveMcp } from "../live-mcp.mjs";
import { prepareBrowserMedia } from "./media-patch.mjs";

export function patchThumbnailVisibility(source, replaceOnce) {
	const start = "        let wpx = (tw * ppp).round() as u32;\n";
	const end =
		"        if selected {\n            p.rect_stroke(img.expand(2.0)";
	replaceOnce(source, start, start, "slide thumbnail start");
	replaceOnce(source, end, end, "slide thumbnail end");
	const begin = source.indexOf(start);
	const finish = source.indexOf(end);
	if (finish <= begin)
		throw new Error("Review the slide thumbnail paint order.");
	const paint = source.slice(begin, finish);
	const guarded = paint
		.trimEnd()
		.split("\n")
		.map((line) => `    ${line}`)
		.join("\n");
	return replaceOnce(
		source,
		paint,
		`        if ui.is_rect_visible(img) {\n${guarded}\n        }\n`,
		"visible slide thumbnails",
	);
}

export default function prepare({
	source,
	directory,
	original,
	write,
	replaceOnce,
}) {
	const patch = (path, search, replacement, label) =>
		write(path, replaceOnce(original(path), search, replacement, label));
	if (
		!original("crates/ui-egui/src/panels/thumbs.rs").includes(
			"let tex = if near {",
		)
	)
		throw new Error("Review upstream thumbnail visibility.");
	const uiPath = "crates/ui-egui/src/lib.rs";
	let ui = original(uiPath);
	ui = replaceOnce(
		ui,
		"pub mod media;",
		'#[cfg(not(target_arch = "wasm32"))]\npub mod media;\n#[cfg(target_arch = "wasm32")]\n#[path = "browser-media.rs"]\npub mod media;',
		"browser media UI",
	);
	ui = replaceOnce(
		ui,
		'"About DeckCraft"',
		'"About Flows Slides"',
		"About command",
	);
	write(uiPath, ui);
	patch(
		"crates/ui-egui/src/services.rs",
		"pub struct Services {",
		"pub struct Services {\n    pub host_command: Option<Box<dyn Fn(&str) -> Option<Result<serde_json::Value, String>>>>,",
		"host services",
	);
	const run =
		"pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {";
	patch(
		"crates/ui-egui/src/app/commands.rs",
		run,
		`${run}\n        if let Some(result) = self.services.host_command.as_ref().and_then(|filter| filter(id)) { return result; }`,
		"command dispatch",
	);
	patch(
		"crates/ui-egui/src/app.rs",
		"        ribbon::title_bar(self, ui);",
		"        if self.services.host_command.is_none() { ribbon::title_bar(self, ui); }",
		"title bar",
	);
	let files = original("crates/ui-egui/src/app/files.rs");
	for (const [signature, command] of [
		["pub fn save(&mut self) {", "file.save"],
		["pub fn save_as_dialog(&mut self, format: &str) {", "file.saveAs"],
	]) {
		files = replaceOnce(
			files,
			signature,
			`${signature}\n        if let Some(result) = self.services.host_command.as_ref().and_then(|filter| filter("${command}")) {\n            if let Err(error) = result { self.set_status(error); }\n            return;\n        }`,
			command,
		);
	}
	write("crates/ui-egui/src/app/files.rs", files);
	const enginePath = "crates/engine/src/lib.rs";
	let engine = original(enginePath);
	engine = replaceOnce(
		engine,
		"pub struct Session {",
		"pub struct Session {\n    pub hosted: bool,",
		"hosted session",
	);
	engine = replaceOnce(
		engine,
		"        Session {\n            docs:",
		"        Session {\n            hosted: false,\n            docs:",
		"session initialization",
	);
	const execute =
		"pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {";
	engine = replaceOnce(
		engine,
		execute,
		`${execute}\n        if self.hosted && id.starts_with("file.") { return Err(EngineError::Other("Use the document controls above the editor.".into())); }`,
		"hosted file commands",
	);
	write(enginePath, engine);
	const dialogsPath = "crates/ui-egui/src/dialogs/about.rs";
	let dialogs = original(dialogsPath);
	for (const [search, replacement] of [
		[
			'egui::RichText::new("DeckCraft").font(theme::bold(22.0))',
			'egui::RichText::new("Flows Slides").font(theme::bold(22.0))',
		],
		[
			'ui.label("Presentations and slide shows, rebuilt from scratch in pure Rust.");',
			'ui.label("Presentation viewing and editing in Flow-Like, powered by DeckCraft.");',
		],
		[
			"        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());\n        paint_logo(ui.painter(), r);",
			"        ui.add_space(8.0);",
		],
	])
		dialogs = replaceOnce(dialogs, search, replacement, "product identity");
	write(dialogsPath, dialogs);
	patch(
		"crates/ui-egui/src/dialogs.rs",
		'"about" => "About DeckCraft",',
		'"about" => "About Flows Slides",',
		"About dialog title",
	);
	const manifest = "apps/deckcraft-web/Cargo.toml";
	write(
		manifest,
		replaceOnce(
			original(manifest),
			"deckcraft-engine = { workspace = true }",
			"deckcraft-engine = { workspace = true }\ndeckcraft-render = { workspace = true }\ndeckcraft-media = { workspace = true }",
			"renderer dependency",
		),
	);
	let lock = original("Cargo.lock");
	const packageBlock = lock.match(
		/\[\[package\]\]\nname = "deckcraft-web"\n[\s\S]*?(?=\n\[\[package\]\]|$)/,
	)?.[0];
	if (!packageBlock)
		throw new Error("The DeckCraft web package changed in Cargo.lock.");
	lock = replaceOnce(
		lock,
		packageBlock,
		replaceOnce(
			packageBlock,
			' "deckcraft-engine",',
			' "deckcraft-engine",\n "deckcraft-media",\n "deckcraft-render",',
			"locked renderer dependency",
		),
		"web package",
	);
	const mediaBlock = lock.match(
		/\[\[package\]\]\nname = "deckcraft-media"\n[\s\S]*?(?=\n\[\[package\]\]|$)/,
	)?.[0];
	if (!mediaBlock) throw new Error("Review DeckCraft's media lock entry.");
	lock = replaceOnce(
		lock,
		mediaBlock,
		replaceOnce(
			mediaBlock,
			' "image",',
			' "image",\n "js-sys",\n "serde_json",\n "wasm-bindgen",\n "wasm-bindgen-futures",',
			"browser media dependencies",
		),
		"media lock entry",
	);
	write("Cargo.lock", lock);
	prepareBrowserMedia({ directory, original, write, replaceOnce });
	write(
		"apps/deckcraft-web/src/web.rs",
		readFileSync(join(directory, "host.rs"), "utf8"),
	);
	write(
		"apps/deckcraft-web/src/main.rs",
		'#[cfg(target_arch = "wasm32")]\nmod web;\n#[cfg(target_arch = "wasm32")]\nfn main() { web::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);
	prepareLiveMcp({ id: "deckcraft", source, write, original, replaceOnce });
}
