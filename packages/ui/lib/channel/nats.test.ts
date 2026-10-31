import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pushHttp } from "./http";
import type { NatsChannelDescriptor } from "./nats";
import { pushNats, resetNatsConnections } from "./nats";
import { rawPushChannel } from "./transport";

const sdk = vi.hoisted(() => ({
	connect: vi.fn(),
	auth: vi.fn((token) => ({ jwt: token })),
}));
vi.mock("@nats-io/nats-core", () => ({
	wsconnect: sdk.connect,
	jwtAuthenticator: sdk.auth,
}));

type Callback = (
	error: Error | null,
	message: { json(): unknown; headers?: { code: number } },
) => void;
class FakeConnection {
	operations: string[] = [];
	callbacks = new Map<string, Callback>();
	published: Array<{
		subject: string;
		body: { id: string; push: unknown };
		reply: string;
	}> = [];
	closedPromise: Promise<void>;
	finish!: () => void;
	onPublish: ((reply: string) => void) | undefined = (reply) =>
		this.ack(reply, { accepted: true });
	constructor() {
		this.closedPromise = new Promise((resolve) => {
			this.finish = resolve;
		});
	}
	subscribe(subject: string, options: { callback: Callback }) {
		this.operations.push("subscribe");
		this.callbacks.set(subject, options.callback);
		return { unsubscribe: () => this.callbacks.delete(subject) };
	}
	async flush() {
		this.operations.push("flush");
	}
	publish(subject: string, data: Uint8Array, options: { reply: string }) {
		this.operations.push("publish");
		this.published.push({
			subject,
			body: JSON.parse(new TextDecoder().decode(data)),
			reply: options.reply,
		});
		this.onPublish?.(options.reply);
	}
	ack(reply: string, value: unknown) {
		this.callbacks.get(reply)?.(null, { json: () => value });
	}
	closed() {
		return this.closedPromise;
	}
	async close() {
		this.finish();
	}
}

const push = {
	channel_id: "run-1",
	request_id: "request-1",
	kind: "reply" as const,
	value: { approved: true },
};
const descriptor = (): NatsChannelDescriptor => ({
	type: "nats",
	url: "wss://channels.example/ws",
	subject: "channel.run-1.inbox",
	token: "signed-grant",
	expires_at: Math.floor(Date.now() / 1000) + 60,
});
let clients: FakeConnection[];
beforeEach(() => {
	clients = [];
	sdk.connect.mockReset().mockImplementation(async () => {
		const client = new FakeConnection();
		clients.push(client);
		return client;
	});
	sdk.auth.mockClear();
});
afterEach(() => {
	resetNatsConnections();
	vi.useRealTimers();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe("NATS channel delivery", () => {
	it("reports an HTTP forwarder rejection even when the API returns 200", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn(async () =>
				Response.json({ accepted: false, message: "No executor is listening" }),
			),
		);
		await expect(
			pushHttp(
				{ type: "http", push_url: "https://api.example/push", token: "grant" },
				push,
				{ deliveryId: "fallback-id" },
			),
		).rejects.toThrow("No executor is listening");
	});
	it("waits for the acknowledgement subscription before publishing and reuses the connection", async () => {
		await pushNats(descriptor(), push, { deliveryId: "delivery-1" });
		await pushNats(
			descriptor(),
			{ ...push, kind: "inbound" },
			{ deliveryId: "delivery-2" },
		);
		expect(sdk.connect).toHaveBeenCalledTimes(1);
		expect(sdk.auth).toHaveBeenCalledWith("signed-grant");
		expect(clients[0].operations).toEqual([
			"subscribe",
			"flush",
			"publish",
			"subscribe",
			"flush",
			"publish",
		]);
		expect(clients[0].published[0]).toEqual({
			subject: descriptor().subject,
			body: { id: "delivery-1", push },
			reply: "channel.run-1.inbox.ack.delivery-1",
		});
		expect(clients[0].callbacks.size).toBe(0);
	});

	it("reports executor rejection without retrying the broker", async () => {
		const client = new FakeConnection();
		client.onPublish = (reply) =>
			client.ack(reply, { accepted: false, error: "Request already answered" });
		sdk.connect.mockResolvedValue(client);
		await expect(pushNats(descriptor(), push)).rejects.toThrow(
			"Request already answered",
		);
		expect(sdk.connect).toHaveBeenCalledTimes(1);
		expect(client.callbacks.size).toBe(0);
	});

	it("does not publish when the broker rejects its acknowledgement subscription", async () => {
		const client = new FakeConnection();
		client.flush = async () => {
			for (const callback of client.callbacks.values())
				callback(new Error("permission denied"), { json: () => null });
		};
		sdk.connect.mockResolvedValue(client);
		await expect(pushNats(descriptor(), push)).rejects.toThrow(
			"subscription failed",
		);
		expect(client.published).toHaveLength(0);
	});

	it("retries an unacknowledged message with the same delivery ID", async () => {
		vi.useFakeTimers();
		const first = new FakeConnection();
		first.onPublish = undefined;
		const second = new FakeConnection();
		sdk.connect.mockResolvedValueOnce(first).mockResolvedValueOnce(second);
		const sent = pushNats(descriptor(), push, { deliveryId: "stable-id" });
		await vi.advanceTimersByTimeAsync(10_001);
		await sent;
		expect(first.published[0].body.id).toBe("stable-id");
		expect(second.published[0].body.id).toBe("stable-id");
		expect(first.callbacks.size).toBe(0);
	});

	it("aborts an in-flight message without retrying or using HTTP fallback", async () => {
		const client = new FakeConnection();
		client.onPublish = undefined;
		sdk.connect.mockResolvedValue(client);
		const controller = new AbortController();
		const fetch = vi.fn();
		vi.stubGlobal("fetch", fetch);
		const sent = rawPushChannel(
			{
				channel_id: "run-1",
				expires_at: descriptor().expires_at,
				transport: descriptor(),
				fallback: {
					type: "http",
					push_url: "https://api.example/push",
					token: "http",
				},
			},
			push,
			{ signal: controller.signal },
		);
		await vi.waitFor(() => expect(client.published).toHaveLength(1));
		controller.abort();
		await expect(sent).rejects.toThrow("aborted");
		expect(fetch).not.toHaveBeenCalled();
		expect(sdk.connect).toHaveBeenCalledTimes(1);
		expect(client.callbacks.size).toBe(0);
	});

	it("preserves the delivery ID when a broker failure uses the HTTP fallback", async () => {
		sdk.connect.mockRejectedValue(new Error("unreachable"));
		vi.spyOn(console, "warn").mockImplementation(() => {});
		const fetch = vi.fn(
			async (_input: RequestInfo | URL, _init?: RequestInit) =>
				new Response(null, { status: 204 }),
		);
		vi.stubGlobal("fetch", fetch);
		await rawPushChannel(
			{
				channel_id: "run-1",
				expires_at: descriptor().expires_at,
				transport: descriptor(),
				fallback: {
					type: "http",
					push_url: "https://api.example/push",
					token: "http",
				},
			},
			push,
			{ deliveryId: "fallback-id" },
		);
		expect(
			(fetch.mock.calls[0][1]?.headers as Record<string, string>)[
				"x-channel-delivery-id"
			],
		).toBe("fallback-id");
	});

	it("rejects expired grants and mismatched channels before connecting", async () => {
		await expect(
			pushNats({ ...descriptor(), expires_at: 1 }, push),
		).rejects.toThrow("expired");
		await expect(
			pushNats(descriptor(), { ...push, channel_id: "another-run" }),
		).rejects.toThrow("does not belong");
		await expect(
			pushNats(descriptor(), push, { deliveryId: "wildcard.>" }),
		).rejects.toThrow("delivery ID");
		expect(sdk.connect).not.toHaveBeenCalled();
	});

	it("closes idle sockets and opens a new connection for rotated credentials", async () => {
		vi.useFakeTimers();
		const initial = {
			...descriptor(),
			expires_at: Math.floor(Date.now() / 1000) + 180,
		};
		await pushNats(initial, push);
		await pushNats({ ...initial, token: "replacement" }, push);
		expect(sdk.connect).toHaveBeenCalledTimes(2);
		await vi.advanceTimersByTimeAsync(60_001);
		await pushNats({ ...initial, token: "replacement" }, push);
		expect(sdk.connect).toHaveBeenCalledTimes(3);
	});
});
