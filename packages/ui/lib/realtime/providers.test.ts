import { createHash, createHmac } from "node:crypto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
	awsRealtimeWebsocketUrl,
	openRealtimeProvider,
	parseRealtimeReply,
	parseRealtimeWakeup,
} from "./providers";
import type { RealtimeTransport } from "./types";

const nats = vi.hoisted(() => ({
	wsconnect: vi.fn(),
	jwtAuthenticator: vi.fn((token) => token),
}));
vi.mock("@nats-io/nats-core", () => nats);

class Socket extends EventTarget {
	static sockets: Socket[] = [];
	binaryType = "";
	send = vi.fn();
	close = vi.fn(() => this.dispatchEvent(new Event("close")));
	constructor(
		readonly url: string,
		readonly protocol: string,
	) {
		super();
		Socket.sockets.push(this);
	}
	message(data: unknown) {
		this.dispatchEvent(new MessageEvent("message", { data }));
	}
}
const controllers: AbortController[] = [];
const signal = () => {
	const controller = new AbortController();
	controllers.push(controller);
	return controller.signal;
};
const callbacks = () => ({ wakeup: vi.fn(), disconnected: vi.fn() });
const wakeup = {
	protocol_version: 1,
	kind: "invalidate",
	nonce: "n1",
} as const;

const reply = {
	protocol_version: 1,
	kind: "reply",
	conversation_id: "c1",
	message_id: "re-1",
	reply_to: "m1",
	sender_id: "bot",
	offset: 0,
	text: "Hello",
	state: "writing",
} as const;

beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(new Date("2026-11-01T00:00:00Z"));
	vi.stubGlobal("WebSocket", Socket);
});
afterEach(() => {
	for (const controller of controllers.splice(0)) controller.abort();
	Socket.sockets = [];
	vi.unstubAllGlobals();
	vi.useRealTimers();
	vi.clearAllMocks();
});

const awsGrant: Extract<RealtimeTransport, { type: "aws_mqtt" }> = {
	type: "aws_mqtt",
	endpoint: "example.iot.eu-central-1.amazonaws.com",
	region: "eu-central-1",
	client_id: "realtime-client",
	topic: "realtime/account/wake",
	credentials: {
		access_key_id: "test-access",
		secret_access_key: "test-secret",
		session_token: "test/session+token",
		expiration: 1_793_491_800,
	},
};

describe("receive-only provider adapters", () => {
	test("accepts invalidation only, never execution or action payloads", () => {
		expect(parseRealtimeWakeup(JSON.stringify(wakeup))).toEqual(wakeup);
		expect(parseRealtimeWakeup({ ...wakeup, kind: "execute" })).toBeUndefined();
		expect(
			parseRealtimeWakeup({ ...wakeup, protocol_version: 2 }),
		).toBeUndefined();
		expect(parseRealtimeWakeup("x".repeat(65_537))).toBeUndefined();
		expect(parseRealtimeWakeup({ ...wakeup, payload: {} })).toBeUndefined();
		expect(
			parseRealtimeWakeup({ ...wakeup, nonce: "あ".repeat(43) }),
		).toBeUndefined();
		expect(
			parseRealtimeWakeup({ ...wakeup, nonce: "control\u0085" }),
		).toBeUndefined();
		expect(
			parseRealtimeWakeup(`${JSON.stringify(wakeup)}${" ".repeat(4096)}`),
		).toBeUndefined();
	});

	test("parses a reply from a string or an object and keeps only known fields", () => {
		expect(parseRealtimeReply(JSON.stringify(reply))).toEqual(reply);
		expect(parseRealtimeReply({ ...reply, extra: { a: 1 } })).toEqual(reply);
		expect(
			Object.keys(parseRealtimeReply({ ...reply, extra: 1 }) ?? {}),
		).toEqual(Object.keys(reply));
		expect(parseRealtimeReply({ ...reply, text: "" })).toEqual({
			...reply,
			text: "",
		});
	});

	test("keeps the draft of a rewritten reply and leaves out the first", () => {
		expect(parseRealtimeReply({ ...reply, draft: 2 })).toEqual({
			...reply,
			draft: 2,
		});
		expect(parseRealtimeReply({ ...reply, draft: 0 })).toEqual(reply);
	});

	test.each([
		["version", { protocol_version: 2 }],
		["kind", { kind: "invalidate" }],
		["empty conversation id", { conversation_id: "" }],
		["empty message id", { message_id: "" }],
		["long reply_to", { reply_to: "a".repeat(257) }],
		["control character", { sender_id: "bot\u0007" }],
		["negative offset", { offset: -1 }],
		["fractional offset", { offset: 1.5 }],
		["non-string text", { text: 5 }],
		["unknown state", { state: "typing" }],
		["long text", { text: "x".repeat(4097) }],
		["negative draft", { draft: -1 }],
		["fractional draft", { draft: 1.5 }],
		["non-number draft", { draft: "1" }],
	])("rejects a reply with an invalid %s", (_name, patch) => {
		expect(parseRealtimeReply({ ...reply, ...patch })).toBeUndefined();
	});

	test("rejects a reply string over 4096 bytes or that is not JSON", () => {
		expect(
			parseRealtimeReply(JSON.stringify({ ...reply, text: "あ".repeat(1400) })),
		).toBeUndefined();
		expect(parseRealtimeReply("{nope")).toBeUndefined();
		expect(parseRealtimeReply(null)).toBeUndefined();
		expect(parseRealtimeReply([reply])).toBeUndefined();
	});

	test("a wake-up parser rejects a reply", () => {
		expect(parseRealtimeWakeup(reply)).toBeUndefined();
	});

	test("AWS delivers replies and wake-ups, and ignores replies without a callback", async () => {
		const open = async (events: ReturnType<typeof callbacks> | object) => {
			const known = Socket.sockets.length;
			const pending = openRealtimeProvider(
				awsGrant,
				events as Parameters<typeof openRealtimeProvider>[1],
				signal(),
			);
			await vi.waitFor(() =>
				expect(Socket.sockets.length).toBeGreaterThan(known),
			);
			const socket = Socket.sockets.at(-1) as Socket;
			socket.dispatchEvent(new Event("open"));
			socket.message(Uint8Array.of(0x20, 2, 0, 0).buffer);
			socket.message(Uint8Array.of(0x90, 3, 0, 1, 0).buffer);
			await pending;
			const topic = new TextEncoder().encode(awsGrant.topic);
			return (payload: unknown) => {
				const data = new TextEncoder().encode(JSON.stringify(payload));
				const remaining = topic.length + data.length + 2;
				socket.message(
					Uint8Array.from([
						0x30,
						(remaining & 0x7f) | 0x80,
						remaining >> 7,
						0,
						topic.length,
						...topic,
						...data,
					]).buffer,
				);
			};
		};
		const events = { ...callbacks(), reply: vi.fn() };
		const publish = await open(events);
		publish(reply);
		publish(wakeup);
		expect(events.reply).toHaveBeenCalledExactlyOnceWith(reply);
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		const bare = callbacks();
		const publishBare = await open(bare);
		expect(() => publishBare(reply)).not.toThrow();
		expect(bare.wakeup).not.toHaveBeenCalled();
	});

	test("Azure pings idle connections and recovers when server liveness disappears", async () => {
		const events = callbacks();
		const pending = openRealtimeProvider(
			{
				type: "azure_web_pubsub",
				url: "wss://hub.test/client?access_token=read",
				group: "account",
			},
			events,
			signal(),
		);
		const socket = Socket.sockets[0];
		socket.dispatchEvent(new Event("open"));
		socket.message(JSON.stringify({ type: "system", event: "connected" }));
		await pending;
		await vi.advanceTimersByTimeAsync(20_000);
		expect(socket.send).toHaveBeenLastCalledWith(
			JSON.stringify({ type: "ping" }),
		);
		socket.message(JSON.stringify({ type: "pong" }));
		await vi.advanceTimersByTimeAsync(40_000);
		expect(events.disconnected).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(20_000);
		expect(events.disconnected).toHaveBeenCalledTimes(1);
	});

	test("rejects a WebSocket with an unexpected negotiated protocol", async () => {
		const pending = openRealtimeProvider(
			{
				type: "azure_web_pubsub",
				url: "wss://hub.test/client",
				group: "account",
			},
			callbacks(),
			signal(),
		);
		const socket = Socket.sockets[0];
		Object.defineProperty(socket, "protocol", { value: "" });
		socket.dispatchEvent(new Event("open"));
		await expect(pending).rejects.toMatchObject({
			stage: "connect",
			message:
				'the provider chose protocol "" instead of "json.webpubsub.azure.v1"',
		});
	});

	test("AWS names the step a connection stopped at and why a live one dropped", async () => {
		const opened = async (count: number) => {
			await vi.waitFor(() => expect(Socket.sockets).toHaveLength(count));
			const socket = Socket.sockets[count - 1];
			socket.dispatchEvent(new Event("open"));
			return socket;
		};

		const silent = openRealtimeProvider(awsGrant, callbacks(), signal());
		const silentResult = expect(silent).rejects.toMatchObject({
			stage: "handshake",
			message: "no answer within 10 s",
		});
		await vi.waitFor(() => expect(Socket.sockets).toHaveLength(1));
		await vi.advanceTimersByTimeAsync(10_000);
		await silentResult;

		const refused = openRealtimeProvider(awsGrant, callbacks(), signal());
		(await opened(2)).message(Uint8Array.of(0x20, 2, 0, 5).buffer);
		await expect(refused).rejects.toMatchObject({
			stage: "connect",
			message: "MQTT connection denied (CONNACK return code 5)",
		});

		const denied = openRealtimeProvider(awsGrant, callbacks(), signal());
		const deniedSocket = await opened(3);
		deniedSocket.message(Uint8Array.of(0x20, 2, 0, 0).buffer);
		deniedSocket.message(Uint8Array.of(0x90, 3, 0, 1, 0x80).buffer);
		await expect(denied).rejects.toMatchObject({
			stage: "subscribe",
			message: `MQTT subscription to ${awsGrant.topic} denied (SUBACK code 128)`,
		});

		const events = callbacks();
		const live = openRealtimeProvider(awsGrant, events, signal());
		const liveSocket = await opened(4);
		liveSocket.message(Uint8Array.of(0x20, 2, 0, 0).buffer);
		liveSocket.message(Uint8Array.of(0x90, 3, 0, 1, 0).buffer);
		await live;
		liveSocket.dispatchEvent(
			Object.assign(new Event("close"), { code: 1006, reason: "" }),
		);
		expect(events.disconnected).toHaveBeenCalledExactlyOnceWith(
			"closed with code 1006",
		);
	});

	test("Azure uses a prejoined group and sends no group mutation messages", async () => {
		const events = callbacks();
		const pending = openRealtimeProvider(
			{
				type: "azure_web_pubsub",
				url: "wss://hub.test/client?access_token=read",
				group: "account",
			},
			events,
			signal(),
		);
		const socket = Socket.sockets[0];
		socket.dispatchEvent(new Event("open"));
		socket.message(
			JSON.stringify({ type: "system", event: "connected", connectionId: "c" }),
		);
		const connection = await pending;
		expect(socket.send).not.toHaveBeenCalled();
		socket.message(
			JSON.stringify({
				type: "message",
				from: "group",
				group: "other",
				data: wakeup,
			}),
		);
		socket.message(
			JSON.stringify({
				type: "message",
				from: "group",
				group: "account",
				data: wakeup,
			}),
		);
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		socket.message(
			JSON.stringify({
				type: "message",
				from: "server",
				dataType: "json",
				data: { ...wakeup, nonce: "server" },
			}),
		);
		expect(events.wakeup).toHaveBeenLastCalledWith({
			...wakeup,
			nonce: "server",
		});
		expect(events.wakeup).toHaveBeenCalledTimes(2);
		connection.close();
		expect(events.disconnected).not.toHaveBeenCalled();
	});

	test("AWS signs MQTT connection URLs with a scoped token outside the canonical signature", async () => {
		const url = new URL(await awsRealtimeWebsocketUrl(awsGrant));
		expect(url.protocol).toBe("wss:");
		expect(url.pathname).toBe("/mqtt");
		expect(url.searchParams.get("X-Amz-Security-Token")).toBe(
			awsGrant.credentials.session_token,
		);
		const signature = url.searchParams.get("X-Amz-Signature");
		url.searchParams.delete("X-Amz-Signature");
		url.searchParams.delete("X-Amz-Security-Token");
		const query = [...url.searchParams]
			.sort(([a], [b]) => a.localeCompare(b))
			.map(
				([key, value]) =>
					`${encodeURIComponent(key)}=${encodeURIComponent(value)}`,
			)
			.join("&");
		const hash = (value: string) =>
			createHash("sha256").update(value).digest("hex");
		const canonical = `GET\n/mqtt\n${query}\nhost:${url.host}\n\nhost\n${hash("")}`;
		const scope = "20261101/eu-central-1/iotdevicegateway/aws4_request";
		let key: string | Buffer = `AWS4${awsGrant.credentials.secret_access_key}`;
		for (const value of [
			"20261101",
			"eu-central-1",
			"iotdevicegateway",
			"aws4_request",
		])
			key = createHmac("sha256", key).update(value).digest();
		expect(signature).toBe(
			createHmac("sha256", key)
				.update(
					`AWS4-HMAC-SHA256\n20261101T000000Z\n${scope}\n${hash(canonical)}`,
				)
				.digest("hex"),
		);
	});

	test("AWS subscribes only to its exact topic and handles fragmented MQTT invalidations", async () => {
		const events = callbacks();
		const pending = openRealtimeProvider(awsGrant, events, signal());
		await vi.waitFor(() => expect(Socket.sockets).toHaveLength(1));
		const socket = Socket.sockets[0];
		socket.dispatchEvent(new Event("open"));
		expect((socket.send.mock.calls[0][0] as Uint8Array)[0]).toBe(0x10);
		socket.message(Uint8Array.of(0x20, 2, 0, 0).buffer);
		expect((socket.send.mock.calls[1][0] as Uint8Array)[0]).toBe(0x82);
		socket.message(Uint8Array.of(0x90, 3, 0, 1, 0).buffer);
		await pending;
		const topic = new TextEncoder().encode(awsGrant.topic);
		const data = new TextEncoder().encode(JSON.stringify(wakeup));
		const packet = Uint8Array.from([
			0x30,
			topic.length + data.length + 2,
			0,
			topic.length,
			...topic,
			...data,
		]);
		socket.message(packet.slice(0, 3).buffer);
		socket.message(packet.slice(3).buffer);
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		expect(socket.send).toHaveBeenCalledTimes(2);
		await vi.advanceTimersByTimeAsync(20_000);
		expect(socket.send).toHaveBeenLastCalledWith(Uint8Array.of(0xc0, 0));
		await vi.advanceTimersByTimeAsync(20_000);
		expect(events.disconnected).toHaveBeenCalledTimes(1);
	});

	test("Firebase exchanges its grant then only reads the granted path", async () => {
		let stream!: ReadableStreamDefaultController<Uint8Array>;
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce(Response.json({ idToken: "read-token" }))
			.mockResolvedValueOnce(
				new Response(
					new ReadableStream({
						start(controller) {
							stream = controller;
						},
					}),
				),
			);
		vi.stubGlobal("fetch", fetcher);
		const events = callbacks();
		const connection = await openRealtimeProvider(
			{
				type: "gcp_firebase_rtdb",
				database_url: "https://project.firebaseio.com",
				api_key: "api",
				project_id: "project",
				custom_token: "read-grant",
				path: "/realtime/account/wake",
			},
			events,
			signal(),
		);
		expect(fetcher.mock.calls[0][1].method).toBe("POST");
		expect(String(fetcher.mock.calls[1][0])).toBe(
			"https://project.firebaseio.com/realtime/account/wake.json?auth=read-token",
		);
		expect(fetcher.mock.calls[1][1].method).toBeUndefined();
		stream.enqueue(
			new TextEncoder().encode(
				`event: put\ndata: ${JSON.stringify({ path: "/", data: wakeup })}\n\n`,
			),
		);
		await vi.advanceTimersByTimeAsync(0);
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		stream.enqueue(
			new TextEncoder().encode('event: auth_revoked\ndata: "expired"\n\n'),
		);
		await vi.advanceTimersByTimeAsync(0);
		expect(events.disconnected).toHaveBeenCalledTimes(1);
		connection.close();
	});

	const firebaseGrant: Extract<
		RealtimeTransport,
		{ type: "gcp_firebase_rtdb" }
	> = {
		type: "gcp_firebase_rtdb",
		database_url: "https://project.firebaseio.com",
		api_key: "api",
		project_id: "project",
		custom_token: "read-grant",
		path: "/realtime/account/wake",
		reply_path: "/realtime/account/reply",
	};

	const eventStream = () => {
		let controller!: ReadableStreamDefaultController<Uint8Array>;
		const response = new Response(
			new ReadableStream<Uint8Array>({
				start(started) {
					controller = started;
				},
			}),
		);
		const put = (data: unknown) =>
			controller.enqueue(
				new TextEncoder().encode(
					`event: put\ndata: ${JSON.stringify({ path: "/", data })}\n\n`,
				),
			);
		return { response, put, end: () => controller.close() };
	};

	test("Firebase reads replies from their own node, apart from the wake-ups", async () => {
		const wake = eventStream();
		const replies = eventStream();
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce(Response.json({ idToken: "read-token" }))
			.mockResolvedValueOnce(wake.response)
			.mockResolvedValueOnce(replies.response);
		vi.stubGlobal("fetch", fetcher);
		const events = { ...callbacks(), reply: vi.fn() };
		const connection = await openRealtimeProvider(
			firebaseGrant,
			events,
			signal(),
		);
		expect(fetcher.mock.calls.slice(1).map(([url]) => String(url))).toEqual([
			"https://project.firebaseio.com/realtime/account/wake.json?auth=read-token",
			"https://project.firebaseio.com/realtime/account/reply.json?auth=read-token",
		]);
		replies.put(null);
		replies.put(wakeup);
		replies.put({ ...reply, draft: 1 });
		await vi.advanceTimersByTimeAsync(0);
		expect(events.wakeup).not.toHaveBeenCalled();
		expect(events.reply).toHaveBeenCalledExactlyOnceWith(
			parseRealtimeReply({ ...reply, draft: 1 }),
		);
		replies.end();
		await vi.advanceTimersByTimeAsync(0);
		wake.put(wakeup);
		await vi.advanceTimersByTimeAsync(0);
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		expect(events.disconnected).not.toHaveBeenCalled();
		connection.close();
	});

	test("Firebase keeps its wake-ups when rules that predate the reply node refuse it", async () => {
		const wake = eventStream();
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce(Response.json({ idToken: "read-token" }))
			.mockResolvedValueOnce(wake.response)
			.mockResolvedValueOnce(
				new Response("Permission denied", { status: 401 }),
			);
		vi.stubGlobal("fetch", fetcher);
		const events = { ...callbacks(), reply: vi.fn() };
		const connection = await openRealtimeProvider(
			firebaseGrant,
			events,
			signal(),
		);
		await vi.advanceTimersByTimeAsync(0);
		expect(fetcher).toHaveBeenCalledTimes(3);
		wake.put(wakeup);
		await vi.advanceTimersByTimeAsync(0);
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		expect(events.disconnected).not.toHaveBeenCalled();
		connection.close();
	});

	test.each([
		["a server that names no reply node", { reply_path: undefined }, true],
		["a reply node that leaves its database", { reply_path: "/a/../b" }, true],
		["a caller that takes no replies", {}, false],
	])(
		"Firebase opens only the wake-up node for %s",
		async (_name, patch, withReplies) => {
			const fetcher = vi
				.fn()
				.mockResolvedValueOnce(Response.json({ idToken: "read-token" }))
				.mockResolvedValueOnce(eventStream().response);
			vi.stubGlobal("fetch", fetcher);
			const events = withReplies
				? { ...callbacks(), reply: vi.fn() }
				: callbacks();
			const connection = await openRealtimeProvider(
				{ ...firebaseGrant, ...patch },
				events,
				signal(),
			);
			expect(fetcher).toHaveBeenCalledTimes(2);
			connection.close();
		},
	);

	test("AWS signs with the server clock when the local clock is skewed", async () => {
		const events = callbacks();
		const pending = openRealtimeProvider(awsGrant, events, signal(), {
			clockOffsetMs: -3_600_000,
		});
		await vi.waitFor(() => expect(Socket.sockets).toHaveLength(1));
		expect(new URL(Socket.sockets[0].url).searchParams.get("X-Amz-Date")).toBe(
			"20261031T230000Z",
		);
		Socket.sockets[0].dispatchEvent(new Event("close"));
		await expect(pending).rejects.toThrow();
	});

	test("NATS gives up on a connection that never completes and closes it when it does", async () => {
		const client = {
			close: vi.fn(async () => {}),
		};
		let connected!: (value: typeof client) => void;
		nats.wsconnect.mockReturnValueOnce(
			new Promise((resolve) => {
				connected = resolve;
			}),
		);
		const pending = openRealtimeProvider(
			{
				type: "nats",
				url: "wss://nats.test",
				subject: "realtime.account.wake",
				token: "read-grant",
			},
			callbacks(),
			signal(),
		);
		const rejected = expect(pending).rejects.toThrow("timed out");
		await vi.advanceTimersByTimeAsync(10_000);
		await rejected;
		connected(client);
		await vi.advanceTimersByTimeAsync(0);
		expect(client.close).toHaveBeenCalledTimes(1);
	});

	test("NATS closes a connection whose subscription flush never completes", async () => {
		const unsubscribe = vi.fn();
		const client = {
			subscribe: vi.fn(() => ({ unsubscribe })),
			flush: vi.fn(() => new Promise<void>(() => {})),
			closed: vi.fn(() => new Promise(() => {})),
			close: vi.fn(async () => {}),
		};
		nats.wsconnect.mockResolvedValueOnce(client);
		const events = callbacks();
		const pending = openRealtimeProvider(
			{
				type: "nats",
				url: "wss://nats.test",
				subject: "realtime.account.wake",
				token: "read-grant",
			},
			events,
			signal(),
		);
		const rejected = expect(pending).rejects.toThrow("timed out");
		await vi.advanceTimersByTimeAsync(10_000);
		await rejected;
		expect(unsubscribe).toHaveBeenCalledTimes(1);
		expect(client.close).toHaveBeenCalledTimes(1);
		expect(events.disconnected).not.toHaveBeenCalled();
	});

	test("NATS subscribes without publishing and closes on account cancellation", async () => {
		const client = {
			subscribe: vi.fn(
				(
					_subject: string,
					_options: {
						callback(error: unknown, message: { data: Uint8Array }): void;
					},
				) => ({ unsubscribe: vi.fn() }),
			),
			flush: vi.fn(async () => {}),
			closed: vi.fn(() => new Promise(() => {})),
			close: vi.fn(async () => {}),
			publish: vi.fn(),
		};
		nats.wsconnect.mockResolvedValueOnce(client);
		const events = callbacks();
		const connection = await openRealtimeProvider(
			{
				type: "nats",
				url: "wss://nats.test",
				subject: "realtime.account.wake",
				token: "read-grant",
			},
			events,
			signal(),
		);
		expect(client.subscribe.mock.calls[0][0]).toBe("realtime.account.wake");
		client.subscribe.mock.calls[0][1].callback(null, {
			data: new TextEncoder().encode(JSON.stringify(wakeup)),
		});
		expect(events.wakeup).toHaveBeenCalledExactlyOnceWith(wakeup);
		expect(client.publish).not.toHaveBeenCalled();
		connection.close();
		expect(client.close).toHaveBeenCalledTimes(1);
	});
});
