import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const directory = dirname(fileURLToPath(import.meta.url));

// The upstream MCP crate owns both schemas and dispatch. This adapter supplies
// its Backend with the editor already mounted by the storage viewer.
export function prepareLiveMcp({ id, source, write, original, replaceOnce }) {
	const crate = `${id}-${id === "soundcraft" ? "automation" : "mcp"}`;
	const path = `apps/${id}-web`;
	const manifest = `${path}/Cargo.toml`;
	const current = (name) => readFileSync(join(source, name), "utf8");
	const dependency = `\n[target.'cfg(target_arch = "wasm32")'.dependencies.${crate}]\npath = "../../crates/${id === "soundcraft" ? "automation" : "mcp"}"\n`;
	write(manifest, current(manifest).replaceAll(dependency, "") + dependency);
	const lock = current("Cargo.lock");
	const block = lock.match(
		new RegExp(
			`\\[\\[package\\]\\]\\nname = "${id}-web"\\n[\\s\\S]*?(?=\\n\\[\\[package\\]\\]|$)`,
		),
	)?.[0];
	if (!block) throw new Error(`Missing ${id}-web lock entry.`);
	write(
		"Cargo.lock",
		replaceOnce(
			lock,
			block,
			replaceOnce(
				block.replaceAll(` "${crate}",\n`, ""),
				"dependencies = [\n",
				`dependencies = [\n "${crate}",\n`,
				"MCP dependency",
			),
			"web MCP lock entry",
		),
	);
	if (id === "cadcraft") {
		write(
			"crates/mcp/src/lib.rs",
			replaceOnce(
				original("crates/mcp/src/lib.rs"),
				"mod tools;",
				"pub mod tools;",
				"MCP dispatcher visibility",
			),
		);
	}
	if (id === "vectorcraft") {
		// The dispatcher also uses this module's pointer parser. Keep it intact
		// and exclude only the native file writer from the browser build.
		write("crates/mcp/src/lib.rs", original("crates/mcp/src/lib.rs"));
		const nativeWrite =
			'vectorcraft_format::write_atomic(std::path::Path::new(path), &png).map_err(|e| format!("write {path}: {e}"))?;';
		write(
			"crates/mcp/src/headless.rs",
			replaceOnce(
				original("crates/mcp/src/headless.rs"),
				nativeWrite,
				`#[cfg(target_arch = "wasm32")]\n                return Err("The embedded editor sends file output through its host.".into());\n                #[cfg(not(target_arch = "wasm32"))]\n                ${nativeWrite}`,
				"native MCP file writer",
			),
		);
	}
	if (id === "soundcraft") {
		let control = replaceOnce(
			original("crates/ui-egui/src/control.rs"),
			"fn handle(app: &mut SoundApp",
			"pub fn handle(app: &mut SoundApp",
			"live control visibility",
		);
		control = replaceOnce(
			control,
			`pub(crate) fn expire_gestures(app: &mut SoundApp) {
    let now = app.clock_now();
    if !app.gesture_acks.iter().any(|ack| now >= ack.deadline) {
        return;
    }`,
			`pub(crate) fn expire_gestures(app: &mut SoundApp) {
    let now = app.clock_now();
    if app.gesture_acks.iter().any(|ack| now >= ack.deadline) { cancel_synthetic(app); }
}
pub(crate) fn cancel_synthetic(app: &mut SoundApp) {`,
			"host input cancellation",
		);
		write("crates/ui-egui/src/control.rs", control);
	}
	const appType = {
		wordcraft: "WordApp",
		deckcraft: "SlideApp",
		gridcraft: "SheetApp",
		vectorcraft: "VectorcraftApp",
		designcraft: "DesignApp",
		solidcraft: "SolveApp",
		cadcraft: "CadApp",
		soundcraft: "SoundApp",
	}[id];
	const uiPath = "crates/ui-egui/src/lib.rs";
	const cancel =
		id === "soundcraft"
			? "control::cancel_synthetic(self);"
			: `self.synthetic.clear();${id === "designcraft" ? " self.control_synthetic.clear();" : ""}`;
	const cancelInput = `\nimpl ${appType} { pub fn host_cancel_synthetic(&mut self) { ${cancel} } }\n`;
	write(
		uiPath,
		current(uiPath).replace(
			new RegExp(
				`\\nimpl ${appType} \\{ pub fn host_cancel_synthetic[^\\n]*\\n`,
				"g",
			),
			"",
		) + cancelInput,
	);
	write(
		`${path}/src/mcp_policy.rs`,
		readFileSync(join(directory, "mcp-policy.rs"), "utf8"),
	);
	const module = crate.replaceAll("-", "_");
	const definitions =
		id === "cadcraft"
			? 'upstream::tools::tool_definitions(true)["tools"].clone()'
			: id === "wordcraft"
				? "upstream::tools::tool_definitions()"
				: id === "soundcraft"
					? "serde_json::json!(upstream::mcp::tool_definitions())"
					: "serde_json::json!(upstream::tool_definitions())";
	const call =
		id === "soundcraft"
			? "upstream::mcp::Server::new(backend).call_tool(name, args)"
			: id === "cadcraft"
				? "upstream::tools::call_tool(&mut backend, name, args).unwrap_or_else(tool_error)"
				: id === "wordcraft"
					? "upstream::tools::call_tool(&mut backend, name, args).to_value()"
					: id === "solidcraft"
						? "upstream::call_tool(&mut backend, name, args)"
						: "upstream::call_tool(&mut backend, name, args).to_value()";
	write(
		`${path}/src/live_mcp.rs`,
		`${id === "cadcraft" ? "use web_sys::js_sys;\n" : ""}use ${module} as upstream;\nconst EDITOR: &str = "${id}";\nfn definitions() -> serde_json::Value { ${definitions} }\nfn call_upstream(mut backend: LiveBackend<'_>, name: &str, args: &serde_json::Value) -> serde_json::Value { ${call} }\n${readFileSync(join(directory, "live-mcp.rs"), "utf8")}`,
	);
}
