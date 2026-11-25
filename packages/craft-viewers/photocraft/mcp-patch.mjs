import { prepareRmcpLive } from "../rmcp-live.mjs";
import { rustFunction } from "../rust-source.mjs";

export function preparePhotoMcp(options) {
	const { original, replaceOnce } = options;
	let server = original("crates/automation/src/server.rs");
	const replaceFunction = (name, body) => {
		server = replaceOnce(
			server,
			rustFunction(server, name),
			body,
			`browser MCP ${name}`,
		);
	};
	// Native folder grants and request-progress waiters belong to the desktop
	// transport. Embedded commands run synchronously against the mounted session.
	server = replaceOnce(
		server,
		rustFunction(server, "agent_folders", { attributes: true }),
		"",
		"native folder tool",
	);
	replaceFunction(
		"command_run",
		`async fn command_run(&self, Parameters(p): Parameters<RunParams>) -> Result<CallToolResult, McpError> {
    self.run_command(p.id, p.params.unwrap_or_else(|| json!({})), p.wait.unwrap_or(true)).await
}`,
	);
	replaceFunction(
		"run_command",
		`async fn run_command(&self, id: String, params: Value, wait: bool) -> Result<CallToolResult, McpError> {
    to_result(self.backend.call("engine.execute", json!({"command": id, "params": params, "wait": wait})).await)
}`,
	);
	let batch = rustFunction(server, "batch");
	batch = replaceOnce(
		batch,
		"async fn batch(&self, p: BatchParams, mut waiter: Waiter)",
		"async fn command_batch(&self, Parameters(p): Parameters<BatchParams>)",
		"browser batch signature",
	);
	const start = batch.indexOf("        let pinned = self.target();");
	const end = batch.indexOf(
		"            let was_error = response.is_err();",
		start,
	);
	if (start < 0 || end < 0)
		throw new Error("The PhotoCraft MCP batch waiter changed.");
	batch = `${batch.slice(0, start)}        for s in p.steps {
            let response = self.backend.call("engine.execute", json!({"command": s.id, "params": s.params.unwrap_or_else(|| json!({})), "wait": s.wait.unwrap_or(true)})).await;
${batch.slice(end)}`;
	batch = replaceOnce(
		batch,
		"                    attach_memory_error_details(&mut obj, &error);\n",
		"",
		"browser batch error",
	);
	batch = replaceOnce(
		batch,
		'            waiter.report((i + 1) as f64, total as f64, "").await;\n',
		"",
		"browser batch progress",
	);
	replaceFunction("command_batch", batch);
	server = replaceOnce(
		server,
		"            attach_memory_error_details(&mut obj, &e);\n",
		"",
		"browser command error",
	);
	server = replaceOnce(
		server,
		"        command reports progress, cancelling the request cancels it, and other calls are served meanwhile.",
		"        command finishes before the browser replies.",
		"browser command description",
	);
	prepareRmcpLive({
		...options,
		id: "photocraft",
		original: (path) =>
			path === "crates/automation/src/server.rs" ? server : original(path),
	});
}
