import { afterEach, describe, expect, it, spyOn } from "bun:test";
import type { IChannelHandle } from "../../schema/channel";
import {
	cancelChannel,
	isChannelHandle,
	pushToChannel,
	replyToChannel,
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

let restore: (() => void) | undefined;
afterEach(() => {
	restore?.();
	restore = undefined;
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
		});
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

	it("skips the fallback for an expired aws_mqtt credential when aborted", async () => {
		const fetchMock = installFetch(() => new Response(null, { status: 204 }));
		restore = fetchMock.restore;
		const controller = new AbortController();
		controller.abort();

		await expect(
			pushToChannel(
				httpHandle({
					transport: {
						type: "aws_mqtt",
						endpoint: "data.iot.example",
						region: "eu-central-1",
						target_client_id: "run-1",
						topic: "runs/run-1/inbox",
						credentials: {
							access_key_id: "AKID",
							secret_access_key: "secret",
							session_token: "token",
							expiration: 1,
						},
					},
					fallback: { type: "http", push_url: FALLBACK_URL, token: "t" },
				}),
				{ request_id: "req-1", value: 1 },
				{ signal: controller.signal },
			),
		).rejects.toThrow(/expired/);
		expect(fetchMock.calls).toHaveLength(0);
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
