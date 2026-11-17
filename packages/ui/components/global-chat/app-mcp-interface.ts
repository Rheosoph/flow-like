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

export const MCP_CHAT_EVENTS_META = "flow-like.com/chat_events";

export interface McpChatEvent {
	event_type: string;
	payload: unknown;
}

export function extractMcpChatEvents(result: Record<string, unknown>): {
	result: Record<string, unknown>;
	chatEvents: McpChatEvent[];
} {
	const meta = result._meta;
	if (!meta || typeof meta !== "object" || !(MCP_CHAT_EVENTS_META in meta)) {
		return { result, chatEvents: [] };
	}
	const { [MCP_CHAT_EVENTS_META]: captured, ...restMeta } = meta as Record<
		string,
		unknown
	>;
	const { _meta: _removed, ...rest } = result;
	const events = (captured as { events?: unknown } | null)?.events;
	return {
		result: Object.keys(restMeta).length ? { ...rest, _meta: restMeta } : rest,
		chatEvents: Array.isArray(events)
			? events.filter(
					(event): event is McpChatEvent =>
						!!event && typeof event === "object" && "event_type" in event,
				)
			: [],
	};
}

export interface McpToolOutcome {
	status: string;
	message?: string;
	app_id?: string;
	event_id?: string;
	event_type?: string;
	mcp_tool?: string;
	result?: Record<string, unknown>;
	chatEvents?: McpChatEvent[];
}

export async function callMcpAppTool(
	eventState: McpEventState,
	appId: string,
	eventId: string,
	args: Record<string, unknown>,
	beforeDispatch?: () => void,
	options?: { chatEvents?: boolean },
): Promise<McpToolOutcome> {
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
	const raw = await (options?.chatEvents
		? eventState.invokeMcp(
				appId,
				eventId,
				"tools/call",
				params,
				beforeDispatch,
				true,
			)
		: beforeDispatch
			? eventState.invokeMcp(
					appId,
					eventId,
					"tools/call",
					params,
					beforeDispatch,
				)
			: eventState.invokeMcp(appId, eventId, "tools/call", params));
	const { result, chatEvents } = extractMcpChatEvents(raw);
	return {
		status: result.isError === true ? "error" : "ok",
		app_id: appId,
		event_id: eventId,
		event_type: "mcp",
		mcp_tool: name,
		result,
		chatEvents,
	};
}
