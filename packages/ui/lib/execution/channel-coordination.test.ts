import { afterEach, expect, test, vi } from "vitest";
const mocks = vi.hoisted(() => ({ owner: undefined as unknown }));
vi.mock("./runtime-client", () => ({
	getExecutionClientForChannel: () => mocks.owner,
}));
import { submitInteractionResponse } from "../../components/interfaces/chat-default/respond-interaction";
import { replyToChannel } from "../channel";
import { createDeviceCommandSession } from "../device-bridge";
import type { IChannelHandle } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import type { IInteractionRequest } from "../schema/interaction";
import { executionElementSource, registerExecutionSurface } from "./surfaces";

const handle: IChannelHandle = {
	channel_id: "run",
	request_id: "request",
	expires_at: Math.floor(Date.now() / 1000) + 60,
	transport: {
		type: "http",
		push_url: "https://example.test/reply",
		token: "capability",
	},
};
afterEach(() => {
	mocks.owner = undefined;
	vi.restoreAllMocks();
});

test("a response from a managed tab goes through its shared owner", async () => {
	const submitReply = vi.fn(async () => {});
	mocks.owner = { submitReply };
	const fetcher = vi.spyOn(globalThis, "fetch");
	await replyToChannel(handle, { approved: true });
	expect(submitReply).toHaveBeenCalledWith(handle, { approved: true });
	expect(fetcher).not.toHaveBeenCalled();
});

test("a losing feedback response displays the value already accepted by the shared session", async () => {
	mocks.owner = {
		submitReply: vi.fn(async () => {}),
		getTasks: () => [
			{
				pendingInteractions: [
					{ channel: handle, status: "submitted", response_value: "first tab" },
				],
			},
		],
	};
	const result = await submitInteractionResponse(
		{ id: "question", channel: handle } as IInteractionRequest,
		"second tab",
	);
	expect(result.value).toBe("first tab");
});

test("two mounted device bridges claim a request before either performs its action", async () => {
	let claimed = false;
	const submitReply = vi.fn(async () => {});
	mocks.owner = {
		claimRequest: vi.fn(async () => {
			if (claimed) return false;
			claimed = true;
			return true;
		}),
		submitReply,
	};
	const execute = vi.fn(async () => "done");
	const first = createDeviceCommandSession({ appId: "app" }, { execute });
	const second = createDeviceCommandSession({ appId: "app" }, { execute });
	const event = {
		event_id: "command",
		event_type: "a2ui",
		payload: {
			type: "deviceCommand",
			app_id: "app",
			request_id: "request",
			command: "clipboard.write",
			args: { format: "text", text: "hello" },
			channel: handle,
		},
	} as IIntercomEvent;
	try {
		first.filter([event]);
		second.filter([event]);
		await vi.waitFor(() => expect(submitReply).toHaveBeenCalledTimes(1));
		expect(execute).toHaveBeenCalledTimes(1);
	} finally {
		first.close();
		second.close();
	}
});

test("a disconnected shared owner rejects instead of falling through to a raw reply", async () => {
	mocks.owner = {
		submitReply: vi.fn(async () => {
			throw new Error("Session changed");
		}),
	};
	const fetcher = vi.spyOn(globalThis, "fetch");
	await expect(replyToChannel(handle, "late")).rejects.toThrow(
		"Session changed",
	);
	expect(fetcher).not.toHaveBeenCalled();
});

test("page requests only use a live surface belonging to their app and page", () => {
	const source = {
		surfaceId: "page",
		components: {},
		storedValues: { name: "visible" },
	};
	const close = registerExecutionSurface({
		appId: "app",
		pageId: "page",
		source: () => source,
	});
	try {
		expect(executionElementSource("app", "page")).toBe(source);
		expect(executionElementSource("other", "page")).toBeNull();
		expect(executionElementSource("app", "other")).toBeNull();
		expect(executionElementSource("app")).toBeNull();
	} finally {
		close();
	}
	expect(executionElementSource("app", "page")).toBeNull();
});

test("a widget request reads only its matching mounted instance", () => {
	const source = {
		surfaceId: "page",
		components: {},
		storedValues: {},
		widgetScope: { instanceId: "widget", components: [] },
	};
	const close = registerExecutionSurface({
		appId: "app",
		pageId: "page",
		source: () => source,
	});
	try {
		expect(executionElementSource("app", "page", "widget")).toBe(source);
		expect(executionElementSource("app", "page", "another-widget")).toBeNull();
	} finally {
		close();
	}
});
