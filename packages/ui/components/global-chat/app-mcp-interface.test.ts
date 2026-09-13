import { describe, expect, test } from "bun:test";
import { callMcpAppTool, describeMcpAppInterface } from "./app-mcp-interface";

function fixture(result: Record<string, unknown>) {
	const calls: unknown[][] = [];
	return {
		calls,
		state: {
			async invokeMcp(
				...args: [string, string, string, Record<string, unknown>?]
			) {
				calls.push(args);
				return result;
			},
		},
	};
}

describe("FlowPilot MCP app interfaces", () => {
	test("retains a live scope guard until the MCP transport is ready", async () => {
		const ready = Promise.withResolvers<void>();
		let current = true;
		let dispatched = false;
		const pending = callMcpAppTool(
			{
				async invokeMcp(_app, _event, _method, _params, beforeDispatch) {
					await ready.promise;
					beforeDispatch?.();
					dispatched = true;
					return {};
				},
			},
			"app",
			"event",
			{ mcp_tool: "list_notes" },
			() => {
				if (!current) throw new Error("Account changed");
			},
		);
		current = false;
		ready.resolve();
		await expect(pending).rejects.toThrow("Account changed");
		expect(dispatched).toBe(false);
	});
	test("describes registered tool schemas including a tool with no arguments", async () => {
		const tools = [
			{ name: "list_notes", inputSchema: { type: "object", properties: {} } },
			{
				name: "find_notes",
				inputSchema: {
					type: "object",
					properties: { query: { type: "string" } },
				},
			},
		];
		const f = fixture({ tools });
		const result = await describeMcpAppInterface(f.state, "app", "event");
		expect(f.calls).toEqual([["app", "event", "tools/list"]]);
		expect(result.tools).toEqual(tools);
		expect(result.invocation).toMatchObject({
			consumer_tool: "call_app_event",
			app_id: "app",
			event_id: "event",
		});
	});

	test("calls a zero-argument MCP tool without a payload pin or server entry run", async () => {
		const response = {
			content: [{ type: "text", text: "[]" }],
			isError: false,
		};
		const f = fixture(response);
		const result = await callMcpAppTool(f.state, "app", "event", {
			mcp_tool: "list_notes",
		});
		expect(f.calls).toEqual([
			["app", "event", "tools/call", { name: "list_notes", arguments: {} }],
		]);
		expect(result).toMatchObject({ status: "ok", result: response });
	});

	test("passes typed arguments through and preserves MCP tool failures", async () => {
		const response = {
			content: [{ type: "text", text: "Notes unavailable" }],
			isError: true,
		};
		const f = fixture(response);
		const result = await callMcpAppTool(f.state, "app", "event", {
			mcp_tool: "find_notes",
			payload: { query: "meeting", limit: 0 },
		});
		expect(f.calls).toEqual([
			[
				"app",
				"event",
				"tools/call",
				{ name: "find_notes", arguments: { query: "meeting", limit: 0 } },
			],
		]);
		expect(result).toMatchObject({ status: "error", result: response });
	});

	test("rejects a generic server payload and malformed arguments before dispatch", async () => {
		const f = fixture({});
		for (const args of [
			{ payload: { action: "list_notes" } },
			{ mcp_tool: "list_notes", payload: [] },
			{ mcp_tool: "list_notes", payload: null },
		]) {
			expect((await callMcpAppTool(f.state, "app", "event", args)).status).toBe(
				"error",
			);
		}
		expect(f.calls).toHaveLength(0);
	});

	test("does not turn failed discovery into an empty inventory", async () => {
		const f = fixture({ error: "unavailable" });
		await expect(
			describeMcpAppInterface(f.state, "app", "event"),
		).rejects.toThrow("did not return a tool inventory");
		await expect(describeMcpAppInterface({}, "app", "event")).rejects.toThrow(
			"unavailable",
		);
	});
});
