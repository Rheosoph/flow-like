import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import { describe, expect, test, vi } from "vitest";
import {
	createNativeEventOutput,
	executeNativeEventWithResult,
} from "../native-event-execution";

const mocks = vi.hoisted(() => ({
	context: vi.fn(async () => ({ _route: "/" })),
}));
vi.mock("@flow-like/flow-like-ui/components/a2ui/workflow-payload", () => ({
	buildWorkflowFrontendContext: mocks.context,
}));

const event = (event_type = "simple_chat", overrides: Partial<IEvent> = {}) =>
	({
		id: "event",
		node_id: "entry-node",
		name: "Assistant",
		event_type,
		inputs: [],
		...overrides,
	}) as IEvent;
const stream = (
	event_type: string,
	payload: unknown,
	event_id = crypto.randomUUID(),
) =>
	({
		event_id,
		event_type,
		payload,
		timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	}) as IIntercomEvent;
const response = (text: string) => ({
	response: {
		choices: [
			{
				index: 0,
				finish_reason: "stop",
				message: {
					role: "assistant",
					content: text,
					reasoning: "private reasoning",
				},
			},
		],
		usage: { completion_tokens: 0, prompt_tokens: 0, total_tokens: 0 },
	},
	attachments: [],
	widgets: [],
	actions: [],
	local_session: {},
	global_session: {},
});
const request = () => ({
	id: crypto.randomUUID(),
	scope: "account",
	responseMode: "text" as const,
	responseDeadline: new Date(Date.now() + 90_000).toISOString(),
	action: { kind: "run_event" as const, appId: "app", eventId: "event" },
});

describe("native Event output protocol", () => {
	test("chat returns the final visible answer without reasoning or duplicate streaming text", () => {
		const output = createNativeEventOutput("app", event(), "session");
		const partial = stream("chat_stream", response("Draft"));
		output.add([partial]);
		output.add([partial, stream("chat_out", response("Antwort 東京"))]);
		expect(output.result()).toEqual({ text: "Antwort 東京" });
	});

	test.each([
		false,
		null,
		42,
		"A&B",
		[1, 2],
		{ count: 2, nested: { ok: true } },
	])("Return Generic Result preserves the actual JSON value %j", (value) => {
		const output = createNativeEventOutput(
			"app",
			event("generic_form"),
			"session",
		);
		output.add([stream("generic_result", value)]);
		expect(output.result()).toEqual({
			text: typeof value === "string" ? value : "",
			json: JSON.stringify(value),
		});
	});

	test("diagnostic events and run payloads never become the Shortcut output", () => {
		const output = createNativeEventOutput("app", event("api"), "session");
		output.add([
			stream("log", { payload: { token: "secret" } }),
			stream("completed", {
				status: "completed",
				payload: { password: "secret" },
			}),
		]);
		expect(output.result()).toEqual({ text: "" });
	});

	test("a streamed workflow error rejects an earlier partial result", () => {
		const output = createNativeEventOutput("app", event("api"), "session");
		output.add([
			stream("generic_result", "partial"),
			stream("error", { internal: "private stack" }),
		]);
		expect(() => output.result()).toThrow("The Event failed");
	});

	test("oversize output fails instead of silently truncating a usable value", () => {
		const output = createNativeEventOutput("app", event("api"), "session");
		output.add([stream("generic_result", "あ".repeat(70_000))]);
		expect(() => output.result()).toThrow("too large");
	});
});

describe("direct native Event execution", () => {
	test("sends canonical chat input once and waits for the completed response", async () => {
		const executeEvent = vi.fn(async (_id, options) => {
			options.beforeDispatch();
			options.onLiveEvents([
				stream("chat_out", response("Ready for Notes")),
				stream("completed", { status: "completed" }),
			]);
		});
		const nativeRequest = request();
		const result = await executeNativeEventWithResult({
			request: nativeRequest,
			appId: "app",
			event: event(),
			input: "Test",
			backend: {} as IBackendState,
			engine: { executeEvent },
			isCurrent: () => true,
		});
		expect(result).toEqual({ text: "Ready for Notes" });
		expect(executeEvent).toHaveBeenCalledOnce();
		expect(executeEvent.mock.calls[0]).toEqual([
			`native-${nativeRequest.id}`,
			expect.objectContaining({
				payload: {
					id: "entry-node",
					payload: {
						chat_id: `native-${nativeRequest.id}`,
						messages: [{ role: "user", content: "Test" }],
						local_session: {},
						global_session: {},
						attachments: [],
						actions: [],
						tools: [],
						_route: "/",
					},
				},
			}),
		]);
	});

	test("missing required generic inputs do not start the Event", async () => {
		const executeEvent = vi.fn();
		await expect(
			executeNativeEventWithResult({
				request: request(),
				appId: "app",
				event: event("generic_form", {
					inputs: [
						{
							name: "query",
							data_type: "String",
							value_type: "Normal",
							optional: false,
						} as never,
					],
				}),
				backend: {} as IBackendState,
				engine: { executeEvent },
				isCurrent: () => true,
			}),
		).rejects.toThrow("needs more inputs");
		expect(executeEvent).not.toHaveBeenCalled();
	});

	test("account changes during frontend context preparation prevent dispatch", async () => {
		let current = true;
		mocks.context.mockImplementationOnce(async () => {
			current = false;
			return { _route: "/" };
		});
		const executeEvent = vi.fn();
		await expect(
			executeNativeEventWithResult({
				request: request(),
				appId: "app",
				event: event(),
				input: "Test",
				backend: {} as IBackendState,
				engine: { executeEvent },
				isCurrent: () => current,
			}),
		).rejects.toThrow("account or workspace changed");
		expect(executeEvent).not.toHaveBeenCalled();
	});

	test.each(["invalid", new Date(0).toISOString()])(
		"expired or malformed deadline %s prevents execution",
		async (responseDeadline) => {
			const executeEvent = vi.fn();
			await expect(
				executeNativeEventWithResult({
					request: { ...request(), responseDeadline },
					appId: "app",
					event: event(),
					input: "Test",
					backend: {} as IBackendState,
					engine: { executeEvent },
					isCurrent: () => true,
				}),
			).rejects.toThrow("expired");
			expect(executeEvent).not.toHaveBeenCalled();
		},
	);
});

test.each(["failed", "cancelled", "unknown", undefined])(
	"terminal status %s rejects a partial output",
	(status) => {
		const output = createNativeEventOutput("app", event("api"), "session");
		output.add([
			stream("generic_result", { total: 42 }),
			stream("completed", { status }),
		]);
		expect(() => output.result()).toThrow("did not complete successfully");
	},
);

test("a disconnected remote stream cannot return a partial answer as success", async () => {
	const executeEvent = vi.fn(async (_id, options) => {
		options.onLiveEvents([stream("chat_stream", response("Partial"))]);
	});
	await expect(
		executeNativeEventWithResult({
			request: request(),
			appId: "app",
			event: event(),
			input: "Test",
			backend: {} as IBackendState,
			engine: { executeEvent },
			isCurrent: () => true,
		}),
	).rejects.toThrow("before completion was confirmed");
});
