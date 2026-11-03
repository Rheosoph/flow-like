import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { RealtimeProviderSession } from "./provider-session";
import type {
	RealtimeProviderCallbacks,
	RealtimeProviderOpener,
} from "./providers";
import { type RealtimeFetch, RealtimeRuntime } from "./runtime";
import type {
	RealtimeCommand,
	RealtimeConnectionSession,
	RealtimeMessage,
	RealtimePort,
} from "./types";

const runtimes: RealtimeRuntime[] = [];
beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(new Date("2026-11-01T00:00:00Z"));
	vi.spyOn(Math, "random").mockReturnValue(0.5);
});
afterEach(() => {
	for (const runtime of runtimes.splice(0)) runtime.dispose();
	vi.useRealTimers();
	vi.restoreAllMocks();
});
const empty = () =>
	Response.json({ protocol_version: 1, runs: [], errors: [] });
const grant = (seconds = 900, url = "wss://nats.test") =>
	Response.json({
		protocol_version: 1,
		expires_at: Math.floor(Date.now() / 1000) + seconds,
		transport: {
			type: "nats",
			url,
			subject: "realtime.account.wake",
			token: "read",
		},
	});
const running = (sequence: number, status = "running") =>
	Response.json({
		protocol_version: 1,
		runs: [
			{
				run_id: "run",
				revision: `r${sequence}`,
				status,
				progress: 0,
				events: [
					{
						sequence,
						event_type: "chunk",
						payload: {},
						created_at: "2026-11-01T00:00:00Z",
					},
				],
				after_sequence: sequence,
				has_more: false,
			},
		],
		errors: [],
	});

function create(
	fetcher: RealtimeFetch,
	open: RealtimeProviderOpener,
	options: ConstructorParameters<typeof RealtimeRuntime>[0] = {},
) {
	const runtime = new RealtimeRuntime({
		fetch: fetcher,
		openProvider: open,
		...options,
	});
	runtimes.push(runtime);
	return runtime;
}

const longLease = { portLeaseMs: 3_600_000 };

function port(
	runtime: RealtimeRuntime,
	id: string,
	transport: "http" | "configured" = "configured",
) {
	const messages: RealtimeMessage[] = [];
	const channel: RealtimePort = {
		postMessage: (message) => messages.push(message),
	};
	const session: RealtimeConnectionSession = {
		scope: "account",
		url: "https://hub.test/api/v1/realtime/poll",
		headers: { Authorization: "Bearer user" },
		transport,
	};
	const send = (command: RealtimeCommand) => runtime.receive(channel, command);
	send({
		type: "connect",
		id,
		session,
		activity: { online: true, visible: true },
	});
	return {
		send,
		messages,
		session,
		subscribe: (runs: { run_id: string }[] = []) =>
			send({
				type: "subscribe",
				id,
				subscription: { notifications: true, runs },
			}),
		close: () => send({ type: "disconnect", id }),
	};
}

function body(call: [RequestInfo | URL, RequestInit?]) {
	return JSON.parse(String(call[1]?.body));
}

/** Answers like the API: a long poll is held for its full wait unless aborted. */
function replay(init?: RequestInit): Promise<Response> {
	const wait = JSON.parse(String(init?.body)).timeout_ms;
	if (!wait) return Promise.resolve(empty());
	return new Promise((resolve, reject) => {
		const timer = setTimeout(() => resolve(empty()), wait);
		init?.signal?.addEventListener(
			"abort",
			() => {
				clearTimeout(timer);
				reject(new Error("aborted"));
			},
			{ once: true },
		);
	});
}
function polls(fetcher: ReturnType<typeof vi.fn<RealtimeFetch>>) {
	return fetcher.mock.calls.filter(([url]) => String(url).endsWith("/poll"));
}

function connects(fetcher: ReturnType<typeof vi.fn<RealtimeFetch>>) {
	return fetcher.mock.calls.filter(([url]) => String(url).endsWith("/connect"));
}

describe("shared provider lifecycle", () => {
	test("an unsupported negotiation route stays on HTTP and is probed on a doubling schedule", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) => {
			if (String(url).endsWith("/connect"))
				return new Response(null, { status: 404 });
			return new Promise((_resolve, reject) =>
				init?.signal?.addEventListener(
					"abort",
					() => reject(new Error("aborted")),
					{ once: true },
				),
			);
		});
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(29_000);
		expect(connects(fetcher)).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(1_001);
		expect(connects(fetcher)).toHaveLength(2);
		await vi.advanceTimersByTimeAsync(59_000);
		expect(connects(fetcher)).toHaveLength(2);
		await vi.advanceTimersByTimeAsync(1_001);
		expect(connects(fetcher)).toHaveLength(3);
		expect(open).not.toHaveBeenCalled();
		expect(
			body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?])
				.timeout_ms,
		).toBe(20_000);
	});

	test("a temporary HTTP grant retries provider negotiation before its expiry", async () => {
		let firstGrant = true;
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) => {
			if (!String(url).endsWith("/connect")) return empty();
			if (!firstGrant) return grant();
			firstGrant = false;
			return Response.json({
				protocol_version: 1,
				expires_at: Math.floor(Date.now() / 1000) + 30,
				transport: { type: "http" },
			});
		});
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(23_000);
		expect(open).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1_001);
		expect(open).toHaveBeenCalledTimes(1);
	});

	test("a hidden account pauses its provider and resumes it with the same grant until renewal is due", async () => {
		const start = Date.now();
		const close = vi.fn();
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant() : empty(),
		);
		const runtime = create(fetcher, open, longLease);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		first.send({
			type: "heartbeat",
			id: "first",
			activity: { online: true, visible: false },
		});
		await vi.advanceTimersByTimeAsync(60_001);
		expect(close).toHaveBeenCalledTimes(1);
		first.send({
			type: "heartbeat",
			id: "first",
			activity: { online: true, visible: true },
		});
		await vi.advanceTimersByTimeAsync(2);
		expect(open).toHaveBeenCalledTimes(2);
		expect(connects(fetcher)).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(start + 870_000 - Date.now() - 1);
		expect(connects(fetcher)).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(2);
		expect(connects(fetcher)).toHaveLength(2);
	});

	test("a provider paused past its grant's renewal time negotiates a new grant on resume", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant(100) : empty(),
		);
		const runtime = create(fetcher, open, longLease);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		first.send({
			type: "heartbeat",
			id: "first",
			activity: { online: false, visible: true },
		});
		await vi.advanceTimersByTimeAsync(80_000);
		expect(connects(fetcher)).toHaveLength(1);
		first.send({
			type: "heartbeat",
			id: "first",
			activity: { online: true, visible: true },
		});
		await vi.advanceTimersByTimeAsync(2);
		expect(connects(fetcher)).toHaveLength(2);
		expect(open).toHaveBeenCalledTimes(2);
	});

	test("an account blocked as unauthorized forgets its grant", async () => {
		let forbidden = false;
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) => {
			if (String(url).endsWith("/connect")) return grant();
			return forbidden ? new Response(null, { status: 403 }) : empty();
		});
		const runtime = create(fetcher, open, longLease);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		forbidden = true;
		first.subscribe([{ run_id: "run" }]);
		await vi.advanceTimersByTimeAsync(2);
		expect(first.messages.at(-1)).toEqual({
			type: "state",
			state: "unauthorized",
		});
		forbidden = false;
		first.send({
			type: "session",
			id: "first",
			session: { ...first.session, headers: { Authorization: "Bearer next" } },
		});
		await vi.advanceTimersByTimeAsync(2);
		expect(connects(fetcher)).toHaveLength(2);
		expect(
			new Headers(connects(fetcher)[1][1]?.headers).get("Authorization"),
		).toBe("Bearer next");
	});

	test("a kept grant is never reused for another account, API or transport preference", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async () => grant());
		const session = new RealtimeProviderSession({
			fetch: fetcher,
			open,
			changed() {},
			wakeup() {},
		});
		const account: RealtimeConnectionSession = {
			scope: "account",
			url: "https://hub.test/api/v1/realtime/poll",
			headers: { Authorization: "Bearer user" },
		};
		const steps: [RealtimeConnectionSession, "configured" | "http"][] = [
			[account, "configured"],
			[{ ...account, scope: "other" }, "configured"],
			[
				{ ...account, url: "https://other.test/api/v1/realtime/poll" },
				"configured",
			],
			[account, "http"],
			[account, "configured"],
		];
		for (const [next, preference] of steps) {
			session.pause();
			session.sync(next, preference);
			await vi.advanceTimersByTimeAsync(1);
		}
		expect(fetcher.mock.calls.map(([url]) => String(url))).toEqual([
			"https://hub.test/api/v1/realtime/connect",
			"https://hub.test/api/v1/realtime/connect",
			"https://other.test/api/v1/realtime/connect",
			"https://hub.test/api/v1/realtime/connect",
			"https://hub.test/api/v1/realtime/connect",
		]);
		session.pause();
		session.sync(account, "configured");
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(5);
		expect(open).toHaveBeenCalledTimes(5);
		session.close();
		session.sync(account, "configured");
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(6);
		session.close();
	});

	test("replay with has_more drains immediately after acknowledgement", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		let sent = false;
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) => {
			if (String(url).endsWith("/connect")) return grant();
			if (JSON.parse(String(init?.body)).timeout_ms === 0 && !sent) {
				sent = true;
				return Response.json({
					protocol_version: 1,
					runs: [
						{
							run_id: "run",
							revision: "r1",
							status: "running",
							progress: 0,
							events: [
								{
									sequence: 0,
									event_type: "chunk",
									payload: {},
									created_at: "2026-11-01T00:00:00Z",
								},
							],
							after_sequence: 0,
							has_more: true,
						},
					],
					errors: [],
				});
			}
			return empty();
		});
		const runtime = create(fetcher, open);
		const first = port(runtime, "first");
		first.subscribe([{ run_id: "run" }]);
		await vi.advanceTimersByTimeAsync(2);
		const snapshot = first.messages.find(
			(message) => message.type === "snapshot",
		);
		if (snapshot?.type !== "snapshot") throw new Error("Missing first replay");
		first.send({
			type: "accepted",
			id: "first",
			delivery_id: snapshot.delivery_id,
		});
		await vi.advanceTimersByTimeAsync(51);
		expect(
			body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?]).runs[0]
				.after_sequence,
		).toBe(0);
		expect(polls(fetcher).length).toBeGreaterThan(2);
	});

	test("one provider serves all configured tabs and replay becomes sparse only after readiness", async () => {
		let ready!: (connection: { close(): void }) => void;
		const close = vi.fn();
		const open = vi.fn<RealtimeProviderOpener>(
			() =>
				new Promise((resolve) => {
					ready = resolve;
				}),
		);
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant() : empty(),
		);
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe();
		port(runtime, "second").subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(open).toHaveBeenCalledTimes(1);
		expect(body(polls(fetcher)[0]).timeout_ms).toBe(20_000);
		ready({ close });
		await vi.advanceTimersByTimeAsync(1);
		expect(
			body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?])
				.timeout_ms,
		).toBe(0);
		const count = polls(fetcher).length;
		await vi.advanceTimersByTimeAsync(29_000);
		expect(polls(fetcher)).toHaveLength(count);
		await vi.advanceTimersByTimeAsync(1_000);
		expect(polls(fetcher)).toHaveLength(count + 1);
	});

	test("coalesces wakeups without cancelling an in-flight replay and ignores duplicate nonces", async () => {
		let callbacks!: RealtimeProviderCallbacks;
		let finish!: (response: Response) => void;
		let pendingSignal: AbortSignal | undefined;
		let hold = false;
		const open = vi.fn<RealtimeProviderOpener>(async (_transport, events) => {
			callbacks = events;
			return { close() {} };
		});
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) => {
			if (String(url).endsWith("/connect")) return grant();
			if (hold) {
				pendingSignal = init?.signal ?? undefined;
				return new Promise((resolve) => {
					finish = resolve;
				});
			}
			return empty();
		});
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(2);
		hold = true;
		callbacks.wakeup({ protocol_version: 1, kind: "invalidate", nonce: "1" });
		await vi.advanceTimersByTimeAsync(101);
		const count = polls(fetcher).length;
		callbacks.wakeup({ protocol_version: 1, kind: "invalidate", nonce: "2" });
		callbacks.wakeup({ protocol_version: 1, kind: "invalidate", nonce: "2" });
		await vi.advanceTimersByTimeAsync(101);
		expect(pendingSignal?.aborted).toBe(false);
		expect(polls(fetcher)).toHaveLength(count);
		hold = false;
		finish(empty());
		await vi.advanceTimersByTimeAsync(1);
		expect(polls(fetcher)).toHaveLength(count + 1);
	});

	test("HTTP preference avoids grants and cannot override another tab's configured preference", async () => {
		const close = vi.fn();
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant() : empty(),
		);
		const runtime = create(fetcher, open);
		const cheap = port(runtime, "cheap", "http");
		cheap.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(open).not.toHaveBeenCalled();
		const configured = port(runtime, "configured");
		configured.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		expect(open).toHaveBeenCalledTimes(1);
		cheap.send({ type: "session", id: "cheap", session: cheap.session });
		await vi.advanceTimersByTimeAsync(1);
		expect(close).not.toHaveBeenCalled();
		configured.close();
		await vi.advanceTimersByTimeAsync(1);
		expect(close).toHaveBeenCalledTimes(1);
		expect(
			body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?])
				.timeout_ms,
		).toBe(20_000);
	});

	test("provider failure immediately restores HTTP while reconnect uses bounded backoff", async () => {
		let callbacks!: RealtimeProviderCallbacks;
		const open = vi.fn<RealtimeProviderOpener>(async (_transport, events) => {
			callbacks = events;
			return { close() {} };
		});
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant() : empty(),
		);
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(2);
		callbacks.disconnected();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?])
				.timeout_ms,
		).toBe(20_000);
		await vi.advanceTimersByTimeAsync(1_900);
		expect(open).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(200);
		expect(open).toHaveBeenCalledTimes(2);
	});

	test("reconnects back off to five minutes and reset only after a minute of stable connection", async () => {
		let callbacks!: RealtimeProviderCallbacks;
		const open = vi.fn<RealtimeProviderOpener>(async (_transport, events) => {
			callbacks = events;
			return { close() {} };
		});
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) =>
			String(url).endsWith("/connect") ? grant(3_600) : replay(init),
		);
		const runtime = create(fetcher, open, longLease);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(2);
		for (const seconds of [2, 4, 8, 16, 32, 64, 128, 256, 300, 300]) {
			const before = open.mock.calls.length;
			callbacks.disconnected();
			await vi.advanceTimersByTimeAsync(seconds * 1000 - 1);
			expect(open).toHaveBeenCalledTimes(before);
			await vi.advanceTimersByTimeAsync(2);
			expect(open).toHaveBeenCalledTimes(before + 1);
		}
		await vi.advanceTimersByTimeAsync(60_000);
		const before = open.mock.calls.length;
		callbacks.disconnected();
		await vi.advanceTimersByTimeAsync(1_999);
		expect(open).toHaveBeenCalledTimes(before);
		await vi.advanceTimersByTimeAsync(2);
		expect(open).toHaveBeenCalledTimes(before + 1);
	});

	test("refuses a cleartext provider for an https API and keeps polling over HTTP", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) =>
			String(url).endsWith("/connect")
				? grant(900, "ws://nats.example.com")
				: replay(init),
		);
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(1_000);
		expect(open).not.toHaveBeenCalled();
		expect(
			body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?])
				.timeout_ms,
		).toBe(20_000);
		await vi.advanceTimersByTimeAsync(1_001);
		expect(connects(fetcher)).toHaveLength(2);
	});

	test.each([
		["https://hub.test/api/v1/realtime/poll", "ws://127.0.0.1:4222"],
		["http://hub.test/api/v1/realtime/poll", "ws://nats.example.com"],
	])("allows cleartext providers next to %s for %s", async (api, provider) => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant(900, provider) : empty(),
		);
		const runtime = create(fetcher, open);
		const first = port(runtime, "first");
		first.send({
			type: "session",
			id: "first",
			session: { ...first.session, url: api },
		});
		first.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		expect(open).toHaveBeenCalledTimes(1);
	});

	test("token refreshes update the grant credentials without reconnecting the provider", async () => {
		const close = vi.fn();
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant(100) : empty(),
		);
		const runtime = create(fetcher, open);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		const count = polls(fetcher).length;
		first.send({
			type: "session",
			id: "first",
			session: { ...first.session, headers: { Authorization: "Bearer next" } },
		});
		await vi.advanceTimersByTimeAsync(2);
		expect(close).not.toHaveBeenCalled();
		expect(open).toHaveBeenCalledTimes(1);
		expect(connects(fetcher)).toHaveLength(1);
		expect(polls(fetcher)).toHaveLength(count);
		await vi.advanceTimersByTimeAsync(80_000);
		expect(connects(fetcher)).toHaveLength(2);
		expect(
			new Headers(connects(fetcher)[1][1]?.headers).get("Authorization"),
		).toBe("Bearer next");
	});

	test("a grant refused for a token refreshed meanwhile is requested again at once, uncounted", async () => {
		let callbacks!: RealtimeProviderCallbacks;
		let refuse: ((response: Response) => void) | undefined;
		const open = vi.fn<RealtimeProviderOpener>(async (_transport, events) => {
			callbacks = events;
			return { close() {} };
		});
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) => {
			if (!String(url).endsWith("/connect")) return replay(init);
			if (new Headers(init?.headers).get("Authorization") === "Bearer next")
				return grant();
			return new Promise((resolve) => {
				refuse = resolve;
			});
		});
		const runtime = create(fetcher, open, longLease);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		first.send({
			type: "session",
			id: "first",
			session: { ...first.session, headers: { Authorization: "Bearer next" } },
		});
		refuse?.(new Response(null, { status: 401 }));
		await vi.advanceTimersByTimeAsync(1);
		expect(connects(fetcher)).toHaveLength(2);
		expect(
			new Headers(connects(fetcher)[1][1]?.headers).get("Authorization"),
		).toBe("Bearer next");
		expect(open).toHaveBeenCalledTimes(1);
		callbacks.disconnected();
		await vi.advanceTimersByTimeAsync(2_001);
		expect(open).toHaveBeenCalledTimes(2);
	});

	test("grant lifetimes follow the server clock even when the local clock is skewed", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const serverNow = Math.floor(Date.now() / 1000) - 3_600;
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect")
				? Response.json({
						protocol_version: 1,
						expires_at: serverNow + 600,
						expires_in: 600,
						max_poll_wait_ms: 20_000,
						transport: {
							type: "nats",
							url: "wss://nats.test",
							subject: "realtime.account.wake",
							token: "read",
						},
					})
				: empty(),
		);
		const runtime = create(fetcher, open, longLease);
		port(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(2);
		expect(open).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(569_000);
		expect(connects(fetcher)).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(1_000);
		expect(connects(fetcher)).toHaveLength(2);
	});

	test("chases a streaming run once its wakeups may have been throttled", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		let sequence = 0;
		const fetcher = vi.fn<RealtimeFetch>(async (url, init) => {
			if (String(url).endsWith("/connect")) return grant();
			if (!body([url, init]).runs.length) return empty();
			sequence++;
			return sequence < 3
				? running(sequence - 1)
				: running(sequence - 1, "COMPLETED");
		});
		const runtime = create(fetcher, open);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(2);
		expect(open).toHaveBeenCalledTimes(1);
		first.subscribe([{ run_id: "run" }]);
		const accept = () => {
			const message = [...first.messages]
				.reverse()
				.find((item) => item.type === "snapshot");
			if (message?.type === "snapshot")
				first.send({
					type: "accepted",
					id: "first",
					delivery_id: message.delivery_id,
				});
		};
		await vi.advanceTimersByTimeAsync(1);
		const settled = polls(fetcher).length;
		accept();
		await vi.advanceTimersByTimeAsync(748);
		expect(polls(fetcher)).toHaveLength(settled);
		await vi.advanceTimersByTimeAsync(2);
		expect(polls(fetcher)).toHaveLength(settled + 1);
		accept();
		await vi.advanceTimersByTimeAsync(751);
		expect(polls(fetcher)).toHaveLength(settled + 2);
		accept();
		await vi.advanceTimersByTimeAsync(4_000);
		expect(polls(fetcher)).toHaveLength(settled + 2);
		await vi.advanceTimersByTimeAsync(1_000);
		expect(polls(fetcher)).toHaveLength(settled + 3);
	});

	test.each([false, true])(
		"a late acknowledgement replays the runs a poll left out at once (that poll still in flight: %s)",
		async (inFlight) => {
			const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
			let served = false;
			let finish: (() => void) | undefined;
			const fetcher = vi.fn<RealtimeFetch>(async (url, init) => {
				if (String(url).endsWith("/connect")) return grant();
				if (!served && body([url, init]).runs.length) {
					served = true;
					return running(0);
				}
				if (served && inFlight && !finish)
					return new Promise((resolve) => {
						finish = () => resolve(empty());
					});
				return empty();
			});
			const runtime = create(fetcher, open);
			port(runtime, "first").subscribe();
			await vi.advanceTimersByTimeAsync(2);
			expect(open).toHaveBeenCalledTimes(1);
			const second = port(runtime, "second");
			second.subscribe([{ run_id: "run" }]);
			await vi.advanceTimersByTimeAsync(1_000);
			const snapshot = second.messages.find(
				(message) => message.type === "snapshot",
			);
			if (snapshot?.type !== "snapshot") throw new Error("Missing replay");
			const count = polls(fetcher).length;
			expect(
				body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?]).runs,
			).toEqual([]);
			second.send({
				type: "accepted",
				id: "second",
				delivery_id: snapshot.delivery_id,
			});
			finish?.();
			await vi.advanceTimersByTimeAsync(1);
			expect(polls(fetcher)).toHaveLength(count + 1);
			expect(
				body(polls(fetcher).at(-1) as [RequestInfo | URL, RequestInit?]).runs,
			).toEqual([{ run_id: "run", after_sequence: 0, revision: "r0" }]);
		},
	);

	test("renews grants before expiry and closes provider credentials on logout", async () => {
		const close = vi.fn();
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant(10) : empty(),
		);
		const runtime = create(fetcher, open);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(8_001);
		expect(open).toHaveBeenCalledTimes(2);
		expect(close).toHaveBeenCalledTimes(1);
		first.close();
		expect(close).toHaveBeenCalledTimes(2);
		const count = fetcher.mock.calls.length;
		await vi.advanceTimersByTimeAsync(30_000);
		expect(fetcher).toHaveBeenCalledTimes(count);
	});

	test("does not open a provider when its grant arrives after logout", async () => {
		let finish!: (response: Response) => void;
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect")
				? new Promise((resolve) => {
						finish = resolve;
					})
				: empty(),
		);
		const runtime = create(fetcher, open);
		const first = port(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		first.close();
		finish(grant());
		await vi.advanceTimersByTimeAsync(1);
		expect(open).not.toHaveBeenCalled();
	});

	test("active run replay recovers within five seconds without a wakeup", async () => {
		const open = vi.fn<RealtimeProviderOpener>(async () => ({ close() {} }));
		const fetcher = vi.fn<RealtimeFetch>(async (url) =>
			String(url).endsWith("/connect") ? grant() : empty(),
		);
		const runtime = create(fetcher, open);
		port(runtime, "first").subscribe([{ run_id: "run" }]);
		await vi.advanceTimersByTimeAsync(2);
		const count = polls(fetcher).length;
		await vi.advanceTimersByTimeAsync(5_000);
		expect(polls(fetcher)).toHaveLength(count + 1);
	});
});
