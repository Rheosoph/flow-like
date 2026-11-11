import { afterEach, describe, expect, it, spyOn } from "bun:test";
import type { IChannelHandle } from "../../schema/channel";
import { registerScopedHttpChannel } from "../http";
import {
	cancelChannel,
	isChannelHandle,
	pushToChannel,
	replyToChannel,
	resetChannelClientCaches,
	steerChannel,
} from "../index";

const PUSH_URL = "https://api.example/api/v1/channels/run-1/push";
const FALLBACK_URL = "https://api.example/api/v1/channels/run-1/push?fallback";

function httpHandle(overrides: Partial<IChannelHandle> = {}): IChannelHandle {
	return {
		channel_id: "run-1",
		request_id: "req-1",
		expires_at: 4_102_444_800,
		transport: { type: "http", push_url: PUSH_URL, token: "push-token" },
		...overrides,
	};
}

type FetchCall = { url: string; init: RequestInit };

function installFetch(
	respond: (call: FetchCall, index: number) => Response | Promise<Response>,
) {
	const calls: FetchCall[] = [];
	const original = globalThis.fetch;
	globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
		const call = { url: String(input), init: init ?? {} };
		calls.push(call);
		return respond(call, calls.length - 1);
	}) as typeof fetch;
	return {
		calls,
		restore: () => {
			globalThis.fetch = original;
		},
	};
}

const deliveryId = (call: FetchCall) =>
	(call.init.headers as Record<string, string>)["x-channel-delivery-id"];

let restore: (() => void) | undefined;
afterEach(() => {
	restore?.();
	restore = undefined;
	resetChannelClientCaches();
});

describe("http channel push", () => {
	it("posts the push envelope with the bearer token", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;

		await replyToChannel(httpHandle(), { approved: true });

		expect(fetchMock.calls).toHaveLength(1);
		const [{ url, init }] = fetchMock.calls;
		expect(url).toBe(PUSH_URL);
		expect(init.method).toBe("POST");
		expect(init.headers).toEqual({
			"content-type": "application/json",
			authorization: "Bearer push-token",
			"x-channel-delivery-id": expect.any(String),
		});
		expect(init.credentials).toBe("omit");
		expect(init.redirect).toBe("error");
		expect(JSON.parse(String(init.body))).toEqual({
			channel_id: "run-1",
			request_id: "req-1",
			kind: "reply",
			value: { approved: true },
		});
	});

	it("surfaces status and body excerpt on a non-2xx response", async () => {
		const fetchMock = installFetch(
			() => new Response("receiver gone", { status: 410 }),
		);
		restore = fetchMock.restore;

		await expect(replyToChannel(httpHandle(), 1)).rejects.toThrow(
			/\(410\): receiver gone/,
		);
	});

	it("cancel and steer address the channel without a request id", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 200 }));
		restore = fetchMock.restore;
		const handle = httpHandle({ request_id: null });

		await cancelChannel(handle);
		await steerChannel(handle, "focus on tests");

		const bodies = fetchMock.calls.map((call) =>
			JSON.parse(String(call.init.body)),
		);
		expect(bodies).toEqual([
			{ channel_id: "run-1", kind: "cancel", value: null },
			{ channel_id: "run-1", kind: "inbound", value: "focus on tests" },
		]);
		expect(bodies[0]).not.toHaveProperty("request_id");
	});

	it("refuses to reply on a channel-level handle", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 200 }));
		restore = fetchMock.restore;

		await expect(
			replyToChannel(httpHandle({ request_id: null }), 1),
		).rejects.toThrow(/no request_id/);
		expect(fetchMock.calls).toHaveLength(0);
	});
});

describe("fallback", () => {
	it("retries once through the fallback after a transport failure", async () => {
		const fetchMock = installFetch((call) => {
			if (call.url === PUSH_URL) throw new TypeError("network down");
			return new Response(null, { status: 204 });
		});
		restore = fetchMock.restore;
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);

		await pushToChannel(
			httpHandle({
				fallback: { type: "http", push_url: FALLBACK_URL, token: "fb-token" },
			}),
			{ request_id: "req-1", kind: "reply", value: "ok" },
		);

		expect(fetchMock.calls.map((call) => call.url)).toEqual([
			PUSH_URL,
			FALLBACK_URL,
		]);
		expect(
			(fetchMock.calls[1].init.headers as Record<string, string>).authorization,
		).toBe("Bearer fb-token");
		expect(deliveryId(fetchMock.calls[1])).toBe(deliveryId(fetchMock.calls[0]));
		expect(warn).toHaveBeenCalledTimes(1);
		expect(String(warn.mock.calls[0][0])).toContain("network down");
		warn.mockRestore();
	});

	it("reports both failures when the fallback fails too", async () => {
		const fetchMock = installFetch(
			(call) =>
				new Response(call.url === PUSH_URL ? "primary boom" : "fallback boom", {
					status: 500,
				}),
		);
		restore = fetchMock.restore;
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);

		await expect(
			pushToChannel(
				httpHandle({
					fallback: { type: "http", push_url: FALLBACK_URL, token: "t" },
				}),
				{ request_id: "req-1", value: 1 },
			),
		).rejects.toThrow(/fallback boom.*primary boom/);
		warn.mockRestore();
	});

	it("rethrows the primary failure when no fallback exists", async () => {
		const fetchMock = installFetch(() => new Response("nope", { status: 503 }));
		restore = fetchMock.restore;

		await expect(replyToChannel(httpHandle(), 1)).rejects.toThrow(
			/\(503\): nope/,
		);
		expect(fetchMock.calls).toHaveLength(1);
	});

	it("does not deliver an aborted push through either transport", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;
		const controller = new AbortController();
		controller.abort();

		await expect(
			pushToChannel(
				httpHandle({
					transport: awsTransport(),
					fallback: { type: "http", push_url: FALLBACK_URL, token: "t" },
				}),
				{ request_id: "req-1", value: 1 },
				{ signal: controller.signal },
			),
		).rejects.toThrow(/abort/i);
		expect(fetchMock.calls).toHaveLength(0);
	});

	it("does not retry a push that is aborted while the provider is sending", async () => {
		const controller = new AbortController();
		const fetchMock = installFetch(() => {
			controller.abort();
			throw new TypeError("aborted by caller");
		});
		restore = fetchMock.restore;

		await expect(
			pushToChannel(
				httpHandle({
					transport: awsTransport(),
					fallback: { type: "http", push_url: FALLBACK_URL, token: "t" },
				}),
				{ request_id: "req-1", value: 1 },
				{ signal: controller.signal },
			),
		).rejects.toThrow(/aborted/i);
		expect(fetchMock.calls.map(({ url }) => url)).toEqual([
			expect.stringContaining("/connections/run-1/messages"),
		]);
	});

	it("never falls back from a scoped HTTP channel", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;
		const scoped = registerScopedHttpChannel(async () => {
			throw new Error("service session refused");
		});

		try {
			await expect(
				replyToChannel(
					httpHandle({
						transport: { type: "http", push_url: scoped.url, token: "s" },
						fallback: { type: "http", push_url: FALLBACK_URL, token: "t" },
					}),
					1,
				),
			).rejects.toThrow("service session refused");
		} finally {
			scoped.close();
		}
		expect(fetchMock.calls).toHaveLength(0);
	});
});

function awsTransport(expiration = 4_102_444_800): IChannelHandle["transport"] {
	return {
		type: "aws_mqtt",
		endpoint: "data.iot.example",
		region: "eu-central-1",
		target_client_id: "run-1",
		topic: "runs/run-1/inbox",
		credentials: {
			access_key_id: "AKID",
			secret_access_key: "secret",
			session_token: "token",
			expiration,
		},
	};
}

class FailingWebSocket {
	private listeners = new Map<string, Array<(event: unknown) => void>>();
	readyState = 0;
	constructor() {
		queueMicrotask(() => {
			for (const listener of this.listeners.get("error") ?? []) listener({});
		});
	}
	addEventListener(type: string, listener: (event: unknown) => void) {
		this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
	}
	removeEventListener() {}
	close() {}
}

describe("provider handles", () => {
	const unreachable: IChannelHandle["transport"][] = [
		awsTransport(1),
		{
			type: "azure_web_pubsub",
			url: "wss://azure.example",
			group: "run:run-1",
			expires_at: 4_102_444_800,
		},
		{
			type: "gcp_firebase_rtdb",
			database_url: "https://firebase.example",
			api_key: "key",
			project_id: "proj",
			custom_token: "token",
			inbox_path: "/channels/run-1/inbox",
			inbound_path: "/channels/run-1/inbound",
			expires_at: 1,
		},
		{
			type: "nats",
			url: "wss://nats.example",
			subject: "channel.run-1.inbox",
			token: "token",
			expires_at: 1,
		},
	];

	function stubUnreachableProviders() {
		const original = globalThis.WebSocket;
		globalThis.WebSocket = FailingWebSocket as unknown as typeof WebSocket;
		const previous = restore;
		restore = () => {
			globalThis.WebSocket = original;
			previous?.();
		};
	}

	function apiFallback() {
		return {
			type: "http" as const,
			push_url: FALLBACK_URL,
			token: "scoped-capability",
		};
	}

	it("sends an aws_mqtt reply on its own transport without touching the API", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;

		await replyToChannel(
			httpHandle({ transport: awsTransport(), fallback: apiFallback() }),
			{ page: "ready" },
		);

		expect(fetchMock.calls).toHaveLength(1);
		expect(fetchMock.calls[0].url).toContain(
			"https://data.iot.example/connections/run-1/messages",
		);
		expect(JSON.parse(String(fetchMock.calls[0].init.body))).toEqual({
			channel_id: "run-1",
			request_id: "req-1",
			kind: "reply",
			value: { page: "ready" },
		});
	});

	it("uses the API only after the provider failed and keeps one delivery id", async () => {
		const fetchMock = installFetch((call) =>
			call.url === FALLBACK_URL
				? new Response(null, { status: 204 })
				: new Response("denied", { status: 403 }),
		);
		restore = fetchMock.restore;
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);

		await replyToChannel(
			httpHandle({ transport: awsTransport(), fallback: apiFallback() }),
			{ page: "ready" },
			{ deliveryId: "reply-delivery" },
		);

		expect(fetchMock.calls).toHaveLength(2);
		expect(fetchMock.calls[0].url).toContain("/connections/run-1/messages");
		expect(fetchMock.calls[1].url).toBe(FALLBACK_URL);
		expect(deliveryId(fetchMock.calls[1])).toBe("reply-delivery");
		expect(String(warn.mock.calls[0][0])).toContain("aws_mqtt");
		warn.mockRestore();
	});

	it("generates one delivery id for the fallback when the caller gives none", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;
		stubUnreachableProviders();
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);

		await replyToChannel(
			httpHandle({ transport: awsTransport(1), fallback: apiFallback() }),
			1,
		);

		expect(deliveryId(fetchMock.calls[0])).toMatch(/^[0-9a-f-]{36}$/);
		warn.mockRestore();
	});

	for (const transport of unreachable) {
		it(`falls back to the API when the ${transport.type} transport cannot be reached`, async () => {
			const fetchMock = installFetch(() => Response.json({ accepted: true }));
			restore = fetchMock.restore;
			stubUnreachableProviders();
			const warn = spyOn(console, "warn").mockImplementation(() => undefined);
			const handle = httpHandle({ transport, fallback: apiFallback() });

			await replyToChannel(
				handle,
				{ page: "ready" },
				{ deliveryId: "reply-delivery" },
			);
			await steerChannel(handle, "continue", { deliveryId: "steer-delivery" });

			expect(fetchMock.calls.map(({ url }) => url)).toEqual([
				FALLBACK_URL,
				FALLBACK_URL,
			]);
			expect(fetchMock.calls.map(deliveryId)).toEqual([
				"reply-delivery",
				"steer-delivery",
			]);
			const first = fetchMock.calls[0].init;
			expect((first.headers as Record<string, string>).authorization).toBe(
				"Bearer scoped-capability",
			);
			expect(JSON.parse(String(first.body))).toEqual({
				channel_id: "run-1",
				request_id: "req-1",
				kind: "reply",
				value: { page: "ready" },
			});
			expect(warn).toHaveBeenCalledTimes(2);
			warn.mockRestore();
		});

		it(`reports the ${transport.type} failure when the handle has no API fallback`, async () => {
			const fetchMock = installFetch(() => Response.json({ accepted: true }));
			restore = fetchMock.restore;
			stubUnreachableProviders();

			await expect(
				replyToChannel(httpHandle({ transport, fallback: null }), 1),
			).rejects.toThrow();
			expect(fetchMock.calls).toHaveLength(0);
		});

		it(`names both failures when ${transport.type} and the API fail`, async () => {
			const fetchMock = installFetch(
				() => new Response("unavailable", { status: 503 }),
			);
			restore = fetchMock.restore;
			stubUnreachableProviders();
			const warn = spyOn(console, "warn").mockImplementation(() => undefined);

			await expect(
				replyToChannel(httpHandle({ transport, fallback: apiFallback() }), 1),
			).rejects.toThrow(new RegExp(`503.*after ${transport.type} failed`, "s"));
			expect(fetchMock.calls.map(({ url }) => url)).toEqual([FALLBACK_URL]);
			warn.mockRestore();
		});
	}

	it("rejects an unknown transport type", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;

		await expect(
			replyToChannel(
				httpHandle({
					transport: { type: "carrier_pigeon" } as never,
				}),
				1,
			),
		).rejects.toThrow(/Unknown channel transport 'carrier_pigeon'/);
		expect(fetchMock.calls).toHaveLength(0);
	});
});

describe("cancel on a provider handle", () => {
	const apiFallback = () => ({
		type: "http" as const,
		push_url: FALLBACK_URL,
		token: "scoped-capability",
	});

	it("goes through the API, which also stops the run, and skips the provider", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;

		await cancelChannel(
			httpHandle({
				request_id: null,
				transport: awsTransport(),
				fallback: apiFallback(),
			}),
		);

		expect(fetchMock.calls.map(({ url }) => url)).toEqual([FALLBACK_URL]);
		expect(JSON.parse(String(fetchMock.calls[0].init.body))).toEqual({
			channel_id: "run-1",
			kind: "cancel",
			value: null,
		});
	});

	it("retries on the provider when the API fails and keeps the delivery id", async () => {
		const fetchMock = installFetch((call) =>
			call.url === FALLBACK_URL
				? new Response("unavailable", { status: 503 })
				: new Response(null, { status: 204 }),
		);
		restore = fetchMock.restore;
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);

		await cancelChannel(
			httpHandle({
				request_id: null,
				transport: awsTransport(),
				fallback: apiFallback(),
			}),
			{ deliveryId: "cancel-delivery" },
		);

		expect(fetchMock.calls).toHaveLength(2);
		expect(fetchMock.calls[0].url).toBe(FALLBACK_URL);
		expect(deliveryId(fetchMock.calls[0])).toBe("cancel-delivery");
		expect(fetchMock.calls[1].url).toContain("/connections/run-1/messages");
		expect(JSON.parse(String(fetchMock.calls[1].init.body))).toMatchObject({
			kind: "cancel",
		});
		expect(String(warn.mock.calls[0][0])).toContain("503");
		warn.mockRestore();
	});

	it("names both failures when the API and the provider fail", async () => {
		const fetchMock = installFetch(
			(call) =>
				new Response(call.url === FALLBACK_URL ? "api boom" : "iot boom", {
					status: 500,
				}),
		);
		restore = fetchMock.restore;
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);

		await expect(
			cancelChannel(
				httpHandle({
					request_id: null,
					transport: awsTransport(),
					fallback: apiFallback(),
				}),
			),
		).rejects.toThrow(/iot boom.*after http failed: .*api boom/s);
		warn.mockRestore();
	});

	it("is not retried on the provider after an abort", async () => {
		const controller = new AbortController();
		const fetchMock = installFetch(() => {
			controller.abort();
			throw new TypeError("aborted by caller");
		});
		restore = fetchMock.restore;

		await expect(
			cancelChannel(
				httpHandle({
					request_id: null,
					transport: awsTransport(),
					fallback: apiFallback(),
				}),
				{ signal: controller.signal },
			),
		).rejects.toThrow(/aborted/i);
		expect(fetchMock.calls.map(({ url }) => url)).toEqual([FALLBACK_URL]);
	});

	it("uses the provider alone when the handle has no API fallback", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;

		await cancelChannel(
			httpHandle({ request_id: null, transport: awsTransport() }),
		);

		expect(fetchMock.calls).toHaveLength(1);
		expect(fetchMock.calls[0].url).toContain("/connections/run-1/messages");
	});

	it("still sends a reply on the same handle to the provider first", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;

		await replyToChannel(
			httpHandle({ transport: awsTransport(), fallback: apiFallback() }),
			1,
		);

		expect(fetchMock.calls).toHaveLength(1);
		expect(fetchMock.calls[0].url).toContain("/connections/run-1/messages");
	});
});

describe("isChannelHandle", () => {
	it("accepts the wire shape and rejects fragments", () => {
		expect(isChannelHandle(httpHandle())).toBe(true);
		expect(isChannelHandle({ channel_id: "x" })).toBe(false);
		expect(isChannelHandle(null)).toBe(false);
		expect(
			isChannelHandle({ channel_id: "x", expires_at: 1, transport: {} }),
		).toBe(false);
	});
});
