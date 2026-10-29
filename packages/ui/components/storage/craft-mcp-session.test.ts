import {
	type Mock,
	afterEach,
	beforeEach,
	describe,
	expect,
	it,
	vi,
} from "vitest";
import { executeDeviceCommand } from "../../lib/device-bridge";
import { CraftMcpSession, executeCraftCommand } from "./craft-mcp-session";

const target = { appId: "app-a", surfaceId: "page", componentId: "document" };
const context = { appId: target.appId };
const elementRef = "page/document";
const message = { jsonrpc: "2.0", id: 1, method: "tools/list", params: {} };
let session: CraftMcpSession;
let frame: Window;
let post: Mock<(message: Record<string, unknown>) => void>;
let state: {
	ready: boolean;
	editable: boolean;
	editing: boolean;
	busy: boolean;
};

function createSession(sessionId = "session-a") {
	return new CraftMcpSession({
		target,
		sessionId,
		editor: "solidcraft",
		channel: "flow-like-craft",
		origin: "http://localhost:3000",
		frame: () => frame,
		post,
		state: () => state,
	});
}

function args(request: unknown = message) {
	return {
		elementRef,
		sessionId: session.options.sessionId,
		editor: "solidcraft",
		message: request,
	};
}

function receive(
	response: unknown,
	overrides: Partial<MessageEvent> = {},
	extra: Record<string, unknown> = {},
) {
	session.receive({
		origin: "http://localhost:3000",
		source: frame,
		data: {
			type: "mcp-response",
			channel: "flow-like-craft",
			session: session.options.sessionId,
			requestId: post.mock.calls.at(-1)?.[0].requestId,
			response,
			...extra,
		},
		...overrides,
	} as MessageEvent);
}

beforeEach(() => {
	post = vi.fn<(message: Record<string, unknown>) => void>();
	frame = {} as Window;
	state = { ready: true, editable: true, editing: true, busy: false };
	session = createSession();
});

afterEach(() => {
	session.dispose();
	vi.useRealTimers();
	vi.restoreAllMocks();
});

describe("live Craft MCP sessions", () => {
	it("resolves only the exact app and component, then requires its pinned session and editor", async () => {
		await expect(
			executeCraftCommand("craft.session", { elementRef }, context),
		).resolves.toEqual({ sessionId: "session-a", editor: "solidcraft" });
		await expect(
			executeCraftCommand(
				"craft.session",
				{ elementRef },
				{ appId: "another-app" },
			),
		).rejects.toMatchObject({ code: "session_unavailable" });
		await expect(
			executeCraftCommand("craft.session", { elementRef: "document" }, context),
		).rejects.toMatchObject({ code: "invalid_target" });
		await expect(
			executeCraftCommand(
				"craft.mcp",
				{ ...args(), sessionId: "old-session" },
				context,
			),
		).rejects.toMatchObject({ code: "session_changed" });
		await expect(
			executeCraftCommand(
				"craft.mcp",
				{ ...args(), editor: "wordcraft" },
				context,
			),
		).rejects.toMatchObject({ code: "session_changed" });
		expect(post).not.toHaveBeenCalled();
	});

	it.each([
		{ visibility: "visible", method: "tools/list", params: {} },
		{ visibility: "hidden", method: "tools/list", params: {} },
		{
			visibility: "hidden",
			method: "tools/call",
			params: { name: "execute", arguments: { command: "solid.box" } },
		},
	] as const)(
		"forwards $method through the device bridge while $visibility",
		async ({ visibility, method, params }) => {
			vi.spyOn(document, "visibilityState", "get").mockReturnValue(visibility);
			await expect(
				executeDeviceCommand("craft.session", { elementRef }, context),
			).resolves.toEqual({ sessionId: "session-a", editor: "solidcraft" });
			const request = { ...message, method, params };
			const deadline = Date.now() + 10_000;
			const reply = executeDeviceCommand("craft.mcp", args(request), {
				...context,
				deadline,
			});
			await vi.dynamicImportSettled();
			expect(post).toHaveBeenCalledWith({
				type: "mcp-request",
				requestId: expect.any(String),
				message: request,
				deadlineAtMs: deadline,
			});
			const response = {
				jsonrpc: "2.0",
				id: 1,
				result: { tools: [], nextCursor: "next" },
			};
			receive(response);
			await expect(reply).resolves.toBe(response);
		},
	);

	it.each(["craft.session", "craft.mcp"])(
		"rejects aborted and expired %s requests through the background device bridge",
		async (command) => {
			vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
			const controller = new AbortController();
			controller.abort();
			await expect(
				executeDeviceCommand(command, args(), {
					...context,
					signal: controller.signal,
				}),
			).rejects.toMatchObject({ code: "cancelled" });
			await expect(
				executeDeviceCommand(command, args(), {
					...context,
					deadline: Date.now() - 1,
				}),
			).rejects.toMatchObject({ code: "expired" });
			expect(post).not.toHaveBeenCalled();
		},
	);

	it.each(["craft.session", "craft.mcp"])(
		"requires an open document for %s while the app is hidden",
		async (command) => {
			vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
			state.ready = false;
			await expect(
				executeDeviceCommand(command, args(), context),
			).rejects.toMatchObject({ code: "session_closed" });
			session.dispose();
			await expect(
				executeDeviceCommand(command, args(), context),
			).rejects.toMatchObject({ code: "session_unavailable" });
			expect(post).not.toHaveBeenCalled();
		},
	);

	it("allows discovered read-only tools while refusing mutation until editing is enabled", async () => {
		state.editing = false;
		const listing = executeCraftCommand("craft.mcp", args(), context);
		receive({
			jsonrpc: "2.0",
			id: 1,
			result: {
				tools: [
					{ name: "inspect", annotations: { readOnlyHint: true } },
					{ name: "execute", annotations: { readOnlyHint: false } },
				],
			},
		});
		await listing;
		const read = {
			jsonrpc: "2.0",
			id: 2,
			method: "tools/call",
			params: { name: "inspect", arguments: {} },
		};
		const reading = executeCraftCommand("craft.mcp", args(read), context);
		receive({ jsonrpc: "2.0", id: 2, result: { content: [] } });
		await expect(reading).resolves.toMatchObject({ id: 2 });
		const mutate = { ...read, params: { name: "execute", arguments: {} } };
		await expect(
			executeCraftCommand("craft.mcp", args(mutate), context),
		).rejects.toMatchObject({ code: "read_only" });
		state.editing = true;
		const writing = executeCraftCommand("craft.mcp", args(mutate), context);
		receive({ jsonrpc: "2.0", id: 2, result: { content: [] } });
		await writing;
		state.editable = false;
		await expect(
			executeCraftCommand("craft.mcp", args(mutate), context),
		).rejects.toMatchObject({ code: "read_only" });
	});

	it("rejects mutation while a save is pending and leaves reads available", async () => {
		state.busy = true;
		await expect(
			executeCraftCommand(
				"craft.mcp",
				args({ ...message, method: "tools/call", params: { name: "execute" } }),
				context,
			),
		).rejects.toMatchObject({ code: "busy" });
		const listing = executeCraftCommand("craft.mcp", args(), context);
		receive({ jsonrpc: "2.0", id: 1, result: { tools: [] } });
		await listing;
	});

	it("accepts null acknowledgements for MCP notifications", async () => {
		state.editing = false;
		const notify = executeCraftCommand(
			"craft.mcp",
			args({ jsonrpc: "2.0", method: "notifications/initialized" }),
			context,
		);
		receive(null);
		await expect(notify).resolves.toBeNull();
	});

	it("acknowledges cancellation notifications while read-only and saving", async () => {
		state.editable = false;
		state.editing = false;
		state.busy = true;
		const notification = {
			jsonrpc: "2.0",
			method: "notifications/cancelled",
			params: { requestId: 42, reason: "The caller stopped waiting." },
		};
		const notify = executeCraftCommand(
			"craft.mcp",
			args(notification),
			context,
		);
		expect(post).not.toHaveBeenCalled();
		await expect(notify).resolves.toBeNull();
	});

	it("cancels only the matching wire JSON-RPC request immediately", async () => {
		const first = executeCraftCommand(
			"craft.mcp",
			args({ ...message, id: "agent-a:1" }),
			context,
		);
		const firstRequestId = post.mock.calls.at(-1)?.[0].requestId;
		const cancelled = expect(first).rejects.toMatchObject({
			code: "cancelled",
		});
		const second = executeCraftCommand(
			"craft.mcp",
			args({ ...message, id: "agent-b:1" }),
			context,
		);
		const secondRequestId = post.mock.calls.at(-1)?.[0].requestId;
		state.editable = false;
		state.editing = false;
		state.busy = true;
		await expect(
			executeCraftCommand(
				"craft.mcp",
				args({
					jsonrpc: "2.0",
					method: "notifications/cancelled",
					params: { requestId: "agent-a:1" },
				}),
				context,
			),
		).resolves.toBeNull();
		await cancelled;
		expect(post.mock.calls.at(-1)?.[0]).toEqual({
			type: "mcp-cancel",
			requestId: firstRequestId,
		});
		receive(
			{ jsonrpc: "2.0", id: "agent-b:1", result: { tools: [] } },
			{},
			{ requestId: secondRequestId },
		);
		await expect(second).resolves.toMatchObject({ id: "agent-b:1" });
	});

	it("rejects a cancelled request that arrives late while another peer can still edit", async () => {
		await executeCraftCommand(
			"craft.mcp",
			args({
				jsonrpc: "2.0",
				method: "notifications/cancelled",
				params: { requestId: "agent-a:1" },
			}),
			context,
		);
		const edit = {
			jsonrpc: "2.0",
			id: "agent-a:1",
			method: "tools/call",
			params: { name: "execute", arguments: { command: "solid.box" } },
		};
		await expect(
			executeCraftCommand("craft.mcp", args(edit), context),
		).rejects.toMatchObject({ code: "cancelled" });
		expect(post).not.toHaveBeenCalled();
		const other = executeCraftCommand(
			"craft.mcp",
			args({ ...edit, id: "agent-b:1" }),
			context,
		);
		expect(post).toHaveBeenCalledOnce();
		receive({ jsonrpc: "2.0", id: "agent-b:1", result: { content: [] } });
		await expect(other).resolves.toMatchObject({ id: "agent-b:1" });
	});

	it("expires cancellation tombstones after 120 seconds", async () => {
		vi.useFakeTimers();
		await executeCraftCommand(
			"craft.mcp",
			args({
				jsonrpc: "2.0",
				method: "notifications/cancelled",
				params: { requestId: message.id },
			}),
			context,
		);
		await vi.advanceTimersByTimeAsync(119_999);
		await expect(
			executeCraftCommand("craft.mcp", args(), context),
		).rejects.toMatchObject({ code: "cancelled" });
		expect(post).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);
		const request = executeCraftCommand("craft.mcp", args(), context);
		receive({ jsonrpc: "2.0", id: message.id, result: { tools: [] } });
		await expect(request).resolves.toMatchObject({ id: message.id });
	});

	it("blocks dispatch instead of evicting cancellations when the cache fills", async () => {
		vi.useFakeTimers();
		for (let id = 0; id <= 512; id++) {
			await executeCraftCommand(
				"craft.mcp",
				args({
					jsonrpc: "2.0",
					method: "notifications/cancelled",
					params: { requestId: `agent-a:${id}` },
				}),
				context,
			);
		}
		await expect(
			executeCraftCommand(
				"craft.mcp",
				args({ ...message, id: "agent-a:0" }),
				context,
			),
		).rejects.toMatchObject({ code: "cancelled" });
		await expect(
			executeCraftCommand(
				"craft.mcp",
				args({ ...message, id: "agent-a:512" }),
				context,
			),
		).rejects.toMatchObject({ code: "busy" });
		expect(post).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(120_000);
		const request = executeCraftCommand("craft.mcp", args(), context);
		receive({ jsonrpc: "2.0", id: message.id, result: { tools: [] } });
		await request;
	});

	it("ignores responses from another frame, origin, or document session", async () => {
		const result = { jsonrpc: "2.0", id: 1, result: { tools: [] } };
		const completion = vi.fn();
		const reply = executeCraftCommand("craft.mcp", args(), context).then(
			completion,
		);
		receive(result, { origin: "https://another.example" });
		receive(result, { source: {} as Window });
		receive(result, {}, { session: "another-session" });
		receive(result, {}, { requestId: "another-request" });
		await Promise.resolve();
		expect(completion).not.toHaveBeenCalled();
		receive(result);
		await reply;
		expect(completion).toHaveBeenCalledExactlyOnceWith(result);
	});

	it("rejects pending calls when the mounted document is replaced", async () => {
		const reply = executeCraftCommand("craft.mcp", args(), context);
		const rejected = expect(reply).rejects.toMatchObject({
			code: "session_closed",
		});
		const previous = session;
		session = createSession("replacement");
		await rejected;
		previous.dispose();
		await expect(
			executeCraftCommand("craft.session", { elementRef }, context),
		).resolves.toMatchObject({ sessionId: "replacement" });
		await expect(
			executeCraftCommand(
				"craft.mcp",
				{ ...args(), sessionId: "session-a" },
				context,
			),
		).rejects.toMatchObject({ code: "session_changed" });
	});

	it("rejects aborted and expired requests without dispatching them", async () => {
		const controller = new AbortController();
		controller.abort();
		await expect(
			executeCraftCommand("craft.mcp", args(), {
				...context,
				signal: controller.signal,
			}),
		).rejects.toMatchObject({ code: "cancelled" });
		await expect(
			executeCraftCommand("craft.mcp", args(), {
				...context,
				deadline: Date.now() - 1,
			}),
		).rejects.toMatchObject({ code: "expired" });
		expect(post).not.toHaveBeenCalled();
	});

	it("bounds the wait and ignores late replies", async () => {
		vi.useFakeTimers();
		const reply = executeCraftCommand("craft.mcp", args(), {
			...context,
			deadline: Date.now() + 50,
		});
		const rejected = expect(reply).rejects.toMatchObject({ code: "timeout" });
		await vi.advanceTimersByTimeAsync(50);
		await rejected;
		expect(post.mock.calls.at(-1)?.[0]).toMatchObject({
			type: "mcp-cancel",
			requestId: expect.any(String),
		});
		receive({ jsonrpc: "2.0", id: 1, result: { tools: [] } });
	});

	it.each(["visible", "hidden"] as const)(
		"cancels queued work through the device bridge while %s",
		async (visibility) => {
			vi.spyOn(document, "visibilityState", "get").mockReturnValue(visibility);
			const controller = new AbortController();
			const reply = executeDeviceCommand("craft.mcp", args(), {
				...context,
				signal: controller.signal,
			});
			const rejected = expect(reply).rejects.toMatchObject({
				code: "cancelled",
			});
			await vi.dynamicImportSettled();
			const requestId = post.mock.calls[0][0].requestId;
			controller.abort();
			await rejected;
			expect(post.mock.calls.at(-1)?.[0]).toEqual({
				type: "mcp-cancel",
				requestId,
			});
		},
	);

	it("rejects a response whose JSON-RPC id does not match the pending request", async () => {
		const reply = executeCraftCommand("craft.mcp", args(), context);
		receive({ jsonrpc: "2.0", id: 2, result: { tools: [] } });
		await expect(reply).rejects.toMatchObject({ code: "invalid_response" });
	});

	it("bounds payloads and rejects malformed JSON-RPC before dispatch", async () => {
		await expect(
			executeCraftCommand(
				"craft.mcp",
				args({ ...message, params: { text: "x".repeat(1024 * 1024) } }),
				context,
			),
		).rejects.toMatchObject({ code: "payload_too_large" });
		await expect(
			executeCraftCommand("craft.mcp", args({ method: "tools/list" }), context),
		).rejects.toMatchObject({ code: "invalid_request" });
		expect(post).not.toHaveBeenCalled();
	});
});
