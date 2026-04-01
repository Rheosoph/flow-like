/**
 * HTTP Request Example — Demonstrates how to declare permissions and make HTTP requests
 *
 * Permissions tell the runtime which host capabilities your node requires.
 * When users place the node, they see the requested permissions and must
 * consent before execution.
 *
 * This example declares the "http" permission and makes a GET request
 * to a public API.
 */

import {
	type Context,
	type ExecutionResult,
	NodeDefinition,
	PinDefinition,
	PinType,
} from "@flow-like/wasm-sdk-typescript";

// ============================================================================
// Node definition — note `nd.addPermission("http")`
// ============================================================================

export function getDefinition(): NodeDefinition {
	const nd = new NodeDefinition(
		"http_request_example_ts",
		"HTTP Request Example",
		"Fetches a random fact from a public API using HTTP",
		"Examples/HTTP",
	);

	nd.addPermission("http");

	nd.addPin(PinDefinition.inputExec("exec"));
	nd.addPin(
		PinDefinition.inputPin("url", PinType.STRING, {
			defaultValue: "https://httpbin.org/get",
		}),
	);
	nd.addPin(PinDefinition.outputExec("exec_out"));
	nd.addPin(PinDefinition.outputPin("status", PinType.I64));
	nd.addPin(PinDefinition.outputPin("body", PinType.STRING));

	return nd;
}

// ============================================================================
// Run handler — makes an HTTP GET request
// ============================================================================

export function run(ctx: Context): ExecutionResult {
	const url =
		ctx.getString("url", "https://httpbin.org/get") ??
		"https://httpbin.org/get";

	ctx.info(`Making GET request to: ${url}`);

	const response = ctx.httpGet(url);
	if (!response) {
		return ctx.fail("HTTP request failed or permission denied");
	}

	ctx.setOutput("status", response.status);
	ctx.setOutput("body", response.body);
	ctx.info(`Response status: ${response.status}`);

	return ctx.success();
}
