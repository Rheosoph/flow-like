import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import {
	type RealtimeFetch,
	RealtimeRuntime,
	parseRealtimeResponse,
} from "./runtime";
import type {
	RealtimeCommand,
	RealtimeConnectionSession,
	RealtimeMessage,
	RealtimePollRequest,
	RealtimePollResponse,
	RealtimePort,
	RealtimeRunSnapshot,
	RealtimeSubscription,
} from "./types";

const runtimes: RealtimeRuntime[] = [];
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
	for (const runtime of runtimes.splice(0)) runtime.dispose();
	vi.useRealTimers();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

function httpGrant(overrides: Record<string, unknown> = {}) {
	return Response.json({
		protocol_version: 1,
		expires_at: Math.floor(Date.now() / 1000) + 900,
		expires_in: 900,
		max_poll_wait_ms: 20_000,
		transport: { type: "http" },
		...overrides,
	});
}

/** `fetcher` sees replay polls only; negotiation answers with an HTTP grant. */
function create(
	fetcher: RealtimeFetch,
	options: ConstructorParameters<typeof RealtimeRuntime>[0] = {},
	grant = () => httpGrant(),
) {
	const runtime = new RealtimeRuntime({
		fetch: (url, init) =>
			String(url).endsWith("/connect")
				? Promise.resolve(grant())
				: fetcher(url, init),
		...options,
	});
	runtimes.push(runtime);
	return runtime;
}

function connect(
	runtime: RealtimeRuntime,
	id: string,
	scope = "account-a",
	onMessage?: (message: RealtimeMessage) => void,
) {
	const messages: RealtimeMessage[] = [];
	const port: RealtimePort = {
		postMessage(message) {
			messages.push(message);
			onMessage?.(message);
		},
	};
	const session: RealtimeConnectionSession = {
		scope,
		url: "https://hub.test/api/v1/realtime/poll",
		headers: { Authorization: `Bearer ${scope}` },
		transport: "http",
	};
	const send = (command: RealtimeCommand) => runtime.receive(port, command);
	send({
		type: "connect",
		id,
		session,
		activity: { online: true, visible: true },
	});
	return {
		messages,
		send,
		session,
		subscribe: (
			subscription: RealtimeSubscription = { notifications: true, runs: [] },
		) => send({ type: "subscribe", id, subscription }),
		accept: () => {
			const message = [...messages]
				.reverse()
				.find((item) => item.type === "snapshot");
			if (message?.type === "snapshot")
				send({ type: "accepted", id, delivery_id: message.delivery_id });
		},
		activity: (online: boolean, visible = true) =>
			send({ type: "heartbeat", id, activity: { online, visible } }),
		close: () => send({ type: "disconnect", id }),
	};
}

function hold(_url: RequestInfo | URL, init?: RequestInit): Promise<Response> {
	return new Promise((_, reject) => {
		if (init?.signal?.aborted) reject(new Error("aborted"));
		init?.signal?.addEventListener(
			"abort",
			() => reject(new Error("aborted")),
			{ once: true },
		);
	});
}

function response(overrides: Partial<RealtimePollResponse> = {}) {
	return Response.json({
		protocol_version: 1,
		runs: [],
		errors: [],
		...overrides,
	});
}

function run(
	id: string,
	sequences: number[],
	revision = "r1",
): RealtimeRunSnapshot {
	return {
		run_id: id,
		revision,
		status: "running",
		progress: 10,
		events: sequences.map((sequence) => ({
			sequence,
			event_type: "chunk",
			payload: { text: String(sequence) },
			created_at: "2026-11-01T00:00:00Z",
		})),
		after_sequence: sequences.at(-1) ?? -1,
		has_more: false,
	};
}

/**
 * Replays runs like the API: the revision covers the status and whether the store reached the
 * end, a run without news is left out and a long poll without news is held. `confirmEnd` reports
 * more after every page with events, like a store that confirms the end with one more read;
 * `cap` cuts a page the store already finished, like the response budget.
 */
function replayApi(state: {
	status: string;
	last: number;
	cap?: number;
	confirmEnd?: boolean;
}) {
	return vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
		const request: RealtimePollRequest = JSON.parse(String(init?.body));
		const runs = request.runs.flatMap((subscription) => {
			const after = subscription.after_sequence ?? -1;
			const stored = Array.from(
				{ length: Math.max(0, state.last - after) },
				(_, index) => after + 1 + index,
			);
			const more = Boolean(state.confirmEnd && stored.length);
			const revision = `${state.status}:${state.status === "COMPLETED" && !more}`;
			if (!stored.length && subscription.revision === revision) return [];
			const sequences = stored.slice(0, state.cap);
			return [
				{
					...run(subscription.run_id, sequences, revision),
					status: state.status,
					after_sequence: sequences.at(-1) ?? after,
					has_more: more || sequences.length < stored.length,
				},
			];
		});
		return runs.length || !request.timeout_ms
			? response({ runs })
			: hold(url, init);
	});
}

const notification = {
	revision: "n1",
	overview: { invites_count: 1, notifications_count: 2, unread_count: 1 },
};
const snapshots = (port: ReturnType<typeof connect>) =>
	port.messages.filter((message) => message.type === "snapshot");

describe("shared realtime runtime", () => {
	test("rotates run capabilities without sharing authorized events with another capability", async () => {
		const requests: {
			runs: { run_id: string; poll_token?: string; after_sequence: number }[];
		}[] = [];
		const runtime = create(async (_url, init) => {
			const request = JSON.parse(String(init?.body));
			requests.push(request);
			const token = request.runs[0].poll_token;
			return token
				? response({
						runs: [
							{
								...run("a", [0]),
								events: [{ ...run("a", [0]).events[0], payload: token }],
							},
						],
					})
				: response({ errors: [{ run_id: "a", code: "unavailable" }] });
		});
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		const noCapability = connect(runtime, "without");
		first.subscribe({
			notifications: false,
			runs: [{ run_id: "a", poll_token: "token-a" }],
		});
		second.subscribe({
			notifications: false,
			runs: [{ run_id: "a", poll_token: "token-b" }],
		});
		noCapability.subscribe({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)[0].snapshot.runs[0].events[0].payload).toBe(
			"token-a",
		);
		expect(snapshots(second)).toHaveLength(0);
		expect(snapshots(noCapability)).toHaveLength(0);
		first.accept();
		await vi.advanceTimersByTimeAsync(50);
		expect(snapshots(second)[0].snapshot.runs[0].events[0].payload).toBe(
			"token-b",
		);
		expect(snapshots(first)).toHaveLength(1);
		expect(snapshots(noCapability)).toHaveLength(0);
		second.accept();
		await vi.advanceTimersByTimeAsync(50);
		expect(snapshots(noCapability)[0].snapshot.errors).toEqual([
			{ run_id: "a", code: "unavailable" },
		]);
		expect(requests.map((request) => request.runs[0].poll_token)).toEqual([
			"token-a",
			"token-b",
			undefined,
		]);
		noCapability.close();
		second.close();
		await vi.advanceTimersByTimeAsync(1);
		expect(requests.at(-1)?.runs[0]).toMatchObject({
			poll_token: "token-a",
			after_sequence: 0,
		});
	});

	test("uses one connection per account and routes notification snapshots only to interested tabs", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(response({ notifications: notification }));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		const third = connect(runtime, "third");
		first.subscribe();
		second.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(1);
		expect(snapshots(first)).toHaveLength(1);
		expect(snapshots(second)).toHaveLength(1);
		expect(snapshots(third)).toHaveLength(0);
		const other = connect(runtime, "other", "account-b");
		other.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(2);
		expect(
			new Headers(fetcher.mock.calls[1][1]?.headers).get("Authorization"),
		).toBe("Bearer account-b");
	});

	test("filters per-tab runs and independently acknowledges replay cursors", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(
				response({ runs: [run("a", [0, 1, 2]), run("b", [0])] }),
			);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		first.subscribe({
			notifications: false,
			runs: [{ run_id: "a", after_sequence: -1 }],
		});
		second.subscribe({
			notifications: false,
			runs: [
				{ run_id: "a", after_sequence: 1 },
				{ run_id: "b", after_sequence: -1 },
			],
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(
			snapshots(first)[0].snapshot.runs.map((item) => item.run_id),
		).toEqual(["a"]);
		expect(
			snapshots(first)[0].snapshot.runs[0].events.map((item) => item.sequence),
		).toEqual([0, 1, 2]);
		expect(
			snapshots(second)[0].snapshot.runs[0].events.map((item) => item.sequence),
		).toEqual([2]);
		second.accept();
		await vi.advanceTimersByTimeAsync(60);
		const request = JSON.parse(String(fetcher.mock.calls[1][1]?.body));
		expect(
			request.runs.find((item: { run_id: string }) => item.run_id === "a")
				.after_sequence,
		).toBe(2);
		first.close();
		expect(fetcher.mock.calls[1][1]?.signal?.aborted).toBe(true);
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(3);
	});

	test("keeps an unaccepted cursor for a repeated subscription", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(response({ runs: [run("a", [0])] }));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		first.subscribe({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		expect(
			JSON.parse(String(fetcher.mock.calls[1][1]?.body)).runs[0].after_sequence,
		).toBe(-1);
	});

	test("keeps differing revisions unknown when a later tab supplies the lower cursor", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher);
		connect(runtime, "first").subscribe({
			notifications: false,
			runs: [{ run_id: "a", after_sequence: 10, revision: "older-status" }],
		});
		connect(runtime, "second").subscribe({
			notifications: false,
			runs: [{ run_id: "a", after_sequence: 2, revision: "newer-status" }],
		});
		connect(runtime, "third").subscribe({
			notifications: false,
			runs: [{ run_id: "a", after_sequence: 1, revision: "newer-status" }],
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(JSON.parse(String(fetcher.mock.calls[0][1]?.body)).runs).toEqual([
			{ run_id: "a", after_sequence: 1 },
		]);
	});

	test("a finished run's closing page reaches the tab that accepted its last events", async () => {
		const fetcher = replayApi({
			status: "COMPLETED",
			last: 1,
			confirmEnd: true,
		});
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)[0].snapshot.runs[0]).toMatchObject({
			status: "COMPLETED",
			after_sequence: 1,
			has_more: true,
		});
		first.accept();
		await vi.advanceTimersByTimeAsync(60);
		expect(snapshots(first)[1]?.snapshot.runs[0]).toMatchObject({
			events: [],
			after_sequence: 1,
			has_more: false,
		});
	});

	test.each([{ confirmEnd: true }, { cap: 2 }])(
		"a caught-up tab receives the end of a run another tab replays from behind: %j",
		async (paging) => {
			const state = { status: "running", last: 3, ...paging };
			const fetcher = replayApi(state);
			const runtime = create(fetcher);
			const follow = (id: string) => {
				const port: ReturnType<typeof connect> = connect(
					runtime,
					id,
					"account-a",
					(message) => {
						if (message.type === "snapshot")
							queueMicrotask(() =>
								port.send({
									type: "accepted",
									id,
									delivery_id: message.delivery_id,
								}),
							);
					},
				);
				port.subscribe({
					notifications: false,
					runs: [{ run_id: "a", poll_token: "token" }],
				});
				return port;
			};
			const latest = (port: ReturnType<typeof connect>) =>
				snapshots(port).at(-1)?.snapshot.runs[0];
			const ahead = follow("ahead");
			await vi.advanceTimersByTimeAsync(200);
			expect(latest(ahead)?.after_sequence).toBe(3);
			state.status = "COMPLETED";
			const behind = follow("behind");
			await vi.advanceTimersByTimeAsync(1);
			expect(
				JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).runs,
			).toEqual([{ run_id: "a", poll_token: "token", after_sequence: -1 }]);
			await vi.advanceTimersByTimeAsync(500);
			for (const port of [behind, ahead])
				expect(latest(port)).toMatchObject({
					status: "COMPLETED",
					after_sequence: 3,
					has_more: false,
				});
		},
	);

	test("rejects an advanced cursor without corresponding events", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(
				response({ runs: [{ ...run("a", [0]), after_sequence: 5 }] }),
			);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe({ notifications: false, runs: [{ run_id: "a" }] });
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)).toHaveLength(0);
		await vi.advanceTimersByTimeAsync(20_000);
		expect(fetcher).toHaveBeenCalledTimes(1);
		expect(first.messages).toContainEqual({
			type: "state",
			state: "unsupported",
		});
	});

	test.each([
		{ run_id: "a", revision: "" },
		{ run_id: "a", revision: "bad\nrevision" },
		{ run_id: "bad\u0085id" },
		{ run_id: "あ".repeat(100) },
		{ run_id: "a", after_sequence: 2_147_483_647 },
	])(
		"rejects invalid port subscriptions without poisoning another tab: %j",
		async (invalid) => {
			const fetcher = vi.fn(hold);
			const runtime = create(fetcher);
			const first = connect(runtime, "first");
			const second = connect(runtime, "second");
			first.subscribe();
			second.subscribe({ notifications: false, runs: [invalid] });
			await vi.advanceTimersByTimeAsync(1);
			expect(fetcher).toHaveBeenCalledTimes(1);
			expect(JSON.parse(String(fetcher.mock.calls[0][1]?.body)).runs).toEqual(
				[],
			);
		},
	);

	test.each([
		JSON.stringify({ protocol_version: 2, runs: [], errors: [] }),
		JSON.stringify({ protocol_version: 1, runs: "invalid", errors: [] }),
		"<html>Fallback page</html>",
	])(
		"reports a response outside the protocol as unsupported and probes again later: %s",
		async (body) => {
			vi.spyOn(Math, "random").mockReturnValue(0.5);
			const fetcher = vi.fn(hold).mockResolvedValueOnce(new Response(body));
			const runtime = create(fetcher);
			const first = connect(runtime, "first");
			first.subscribe();
			await vi.advanceTimersByTimeAsync(29_000);
			expect(fetcher).toHaveBeenCalledTimes(1);
			expect(first.messages).toContainEqual({
				type: "state",
				state: "unsupported",
			});
			await vi.advanceTimersByTimeAsync(1_001);
			expect(fetcher).toHaveBeenCalledTimes(2);
		},
	);

	test("drops a run the API cannot find without interrupting the inbox", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			response({
				notifications: notification,
				errors: [{ run_id: "denied", code: "not_found" }],
			}),
		);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe({ notifications: true, runs: [{ run_id: "denied" }] });
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)[0].snapshot.errors).toEqual([
			{ run_id: "denied", code: "not_found" },
		]);
		first.accept();
		await vi.advanceTimersByTimeAsync(60);
		expect(JSON.parse(String(fetcher.mock.calls[1][1]?.body))).toMatchObject({
			notifications: { revision: "n1" },
			runs: [],
		});
	});

	test("keeps a temporarily unavailable run and its cursor while backing off", async () => {
		const start = Date.now();
		const polls: { at: number; after?: number }[] = [];
		let unavailable = 2;
		const fetcher = vi.fn(
			async (_url: RequestInfo | URL, init?: RequestInit) => {
				const request = JSON.parse(String(init?.body));
				polls.push({
					at: Date.now() - start,
					after: request.runs[0]?.after_sequence,
				});
				return unavailable-- > 0
					? response({ errors: [{ run_id: "a", code: "unavailable" }] })
					: response({ runs: [run("a", [4])] });
			},
		);
		const runtime = create(fetcher);
		const first: ReturnType<typeof connect> = connect(
			runtime,
			"first",
			"account-a",
			(message) => {
				if (message.type === "snapshot")
					queueMicrotask(() =>
						first.send({
							type: "accepted",
							id: "first",
							delivery_id: message.delivery_id,
						}),
					);
			},
		);
		first.subscribe({
			notifications: false,
			runs: [{ run_id: "a", after_sequence: 3 }],
		});
		await vi.advanceTimersByTimeAsync(6_001);
		expect(polls.slice(0, 3)).toEqual([
			{ at: 0, after: 3 },
			{ at: 2_000, after: 3 },
			{ at: 6_000, after: 3 },
		]);
		expect(snapshots(first).at(-1)?.snapshot.runs[0].events).toHaveLength(1);
		expect(first.messages).not.toContainEqual({
			type: "state",
			state: "unsupported",
		});
	});

	test("rotates a bounded union of run subscriptions across tabs", async () => {
		const fetcher = vi.fn(async () => response());
		const runtime = create(fetcher);
		connect(runtime, "first").subscribe({
			notifications: false,
			runs: Array.from({ length: 16 }, (_, index) => ({ run_id: `a${index}` })),
		});
		connect(runtime, "second").subscribe({
			notifications: false,
			runs: [{ run_id: "b" }],
		});
		await vi.advanceTimersByTimeAsync(2_500);
		const bodies = fetcher.mock.calls.map((call) =>
			JSON.parse(String((call as unknown as [unknown, RequestInit])[1].body)),
		);
		expect(bodies[0].runs).toHaveLength(16);
		expect(
			bodies[1].runs.some((item: { run_id: string }) => item.run_id === "b"),
		).toBe(true);
	});

	test("asks another live tab to renew after the original tab closes", async () => {
		const fetcher = vi.fn(
			async (_url: RequestInfo | URL, init?: RequestInit) => {
				if (
					new Headers(init?.headers).get("Authorization") === "Bearer renewed"
				)
					return hold(_url, init);
				return new Response(null, { status: 401 });
			},
		);
		const runtime = create(fetcher);
		const first = connect(runtime, "first", "account-a", (message) => {
			if (message.type === "unauthorized") first.close();
		});
		const second = connect(runtime, "second", "account-a", (message) => {
			if (message.type === "unauthorized")
				second.send({
					type: "authorize",
					id: "second",
					request_id: message.request_id,
					headers: { Authorization: "Bearer renewed" },
				});
		});
		first.subscribe();
		second.subscribe();
		await vi.advanceTimersByTimeAsync(150);
		expect(
			fetcher.mock.calls.some(
				(call) =>
					new Headers(call[1]?.headers).get("Authorization") ===
					"Bearer renewed",
			),
		).toBe(true);
	});

	test("keeps reporting an unsupported endpoint and probes it on a doubling schedule", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.5);
		const fetcher = vi.fn(async () => new Response(null, { status: 404 }));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(29_000);
		expect(fetcher).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(1_001);
		expect(fetcher).toHaveBeenCalledTimes(2);
		await vi.advanceTimersByTimeAsync(59_000);
		expect(fetcher).toHaveBeenCalledTimes(2);
		await vi.advanceTimersByTimeAsync(1_001);
		expect(fetcher).toHaveBeenCalledTimes(3);
		expect(
			first.messages.filter((message) => message.type === "state"),
		).toEqual([
			{ type: "state", state: "paused" },
			{ type: "state", state: "connecting" },
			{ type: "state", state: "unsupported" },
		]);
	});

	test("probes a forbidden account again and clears the state on success", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.5);
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(new Response(null, { status: 403 }))
			.mockResolvedValueOnce(response({ notifications: notification }));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(29_000);
		expect(first.messages.at(-1)).toEqual({
			type: "state",
			state: "unauthorized",
		});
		expect(fetcher).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(1_001);
		expect(first.messages).toContainEqual({
			type: "state",
			state: "connected",
		});
		expect(snapshots(first)).toHaveLength(1);
	});

	test("a signed-out account waits for new credentials instead of probing", async () => {
		const fetcher = vi.fn(
			async (_url: RequestInfo | URL, init?: RequestInit) =>
				new Headers(init?.headers).get("Authorization") === "Bearer renewed"
					? response({ notifications: notification })
					: new Response(null, { status: 401 }),
		);
		const runtime = create(fetcher, { portLeaseMs: 3_600_000 });
		const first = connect(runtime, "first", "account-a", (message) => {
			if (message.type === "unauthorized")
				first.send({
					type: "authorize",
					id: "first",
					request_id: message.request_id,
				});
		});
		first.subscribe();
		await vi.advanceTimersByTimeAsync(10);
		expect(first.messages.at(-1)).toEqual({
			type: "state",
			state: "unauthorized",
		});
		const count = fetcher.mock.calls.length;
		await vi.advanceTimersByTimeAsync(600_000);
		expect(fetcher).toHaveBeenCalledTimes(count);
		first.send({
			type: "session",
			id: "first",
			session: {
				...first.session,
				headers: { Authorization: "Bearer renewed" },
			},
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)).toHaveLength(1);
		expect(first.messages).toContainEqual({
			type: "state",
			state: "connected",
		});
	});

	test("a newly connected tab recovers an account blocked on old credentials", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(new Response(null, { status: 403 }));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(first.messages).toContainEqual({
			type: "state",
			state: "unauthorized",
		});
		const port = { postMessage: vi.fn() };
		runtime.receive(port, {
			type: "connect",
			id: "renewed",
			session: {
				...first.session,
				headers: { Authorization: "Bearer renewed" },
			},
			activity: { online: true, visible: true },
		});
		runtime.receive(port, {
			type: "subscribe",
			id: "renewed",
			subscription: { notifications: true, runs: [] },
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(2);
		expect(
			new Headers(fetcher.mock.calls[1][1]?.headers).get("Authorization"),
		).toBe("Bearer renewed");
	});

	test("cancels an oversized streaming response before consuming its remaining body", async () => {
		const cancel = vi.fn();
		const body = new ReadableStream({
			start(controller) {
				controller.enqueue(new Uint8Array(4 * 1024 * 1024));
				controller.enqueue(new Uint8Array(1));
			},
			cancel,
		});
		const fetcher = vi.fn(hold).mockResolvedValueOnce(new Response(body));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(cancel).toHaveBeenCalledTimes(1);
		expect(snapshots(first)).toHaveLength(0);
		expect(fetcher.mock.calls[0][1]?.redirect).toBe("error");
	});

	test("accepts responses up to the 4 MiB protocol budget", async () => {
		const body = `${JSON.stringify({
			protocol_version: 1,
			notifications: notification,
			runs: [],
			errors: [],
		})}${" ".repeat(2 * 1024 * 1024)}`;
		const fetcher = vi.fn(hold).mockResolvedValueOnce(new Response(body));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)).toHaveLength(1);
	});

	test("ignores late data after logout even if fetch ignores cancellation", async () => {
		let resolve!: (value: Response) => void;
		const fetcher = vi.fn(
			() =>
				new Promise<Response>((done) => {
					resolve = done;
				}),
		);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		first.close();
		resolve(response({ notifications: notification }));
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)).toHaveLength(0);
	});

	test("pauses offline and resumes when a live tab returns", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		first.activity(false);
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).not.toHaveBeenCalled();
		first.activity(true);
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	test("instructs a suspended tab to register again after its lease expires", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher, { portLeaseMs: 50, sweepMs: 20 });
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(61);
		expect(first.messages).toContainEqual({ type: "expired" });
		first.activity(true);
		expect(
			first.messages.filter((message) => message.type === "expired"),
		).toHaveLength(2);
		first.send({
			type: "connect",
			id: "first",
			session: first.session,
			activity: { online: true, visible: true },
		});
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher).toHaveBeenCalledTimes(2);
	});

	test("the default fetch is called without a receiver, as browsers require", async () => {
		const urls: string[] = [];
		vi.stubGlobal(
			"fetch",
			function (this: unknown, input: RequestInfo | URL): Promise<Response> {
				if (this !== undefined && this !== globalThis)
					throw new TypeError("Illegal invocation");
				urls.push(String(input));
				return Promise.resolve(
					String(input).endsWith("/connect")
						? httpGrant()
						: response({ notifications: notification }),
				);
			},
		);
		const runtime = new RealtimeRuntime();
		runtimes.push(runtime);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(urls).toContain("https://hub.test/api/v1/realtime/connect");
		expect(urls).toContain("https://hub.test/api/v1/realtime/poll");
		expect(snapshots(first)).toHaveLength(1);
		expect(first.messages).toContainEqual({
			type: "state",
			state: "connected",
		});
		expect(first.messages).not.toContainEqual({
			type: "state",
			state: "reconnecting",
		});
	});

	test.each([
		{ runs: [], interval: 30_000 },
		{ runs: [{ run_id: "a" }], interval: 2_000 },
	])(
		"polls on an interval when the API cannot hold a poll: %j",
		async ({ runs, interval }) => {
			vi.spyOn(Math, "random").mockReturnValue(0.5);
			const polls: { at: number; timeout: number }[] = [];
			const fetcher = vi.fn(
				async (_url: RequestInfo | URL, init?: RequestInit) => {
					polls.push({
						at: Date.now(),
						timeout: JSON.parse(String(init?.body)).timeout_ms,
					});
					return response();
				},
			);
			const runtime = create(fetcher, {}, () =>
				httpGrant({ max_poll_wait_ms: 0 }),
			);
			const first = connect(runtime, "first");
			first.subscribe({ notifications: true, runs });
			await vi.advanceTimersByTimeAsync(1);
			const settled = polls.length;
			expect(polls.at(-1)?.timeout).toBe(0);
			await vi.advanceTimersByTimeAsync(interval - 2);
			expect(polls).toHaveLength(settled);
			await vi.advanceTimersByTimeAsync(interval * 2);
			expect(polls).toHaveLength(settled + 2);
			const [previous, next] = polls.slice(-2);
			expect(next.at - previous.at).toBe(interval);
			expect(polls.slice(settled).every((poll) => poll.timeout === 0)).toBe(
				true,
			);
		},
	);

	test("long polls no longer than the API holds", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher, {}, () =>
			httpGrant({ max_poll_wait_ms: 5_000 }),
		);
		connect(runtime, "first").subscribe();
		await vi.advanceTimersByTimeAsync(1);
		expect(
			JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body)).timeout_ms,
		).toBe(5_000);
	});

	test("a long poll answered at once without changes paces like an interval", async () => {
		const fetcher = vi.fn(async () => response());
		const runtime = create(fetcher, {}, () =>
			httpGrant({ max_poll_wait_ms: undefined }),
		);
		connect(runtime, "first").subscribe({
			notifications: false,
			runs: [{ run_id: "a" }],
		});
		await vi.advanceTimersByTimeAsync(1_500);
		expect(fetcher).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(3_500);
		expect(fetcher.mock.calls.length).toBeLessThanOrEqual(3);
	});

	test("a poll the API holds for less than the assumed wait still re-polls at once", async () => {
		const fetcher = vi.fn(
			() =>
				new Promise<Response>((resolve) =>
					setTimeout(() => resolve(response()), 2_000),
				),
		);
		const runtime = create(fetcher, {}, () =>
			httpGrant({ max_poll_wait_ms: undefined }),
		);
		connect(runtime, "first").subscribe({
			notifications: false,
			runs: [{ run_id: "a" }],
		});
		await vi.advanceTimersByTimeAsync(2_100);
		expect(fetcher).toHaveBeenCalledTimes(2);
	});

	test("a late acknowledgement restarts a long poll that left its tab out", async () => {
		const fetcher = vi
			.fn(hold)
			.mockResolvedValueOnce(response({ runs: [run("a", [0])] }));
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		first.subscribe({ notifications: false, runs: [{ run_id: "a" }] });
		second.subscribe({ notifications: false, runs: [{ run_id: "b" }] });
		await vi.advanceTimersByTimeAsync(60);
		const runIds = (call: number) =>
			JSON.parse(String(fetcher.mock.calls[call][1]?.body)).runs.map(
				(item: { run_id: string }) => item.run_id,
			);
		expect(runIds(1)).toEqual(["b"]);
		first.accept();
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher.mock.calls[1][1]?.signal?.aborted).toBe(true);
		expect(runIds(2).sort()).toEqual(["a", "b"]);
		expect(
			JSON.parse(String(fetcher.mock.calls[2][1]?.body)).runs.find(
				(item: { run_id: string }) => item.run_id === "a",
			).after_sequence,
		).toBe(0);
	});

	test("a late acknowledgement gets the inbox snapshot another tab received meanwhile", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.5);
		let inbox = notification;
		const fetcher = vi.fn(
			async (_url: RequestInfo | URL, init?: RequestInit) =>
				JSON.parse(String(init?.body)).notifications?.revision ===
				inbox.revision
					? response()
					: response({ notifications: inbox }),
		);
		const runtime = create(fetcher, {}, () =>
			httpGrant({ max_poll_wait_ms: 0 }),
		);
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		first.subscribe();
		second.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		first.accept();
		inbox = { ...notification, revision: "n2" };
		await vi.advanceTimersByTimeAsync(30_000);
		expect(snapshots(first).at(-1)?.snapshot.notifications?.revision).toBe(
			"n2",
		);
		const calls = fetcher.mock.calls.length;
		second.accept();
		expect(snapshots(second).at(-1)?.snapshot.notifications?.revision).toBe(
			"n2",
		);
		expect(fetcher).toHaveBeenCalledTimes(calls);
	});

	test("a closed runtime tells its tabs instead of going silent", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		await vi.advanceTimersByTimeAsync(1);
		runtime.dispose();
		expect(first.messages.at(-1)).toEqual({ type: "state", state: "closed" });
		expect(fetcher.mock.calls[0][1]?.signal?.aborted).toBe(true);
	});

	test("a tab on another hub URL for the same account gets its own connection", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe();
		const other = "https://other.test/api/v1/realtime/poll";
		const port = { postMessage: vi.fn() };
		runtime.receive(port, {
			type: "connect",
			id: "other",
			session: { ...first.session, url: other },
			activity: { online: true, visible: true },
		});
		expect(port.postMessage).toHaveBeenCalledWith({ type: "ready" });
		runtime.receive(port, {
			type: "subscribe",
			id: "other",
			subscription: { notifications: true, runs: [] },
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(fetcher.mock.calls.map(([url]) => String(url))).toEqual([
			first.session.url,
			other,
		]);
		first.send({
			type: "session",
			id: "first",
			session: { ...first.session, url: other },
		});
		expect(
			first.messages.filter((message) => message.type === "ready"),
		).toHaveLength(2);
		expect(fetcher.mock.calls[0][1]?.signal?.aborted).toBe(true);
	});
});

test("malformed event order cannot become an acknowledged batch", () => {
	expect(() =>
		parseRealtimeResponse({
			protocol_version: 1,
			runs: [run("a", [2, 1])],
			errors: [],
		}),
	).toThrow();
});

const centry = (micros: number) =>
	({
		id: `e${micros}`,
		message_id: `m${micros}`,
		cursor: `${micros}:e${micros}`,
	}) as IInboxEntry;

describe("conversation subscriptions", () => {
	test("merges ports by the oldest cursor and routes only newer entries", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			response({
				conversations: [
					{
						conversation_id: "c",
						entries: [centry(3), centry(4), centry(5)],
						has_more: false,
					},
				],
			}),
		);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		first.subscribe({
			notifications: false,
			runs: [],
			conversations: [{ conversation_id: "c", after: "2:e2" }],
		});
		second.subscribe({
			notifications: false,
			runs: [],
			conversations: [{ conversation_id: "c", after: "4:e4" }],
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(JSON.parse(String(fetcher.mock.calls[0][1]?.body))).toMatchObject({
			conversations: [{ conversation_id: "c", after: "2:e2" }],
		});
		expect(
			snapshots(first)[0].snapshot.conversations?.[0].entries.map(
				(item) => item.cursor,
			),
		).toEqual(["3:e3", "4:e4", "5:e5"]);
		expect(
			snapshots(second)[0].snapshot.conversations?.[0].entries.map(
				(item) => item.cursor,
			),
		).toEqual(["5:e5"]);
		first.accept();
		second.accept();
		await vi.advanceTimersByTimeAsync(60);
		expect(
			JSON.parse(String(fetcher.mock.calls[1][1]?.body)).conversations,
		).toEqual([{ conversation_id: "c", after: "5:e5" }]);
	});

	test("a port holding no entries makes the merged request cursorless", async () => {
		const fetcher = vi.fn(hold);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		const second = connect(runtime, "second");
		first.subscribe({
			notifications: false,
			runs: [],
			conversations: [{ conversation_id: "c", as: "bot", after: "2:e2" }],
		});
		second.subscribe({
			notifications: false,
			runs: [],
			conversations: [{ conversation_id: "c", as: "bot" }],
		});
		await vi.advanceTimersByTimeAsync(1);
		const body = JSON.parse(String(fetcher.mock.calls.at(-1)?.[1]?.body));
		expect(body.conversations).toEqual([{ conversation_id: "c", as: "bot" }]);
	});

	test("drops a conversation that is not found and keeps an unavailable one", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			response({
				conversation_errors: [
					{ conversation_id: "gone", code: "not_found" },
					{ conversation_id: "later", code: "unavailable" },
				],
			}),
		);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe({
			notifications: true,
			runs: [],
			conversations: [
				{ conversation_id: "gone" },
				{ conversation_id: "later" },
			],
		});
		await vi.advanceTimersByTimeAsync(1);
		expect(snapshots(first)[0].snapshot.conversation_errors).toHaveLength(2);
		first.accept();
		await vi.advanceTimersByTimeAsync(60);
		expect(
			JSON.parse(String(fetcher.mock.calls[1][1]?.body)).conversations,
		).toBeUndefined();
	});

	test("polls again soon while a conversation has more", async () => {
		const fetcher = vi.fn(hold).mockResolvedValueOnce(
			response({
				conversations: [
					{ conversation_id: "c", entries: [centry(1)], has_more: true },
				],
			}),
		);
		const runtime = create(fetcher);
		const first = connect(runtime, "first");
		first.subscribe({
			notifications: false,
			runs: [],
			conversations: [{ conversation_id: "c" }],
		});
		await vi.advanceTimersByTimeAsync(1);
		first.accept();
		await vi.advanceTimersByTimeAsync(60);
		expect(fetcher).toHaveBeenCalledTimes(2);
		expect(
			JSON.parse(String(fetcher.mock.calls[1][1]?.body)).conversations,
		).toEqual([{ conversation_id: "c", after: "1:e1" }]);
	});

	test("rejects malformed conversation sections", () => {
		const base = { protocol_version: 1, runs: [], errors: [] };
		const bad = [
			{ conversation_id: "c", entries: [{ cursor: "1:a" }], has_more: false },
			{
				conversation_id: "c",
				entries: [centry(2), centry(1)],
				has_more: false,
			},
			{ conversation_id: "c", entries: [], has_more: "no" },
		];
		for (const update of bad)
			expect(() =>
				parseRealtimeResponse({ ...base, conversations: [update] }),
			).toThrow();
		expect(() =>
			parseRealtimeResponse({
				...base,
				conversation_errors: [{ conversation_id: "c" }],
			}),
		).toThrow();
		expect(
			parseRealtimeResponse({
				...base,
				conversations: [
					{ conversation_id: "c", entries: [centry(1)], has_more: false },
				],
			}).conversations,
		).toHaveLength(1);
	});
});
