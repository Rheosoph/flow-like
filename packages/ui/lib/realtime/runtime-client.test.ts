import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { type RealtimeFetch, RealtimeRuntime } from "./runtime";
import type {
	RealtimeClient,
	RealtimeCommand,
	RealtimeMessage,
	RealtimePollResponse,
	RealtimePort,
	RealtimeReply,
} from "./types";

const clients: RealtimeClient[] = [];
const runtimes: RealtimeRuntime[] = [];
beforeEach(() => {
	vi.resetModules();
	vi.useFakeTimers();
	vi.stubGlobal("window", {});
});
afterEach(async () => {
	for (const client of clients.splice(0)) client.close();
	await vi.advanceTimersByTimeAsync(1);
	for (const runtime of runtimes.splice(0)) runtime.dispose();
	vi.useRealTimers();
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

const session = {
	scope: "account",
	url: "https://hub.test/api/v1/realtime/poll",
	headers: { Authorization: "Bearer token" },
	transport: "http" as const,
};

/** `fetcher` sees replay polls only; negotiation answers with an HTTP grant. */
function replayOnly(fetcher: RealtimeFetch): RealtimeFetch {
	return (url, init) =>
		String(url).endsWith("/connect")
			? Promise.resolve(
					Response.json({
						protocol_version: 1,
						expires_at: Math.floor(Date.now() / 1000) + 900,
						expires_in: 900,
						max_poll_wait_ms: 20_000,
						transport: { type: "http" },
					}),
				)
			: fetcher(url, init);
}

function hold(_url: RequestInfo | URL, init?: RequestInit): Promise<Response> {
	return new Promise((_, reject) => {
		init?.signal?.addEventListener(
			"abort",
			() => reject(new Error("aborted")),
			{ once: true },
		);
	});
}

const notification: RealtimePollResponse = {
	protocol_version: 1,
	notifications: {
		revision: "n1",
		overview: { invites_count: 0, notifications_count: 1, unread_count: 1 },
	},
	runs: [],
	errors: [],
};

function worker(
	runtime: RealtimeRuntime,
	intercept?: (message: RealtimeMessage, deliver: () => void) => void,
) {
	runtimes.push(runtime);
	const instances: {
		onerror: (() => void) | null;
		port: {
			onmessage: ((message: MessageEvent<RealtimeMessage>) => void) | null;
		};
	}[] = [];
	vi.stubGlobal(
		"SharedWorker",
		class {
			onerror: (() => void) | null = null;
			port: {
				onmessage: ((message: MessageEvent<RealtimeMessage>) => void) | null;
				postMessage: (command: RealtimeCommand) => void;
			};
			constructor() {
				const runtimePort: RealtimePort = {
					postMessage: (message) => {
						const deliver = () =>
							queueMicrotask(() =>
								this.port.onmessage?.({
									data: structuredClone(message),
								} as MessageEvent<RealtimeMessage>),
							);
						if (intercept) intercept(message, deliver);
						else deliver();
					},
				};
				this.port = {
					onmessage: null,
					postMessage: (command) =>
						queueMicrotask(() =>
							runtime.receive(runtimePort, structuredClone(command)),
						),
				};
				instances.push(this);
			}
		},
	);
	return instances;
}

async function connect() {
	const { getRealtimeClient } = await import("./runtime-client");
	const client = await getRealtimeClient(session);
	clients.push(client);
	return client;
}

describe("browser realtime connection", () => {
	test.each(["event", "error"])(
		"old %s delivery cannot mutate replacement capability cursors before fallback",
		async (kind) => {
			const oldSnapshot: RealtimePollResponse = {
				protocol_version: 1,
				errors: [],
				runs: [],
			};
			if (kind === "error")
				oldSnapshot.errors.push({ run_id: "a", code: "not_found" });
			else
				oldSnapshot.runs.push({
					run_id: "a",
					revision: "old",
					status: "running",
					progress: 0,
					events: [
						{
							sequence: 0,
							event_type: "chunk",
							payload: "old",
							created_at: "now",
						},
					],
					after_sequence: 0,
					has_more: false,
				});
			const fetcher = vi
				.fn(hold)
				.mockResolvedValueOnce(Response.json(oldSnapshot));
			vi.stubGlobal("fetch", replayOnly(fetcher));
			const instances = worker(
				new RealtimeRuntime({ fetch: replayOnly(fetcher) }),
			);
			const client = await connect();
			let finish!: () => void;
			let observed = false;
			client.subscribe((update) => {
				if (update.type !== "snapshot") return;
				observed = true;
				return new Promise<void>((resolve) => {
					finish = resolve;
				});
			});
			client.setSubscriptions({
				notifications: false,
				runs: [{ run_id: "a", poll_token: "old-token" }],
			});
			await vi.advanceTimersByTimeAsync(1);
			expect(observed).toBe(true);
			client.setSubscriptions({
				notifications: false,
				runs: [{ run_id: "a", poll_token: "new-token", after_sequence: -1 }],
			});
			finish();
			await vi.advanceTimersByTimeAsync(1);
			instances[0].onerror?.();
			await vi.advanceTimersByTimeAsync(1);
			expect(
				JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).runs,
			).toEqual([{ run_id: "a", poll_token: "new-token", after_sequence: -1 }]);
		},
	);

	test("a queued snapshot from a previous subscription never reaches its replacement listener", async () => {
		const queued: (() => void)[] = [];
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(Response.json(notification));
		vi.stubGlobal("fetch", replayOnly(fetcher));
		const instances = worker(
			new RealtimeRuntime({ fetch: replayOnly(fetcher) }),
			(message, deliver) => {
				if (message.type === "snapshot") queued.push(deliver);
				else deliver();
			},
		);
		const client = await connect();
		const listener = vi.fn();
		client.subscribe(listener);
		client.setSubscriptions({ notifications: true, runs: [] });
		await vi.advanceTimersByTimeAsync(1);
		expect(queued).toHaveLength(1);
		client.setSubscriptions({
			notifications: false,
			runs: [{ run_id: "a", poll_token: "replacement", after_sequence: -1 }],
		});
		queued[0]();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			listener.mock.calls.some(([update]) => update.type === "snapshot"),
		).toBe(false);
		instances[0].onerror?.();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).runs,
		).toEqual([{ run_id: "a", poll_token: "replacement", after_sequence: -1 }]);
	});

	test("rejects invalid subscriptions before changing the active subscription", async () => {
		const fetcher = vi.fn(hold);
		worker(new RealtimeRuntime({ fetch: replayOnly(fetcher) }));
		const client = await connect();
		client.setSubscriptions({ notifications: true, runs: [] });
		for (const runs of [
			[{ run_id: "a", revision: "" }],
			[{ run_id: "bad\nid" }],
			[{ run_id: "a" }, { run_id: "a" }],
			[{ run_id: "a", after_sequence: -2 }],
			[{ run_id: "a", poll_token: "" }],
			[{ run_id: "a", poll_token: "invalid\ntoken" }],
			[{ run_id: "a", poll_token: "x".repeat(8193) }],
		]) {
			expect(() =>
				client.setSubscriptions({ notifications: false, runs }),
			).toThrow("Invalid realtime subscriptions");
		}
		await vi.advanceTimersByTimeAsync(1);
		expect(JSON.parse(String(fetcher.mock.calls[0][1]?.body))).toMatchObject({
			notifications: {},
			runs: [],
		});
	});

	test("falls back to the same runtime when SharedWorker construction fails", async () => {
		vi.stubGlobal(
			"SharedWorker",
			class {
				constructor() {
					throw new Error("blocked");
				}
			},
		);
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(Response.json(notification));
		vi.stubGlobal("fetch", replayOnly(fetcher));
		const client = await connect();
		const received: RealtimePollResponse[] = [];
		client.subscribe((update) => {
			if (update.type === "snapshot") received.push(update.snapshot);
		});
		client.setSubscriptions({ notifications: true, runs: [] });
		await vi.advanceTimersByTimeAsync(1);
		expect(received).toEqual([notification]);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	test("two worker ports share networking and remain independent when one closes", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(Response.json(notification));
		worker(new RealtimeRuntime({ fetch: replayOnly(fetcher) }));
		const first = await connect();
		const second = await connect();
		const received = vi.fn();
		second.subscribe(received);
		first.setSubscriptions({ notifications: true, runs: [] });
		second.setSubscriptions({ notifications: true, runs: [] });
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(1);
		expect(received).toHaveBeenCalledWith({
			type: "snapshot",
			snapshot: notification,
		});
		first.close();
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(2);
	});

	test("worker failure moves accepted run cursors into the fallback", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			Response.json({
				protocol_version: 1,
				runs: [
					{
						run_id: "a",
						revision: "r1",
						status: "running",
						progress: 1,
						events: [
							{
								sequence: 0,
								event_type: "chunk",
								payload: "text",
								created_at: "now",
							},
						],
						after_sequence: 0,
						has_more: false,
					},
				],
				errors: [],
			}),
		);
		vi.stubGlobal("fetch", replayOnly(fetcher));
		const instances = worker(
			new RealtimeRuntime({ fetch: replayOnly(fetcher) }),
		);
		const client = await connect();
		client.subscribe(() => {});
		client.setSubscriptions({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		instances[0].onerror?.();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).runs[0]
				.after_sequence,
		).toBe(0);
	});

	test("worker failure moves accepted conversation cursors into the fallback", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			Response.json({
				protocol_version: 1,
				runs: [],
				errors: [],
				conversations: [
					{
						conversation_id: "c",
						entries: [{ id: "e1", message_id: "m1", cursor: "1:e1" }],
						has_more: false,
					},
				],
			}),
		);
		vi.stubGlobal("fetch", replayOnly(fetcher));
		const instances = worker(
			new RealtimeRuntime({ fetch: replayOnly(fetcher) }),
		);
		const client = await connect();
		client.subscribe(() => {});
		client.setSubscriptions({
			notifications: false,
			runs: [],
			conversations: [{ conversation_id: "c" }],
		});
		await vi.advanceTimersByTimeAsync(1);
		instances[0].onerror?.();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).conversations,
		).toEqual([{ conversation_id: "c", after: "1:e1" }]);
	});

	test("a temporarily unavailable run stays subscribed across a worker fallback", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			Response.json({
				protocol_version: 1,
				runs: [],
				errors: [{ run_id: "a", code: "unavailable" }],
			}),
		);
		vi.stubGlobal("fetch", replayOnly(fetcher));
		const instances = worker(
			new RealtimeRuntime({ fetch: replayOnly(fetcher) }),
		);
		const client = await connect();
		const errors: string[] = [];
		client.subscribe((update) => {
			if (update.type === "snapshot")
				errors.push(...update.snapshot.errors.map((error) => error.code));
		});
		client.setSubscriptions({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		expect(errors).toEqual(["unavailable"]);
		instances[0].onerror?.();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).runs,
		).toEqual([{ run_id: "a", after_sequence: -1 }]);
	});

	test("a suspended tab registers again with its accepted cursor", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			Response.json({
				protocol_version: 1,
				runs: [
					{
						run_id: "a",
						revision: "r1",
						status: "running",
						progress: 1,
						events: [
							{
								sequence: 0,
								event_type: "chunk",
								payload: "text",
								created_at: "now",
							},
						],
						after_sequence: 0,
						has_more: false,
					},
				],
				errors: [],
			}),
		);
		worker(
			new RealtimeRuntime({
				fetch: replayOnly(fetcher),
				portLeaseMs: 50,
				sweepMs: 20,
			}),
		);
		const client = await connect();
		client.setSubscriptions({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(61);
		expect(fetcher.mock.calls.length).toBeGreaterThanOrEqual(2);
		expect(
			JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).runs[0]
				.after_sequence,
		).toBe(0);
	});

	test("a failing consumer retries the same unacknowledged batch", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(Response.json(notification));
		worker(new RealtimeRuntime({ fetch: replayOnly(fetcher) }));
		const client = await connect();
		let attempts = 0;
		client.subscribe((update) => {
			if (update.type === "snapshot" && ++attempts === 1)
				throw new Error("cache write failed");
		});
		client.setSubscriptions({ notifications: true, runs: [] });
		await vi.advanceTimersByTimeAsync(15_001);
		expect(attempts).toBe(2);
	});

	test("a reply reaches listeners and is never acknowledged", async () => {
		const fetcher = vi.fn(hold);
		vi.stubGlobal("fetch", replayOnly(fetcher));
		const runtime = new RealtimeRuntime({ fetch: replayOnly(fetcher) });
		const received = vi.spyOn(runtime, "receive");
		const instances = worker(runtime);
		const client = await connect();
		const listener = vi.fn();
		client.subscribe(listener);
		await vi.advanceTimersByTimeAsync(1);
		const reply: RealtimeReply = {
			protocol_version: 1,
			kind: "reply",
			conversation_id: "c1",
			message_id: "re-1",
			reply_to: "m1",
			sender_id: "bot",
			offset: 0,
			text: "Hello",
			state: "writing",
		};
		const before = received.mock.calls.length;
		instances[0].port.onmessage?.({
			data: { type: "reply", reply },
		} as MessageEvent<RealtimeMessage>);
		await vi.advanceTimersByTimeAsync(1);
		expect(listener).toHaveBeenCalledWith({ type: "reply", reply });
		expect(
			received.mock.calls.slice(before).map(([, command]) => command.type),
		).not.toContain("accepted");
	});
});
