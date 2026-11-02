import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { bridgeBranch, rustFunction } from "./rust-source.mjs";

const directory = dirname(fileURLToPath(import.meta.url));
const toolsByEditor = {
	filmcraft: [
		"command_list",
		"command_run",
		"command_batch",
		"doc_inspect",
		"project_inspect",
		"sequence_inspect",
		"ui_inspect",
		"ui_elements",
		"ui_control",
	],
	photocraft: [
		"command_list",
		"command_run",
		"command_batch",
		"doc_inspect",
		"ui_inspect",
		"ui_pointer",
		"ui_menu_invoke",
		"ui_set",
		"control_call",
		"jobs_list",
		"jobs_cancel",
	],
};

const nativeToolsByEditor = {
	filmcraft: [
		"render_preview",
		"media_import",
		"render_frame",
		"ui_click",
		"ui_drag",
		"ui_key",
		"ui_type",
		"ui_screenshot",
	],
	photocraft: [
		"session_list",
		"doc_open",
		"doc_place",
		"doc_replace_smart_object",
		"doc_export_layers",
		"doc_export_data_sets",
		"doc_import_data_sets",
		"doc_new",
		"doc_save",
		"doc_export",
		"doc_render_preview",
		"render_preview",
		"doc_select",
		"doc_close",
		"ui_screenshot",
	],
};

// These native servers couple rmcp to TCP and blocking headless jobs. Copy the
// pinned parameter types, tool attributes and bridge handlers at build time.
// Unsupported source shapes fail the build instead of silently changing tools.
export function prepareRmcpLive({ id, source, write, original, replaceOnce }) {
	const text = original("crates/automation/src/server.rs");
	const className = id === "filmcraft" ? "FilmcraftMcp" : "PhotocraftMcp";
	const names = toolsByEditor[id];
	if (!names) throw new Error(`No rmcp live adapter for ${id}`);
	const observed = [...text.matchAll(/#\[tool\([\s\S]*?\]\s*async fn (\w+)/g)]
		.map((match) => match[1])
		.sort();
	const reviewed = [...names, ...nativeToolsByEditor[id]].sort();
	if (JSON.stringify(observed) !== JSON.stringify(reviewed))
		throw new Error(
			`The ${id} upstream MCP catalog changed; review its live/native tool boundary.`,
		);

	const firstType = text.indexOf(
		"#[derive(Debug, Default, Deserialize, JsonSchema)]",
	);
	const typesEnd = text.indexOf(
		id === "filmcraft" ? "fn ok_json(" : "// Helpers",
		firstType,
	);
	if (firstType < 0 || typesEnd < 0)
		throw new Error(`The ${id} MCP parameter declarations changed.`);
	let types = text.slice(firstType, typesEnd);
	// Strip the separator preceding the PhotoCraft helper section.
	if (id === "photocraft")
		types = types.slice(
			0,
			types.lastIndexOf(
				"// ---------------------------------------------------------------------------",
			),
		);
	const parameterType = (name) =>
		rustFunction(text, name).match(/Parameters<([A-Za-z0-9_]+)>/)?.[1];
	const toolFunctions = names.map((name) =>
		bridgeBranch(rustFunction(text, name, { attributes: true })),
	);
	const helpers =
		id === "filmcraft"
			? ["ok_json", "fail", "wrap"]
					.map((name) => rustFunction(text, name))
					.join("\n")
			: [
					"strict_schema",
					"strict_node",
					"into_command_rows",
					"ok_json",
					"bounded_tool_result",
					"fail",
					"no_backend",
					"to_result",
					"bridge_only",
				]
					.map((name) => rustFunction(text, name))
					.join("\n");
	const methods =
		id === "filmcraft"
			? `${rustFunction(text, "document")}\n${rustFunction(text, "ui")}\nasync fn run(&self, id: &str, params: Value) -> Result<Value, AutomationError> { self.backend.execute(id, params).await }\nfn bridge_client(&self) -> Option<Arc<BridgeClient>> { Some(self.backend.clone()) }`
			: `${bridgeBranch(rustFunction(text, "run_command"))}\n${bridgeBranch(rustFunction(text, "jobs_call"))}\nfn bridge_client(&self) -> Option<&BridgeClient> { Some(&self.backend) }`;
	const annotated =
		id === "photocraft"
			? rustFunction(
					original("crates/automation/src/server/conventions.rs"),
					"annotated",
				)
			: "fn annotated(tool: rmcp::model::Tool) -> rmcp::model::Tool { tool }";
	const args = names
		.map(
			(name) =>
				`"${name}" => self.${name}(${parameterType(name) ? `Parameters(serde_json::from_value(args).map_err(|e| McpError::invalid_params(e.to_string(), None))?)` : ""}).await,`,
		)
		.join("\n");
	const budget =
		id === "photocraft"
			? `mod budgets { ${original("crates/automation/src/budgets.rs")
					.replaceAll(/^\/\/!.*$/gm, "")
					.replace("use crate::AutomationError;", "use super::AutomationError;")
					.replace(
						"use crate::security::MAX_REQUEST_BYTES;",
						"const MAX_REQUEST_BYTES: usize = 1 << 20;",
					)} }\nuse budgets::{BatchReplyBudget, json_bytes};\n${
					original("crates/automation/src/security.rs").match(
						/pub const MAX_BATCH_STEPS: usize = \d+;/,
					)?.[0] ||
					(
						() => {
							throw new Error("Missing upstream batch ceiling");
						}
					)()
				}`
			: "";
	const strict =
		id === "photocraft"
			? "for route in tool_router.map.values_mut() { let mut schema = (*route.attr.input_schema).clone(); strict_schema(&mut schema); route.attr.input_schema = Arc::new(schema); }"
			: "";
	const module = `${readFileSync(join(directory, "rmcp-live.rs"), "utf8")}\nuse rmcp::model::{Tool, ToolAnnotations};\n${types}\n${budget}\n${helpers}\n${annotated}\n
#[derive(Clone)]
struct ${className} { backend: Arc<BridgeClient>, tool_router: ToolRouter<Self> }
impl ServerHandler for ${className} {}
impl ${className} {
    fn new(tx: mpsc::Sender<ControlCall>) -> Self {
        let mut tool_router = Self::tool_router();
        ${strict}
        Self { backend: Arc::new(BridgeClient { tx }), tool_router }
    }
    ${methods}
    async fn call(&self, name: &str, args: Value) -> Result<CallToolResult, McpError> {
        let tool = self.tool_router.get(name).ok_or_else(|| McpError::invalid_params("Unknown editor tool.", None))?;
        let properties = tool.input_schema.get("properties").and_then(Value::as_object);
        let object = args.as_object().ok_or_else(|| McpError::invalid_params("Tool arguments must be an object.", None))?;
        if let Some(key) = object.keys().find(|key| properties.is_none_or(|p| !p.contains_key(*key))) { return Err(McpError::invalid_params(format!("Unknown argument: {key}"), None)); }
        match name { ${args} _ => Err(McpError::invalid_params("Unknown editor tool.", None)) }
    }
}
#[tool_router]
impl ${className} { ${toolFunctions.join("\n")} }
mod policy { ${readFileSync(join(directory, "mcp-policy.rs"), "utf8")} }
use policy::{file_arguments, allowed_command, command_read_only};
const HOST_FILES: &str = "Use the document controls above the editor for file operations.";
const EDITOR: &str = "${id}";
${readFileSync(join(directory, "rmcp-receive.rs"), "utf8").replaceAll("LiveServer", className)}
`;
	const web = `apps/${id}-web`;
	write(`${web}/src/live_mcp.rs`, module);
	const manifest = `${web}/Cargo.toml`;
	const current = readFileSync(join(source, manifest), "utf8");
	const mcpDependency = original("crates/automation/Cargo.toml").match(
		/^rmcp = .*$/m,
	)?.[0];
	if (!mcpDependency) throw new Error("Missing upstream rmcp dependency.");
	const deps = `${mcpDependency.replace(', "transport-io"', "")}\nschemars = "1"\nuuid = { version = "1", features = ["js"] }\nserde = { workspace = true }\n`;
	write(
		manifest,
		replaceOnce(
			current,
			"[target.'cfg(target_arch = \"wasm32\")'.dependencies]",
			`[target.'cfg(target_arch = "wasm32")'.dependencies]\n${deps}`,
			"browser MCP dependencies",
		),
	);
}
