import type { IEventState } from "../../state/backend-state/event-state";
import { argObject, argString } from "./tools/tool-arguments";

type McpEventState = Pick<IEventState, "invokeMcp">;

export function isMcpAppEvent(eventType: string): boolean {
	return eventType.trim().toLowerCase() === "mcp";
}

/** Discover the registered functions, whose inputs differ from the server entry's pins. */
export async function describeMcpAppInterface(
	eventState: McpEventState,
	appId: string,
	eventId: string,
) {
	if (!eventState.invokeMcp) {
		throw new Error("MCP interface discovery is unavailable on this backend.");
	}
	const result = await eventState.invokeMcp(appId, eventId, "tools/list");
	if (!Array.isArray(result.tools)) {
		throw new Error("MCP tools/list did not return a tool inventory.");
	}
	return {
		tools: result.tools,
		invocation: {
			consumer_tool: "call_app_event",
			app_id: appId,
			event_id: eventId,
			instructions:
				"Set mcp_tool to an exact tools[].name and payload to the arguments described by that tool's inputSchema. Omit payload or pass {} for a tool with no arguments. The runtime fills the internal payload and _client pins automatically.",
		},
	};
}

export async function callMcpAppTool(
	eventState: McpEventState,
	appId: string,
	eventId: string,
	args: Record<string, unknown>,
	beforeDispatch?: () => void,
) {
	const name = argString(args, "mcp_tool");
	if (!name) {
		return {
			status: "error",
			message:
				"This is an MCP server interface. Read describe_app_interface, then set mcp_tool to an exact tool name and payload to its arguments ({} for no arguments).",
		};
	}
	if (args.payload !== undefined && !argObject(args, "payload")) {
		return {
			status: "error",
			message: "MCP tool arguments must be an object.",
		};
	}
	if (!eventState.invokeMcp) {
		return {
			status: "error",
			message: "MCP tool execution is unavailable on this backend.",
		};
	}
	const params = {
		name,
		arguments: argObject(args, "payload") ?? {},
	};
	beforeDispatch?.();
	const result = await (beforeDispatch
		? eventState.invokeMcp(appId, eventId, "tools/call", params, beforeDispatch)
		: eventState.invokeMcp(appId, eventId, "tools/call", params));
	return {
		status: result.isError === true ? "error" : "ok",
		app_id: appId,
		event_id: eventId,
		event_type: "mcp",
		mcp_tool: name,
		result,
	};
}
