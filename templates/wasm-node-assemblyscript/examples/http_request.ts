/**
 * HTTP Request Node — Demonstrates declaring HTTP permissions (AssemblyScript)
 *
 * This example shows how to declare the "http" permission and use the host
 * function to make outbound HTTP requests from an AssemblyScript WASM node.
 * Register this node in your examples.ts or index.ts entry point.
 */

import { httpRequest } from "@flow-like/wasm-sdk-assemblyscript/assembly/host";
import {
	type Context,
	DataType,
	type ExecutionResult,
	FlowNode,
	NodeDefinition,
	PinDefinition,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

// ============================================================================
// Node class — note `def.addPermission("http")`
// ============================================================================

export class HttpGetRequestNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "http_get_request_as";
		def.friendly_name = "HTTP GET Request (AS)";
		def.description = "Sends a GET request to a URL and reports the result";
		def.category = "Network/HTTP";
		def.addPermission("http");

		def.addPin(
			PinDefinition.input(
				"exec",
				"Execute",
				"Trigger execution",
				DataType.Exec,
			),
		);
		def.addPin(
			PinDefinition.input(
				"url",
				"URL",
				"Target URL",
				DataType.String,
			).withDefaultString("https://httpbin.org/get"),
		);
		def.addPin(
			PinDefinition.input(
				"headers_json",
				"Headers (JSON)",
				"Request headers as JSON",
				DataType.String,
			).withDefaultString("{}"),
		);
		def.addPin(
			PinDefinition.output(
				"exec_out",
				"Done",
				"Fires after the request",
				DataType.Exec,
			),
		);
		def.addPin(
			PinDefinition.output(
				"success",
				"Success",
				"Whether the HTTP call was accepted",
				DataType.Bool,
			),
		);

		return def;
	}

	execute(ctx: Context): ExecutionResult {
		const url = ctx.getString("url");
		const headers = ctx.getString("headers_json");

		// Method 0 = GET.  The host checks the "http" capability.
		const ok = httpRequest(0, url, headers, "");

		ctx.setBool("success", ok);
		return ctx.success();
	}
}
