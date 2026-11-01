import { afterEach, describe, expect, test, vi } from "vitest";
import { RealtimeRuntime } from "../realtime/runtime";
import { getRealtimeClientForRuntime } from "../realtime/runtime-client";
import type {
	RealtimeClient,
	RealtimeConnectionSession,
	RealtimeListener,
	RealtimePollRequest,
	RealtimePollResponse,
	RealtimeRunSnapshot,
} from "../realtime/types";
import type { IChannelHandle } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import type { ExecutionRealtimeClientFactory } from "./realtime";
import { ExecutionRuntime } from "./runtime";
import { ExecutionRuntimeClient } from "./runtime-client";
import {
	type ExecutionAuthorization,
	type ExecutionCommand,
	ExecutionDetachedError,
	type ExecutionMessage,
	type ExecutionMetadata,
	type ExecutionTask,
	type PreparedExecutionRequest,
} from "./types";

const clients: ExecutionRuntimeClient[] = [];
afterEach(() => {
	for (const client of clients.splice(0)) client.close();
	vi.restoreAllMocks();
});

function connect(runtime: ExecutionRuntime, scope = "account-a") {
	const port = {
		onmessage: null as ((event: MessageEvent<ExecutionMessage>) => void) | null,
		postMessage: (command: ExecutionCommand | ExecutionAuthorization) => {
			queueMicrotask(() => {
				void runtime.receive(worker, structuredClone(command));
			});
		},
	};
	const worker = {
		postMessage: (message: ExecutionMessage) => {
			const cloned = structuredClone(message);
			queueMicrotask(() =>
				port.onmessage?.({ data: cloned } as MessageEvent<ExecutionMessage>),
			);
		},
	};
	const client = new ExecutionRuntimeClient(scope, port, "in-tab");
	clients.push(client);
	return client;
}

function metadata(taskId = "task", scope = "account-a"): ExecutionMetadata {
	return {
		taskId,
		streamId: `session-${taskId}`,
		scope,
		appId: "app",
		eventId: "event",
		title: "Review orders",
		startedAt: new Date().toISOString(),
	};
}

function event(
	id: string,
	type: string,
	payload: unknown = {},
): IIntercomEvent {
	return {
		event_id: id,
		event_type: type,
		payload,
		timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
}

function stream() {
	let controller!: ReadableStreamDefaultController<Uint8Array>;
	const body = new ReadableStream<Uint8Array>({
		start(value) {
			controller = value;
		},
	});
	const encoder = new TextEncoder();
	return {
		response: new Response(body),
		emit: (...events: IIntercomEvent[]) =>
			controller.enqueue(
				encoder.encode(
					events
						.map((item) => `data: ${JSON.stringify(item)}\r\n\r\n`)
						.join(""),
				),
			),
		raw: (data: string) => controller.enqueue(encoder.encode(data)),
		end: () => controller.close(),
		fail: (error: unknown) => controller.error(error),
	};
}

const handle: IChannelHandle = {
	channel_id: "channel",
	request_id: "approval",
	expires_at: 4_000_000_000,
	transport: {
		type: "http",
		push_url: "https://example.test/push",
		token: "capability",
	},
} as IChannelHandle;

async function until(check: () => boolean) {
	for (let attempt = 0; attempt < 100 && !check(); attempt++)
		await new Promise((resolve) => setTimeout(resolve, 1));
	expect(check()).toBe(true);
}

function queued(
	overrides: Partial<PreparedExecutionRequest> = {},
): PreparedExecutionRequest {
	return {
		url: "https://hub.test/invoke",
		body: {},
		cancelUrl: "https://hub.test/runs/{runId}",
		realtime: {
			scope: "account-a",
			url: "https://hub.test/api/v1/realtime/poll",
			headers: { Authorization: "Bearer account" },
		},
		...overrides,
	};
}

/** A replay bus whose updates the test delivers. */
function fakeBus() {
	let listener: RealtimeListener | undefined;
	const factory: ExecutionRealtimeClientFactory = async () => ({
		setSession: vi.fn(),
		setActivity: vi.fn(),
		setSubscriptions: vi.fn(),
		close: vi.fn(),
		subscribe: (callback) => {
			listener = callback;
			return () => {};
		},
	});
	return {
		factory,
		emit: async (update: Parameters<RealtimeListener>[0]) => {
			await until(() => listener !== undefined);
			await listener?.(update);
		},
	};
}

function replayed(run: Partial<RealtimeRunSnapshot> = {}) {
	return {
		type: "snapshot" as const,
		snapshot: {
			protocol_version: 1 as const,
			errors: [],
			runs: [
				{
					run_id: "queued",
					revision: "r1",
					status: "COMPLETED",
					progress: 100,
					events: [],
					after_sequence: -1,
					has_more: false,
					...run,
				},
			],
		},
	};
}

function queuedRuntime(
	bus: ReturnType<typeof fakeBus>,
	deletes: string[] = [],
) {
	return new ExecutionRuntime({
		fetch: async (url, init) => {
			if (init?.method === "DELETE") {
				deletes.push(String(url));
				return new Response(null, { status: 204 });
			}
			return Response.json({ run_id: "queued", poll_token: "capability" });
		},
		push: async () => {},
		realtime: bus.factory,
	});
}

describe("execution runtime", () => {
	test("queued replay uses renewed dispatch headers and reports the bus's exact rejected bearer", async () => {
		let session: RealtimeConnectionSession | undefined;
		let listener: RealtimeListener | undefined;
		const close = vi.fn();
		const busClient: RealtimeClient = {
			close,
			setActivity: vi.fn(),
			setSession: vi.fn(),
			subscribe: (callback) => {
				listener = callback;
				return () => {};
			},
			setSubscriptions: () => {
				void listener?.({
					type: "snapshot",
					snapshot: {
						protocol_version: 1,
						errors: [],
						runs: [
							{
								run_id: "queued",
								revision: "r1",
								status: "completed",
								progress: 100,
								after_sequence: -1,
								has_more: false,
								events: [],
							},
						],
					},
				});
			},
		};
		const fetcher = vi
			.fn(async () =>
				Response.json({ run_id: "queued", poll_token: "capability" }),
			)
			.mockResolvedValueOnce(new Response(null, { status: 401 }));
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push: async () => {},
			realtime: async (incoming, options) => {
				session = incoming;
				await options?.onUnauthorized?.("bus-rejected");
				return busClient;
			},
		});
		const client = connect(runtime);
		const renew = vi
			.fn(async () => ({ authorization: "Bearer latest" }))
			.mockResolvedValueOnce({ authorization: "Bearer fresh" });
		await client.execute(
			metadata(),
			{
				url: "https://hub.test/invoke",
				body: {},
				headers: { authorization: "Bearer old" },
				realtime: {
					scope: "account-a",
					url: "https://hub.test/api/v1/realtime/poll",
					headers: { authorization: "Bearer old" },
				},
			},
			{ onUnauthorized: renew },
		);
		expect(session?.headers).toEqual({ authorization: "Bearer fresh" });
		expect(renew.mock.calls).toEqual([["old"], ["bus-rejected"]]);
		expect(close).toHaveBeenCalledOnce();
	});

	test("queued receipts replay through the shared bus and keep one interaction claim across tabs", async () => {
		const requests: RealtimePollRequest[] = [];
		let finish!: (response: Response) => void;
		const final = new Promise<Response>((resolve) => {
			finish = resolve;
		});
		const response = (
			after: number,
			hasMore: boolean,
			events: RealtimePollResponse["runs"][number]["events"],
		): Response =>
			Response.json({
				protocol_version: 1,
				errors: [],
				runs: [
					{
						run_id: "queued",
						revision: `r${after}`,
						status: "completed",
						progress: 100,
						after_sequence: after,
						has_more: hasMore,
						events,
					},
				],
			});
		const bus = new RealtimeRuntime({
			fetch: async (url, init) => {
				if (String(url).endsWith("/connect"))
					return Response.json({
						protocol_version: 1,
						expires_at: Math.floor(Date.now() / 1000) + 900,
						expires_in: 900,
						max_poll_wait_ms: 20_000,
						transport: { type: "http" },
					});
				requests.push(JSON.parse(String(init?.body)));
				if (requests.length === 1)
					return response(0, true, [
						{
							sequence: 0,
							event_id: "question",
							event_type: "a2ui",
							payload: {
								type: "requestElements",
								request_id: "approval",
								channel: handle,
							},
							created_at: "2026-11-01T00:00:00Z",
						},
					]);
				return final;
			},
		});
		const push = vi.fn(async () => {});
		const runtime = new ExecutionRuntime({
			fetch: async () =>
				Response.json({
					run_id: "queued",
					status: "pending",
					poll_token: "private-poll-token",
				}),
			push,
			realtime: (session, options) =>
				getRealtimeClientForRuntime(bus, session, options),
		});
		const first = connect(runtime);
		const second = connect(runtime);
		try {
			const running = first.execute(metadata(), {
				url: "https://hub.test/invoke",
				body: {},
				realtime: {
					scope: "account-a",
					url: "https://hub.test/api/v1/realtime/poll",
					headers: { Authorization: "Bearer account" },
					transport: "http",
				},
			});
			await until(() => second.getTask("task")?.status === "waiting");
			expect(await first.claimRequest(handle)).toBe(true);
			expect(await second.claimRequest(handle)).toBe(false);
			await Promise.all([
				first.submitReply(handle, { ok: true }),
				second.submitReply(handle, { ok: true }),
			]);
			expect(push).toHaveBeenCalledTimes(1);
			expect(first.getTask("task")?.status).toBe("running");
			await until(() => requests.length === 2);
			expect(requests[1].runs[0]).toMatchObject({
				after_sequence: 0,
				poll_token: "private-poll-token",
			});
			finish(
				response(1, false, [
					{
						sequence: 1,
						event_id: "done",
						event_type: "completed",
						payload: { status: "completed" },
						created_at: "2026-11-01T00:00:01Z",
					},
				]),
			);
			await running;
			expect(
				first.getTask("task")?.events.map((event) => event.event_id),
			).toEqual(["queued:receipt", "question", "done"]);
			expect(JSON.stringify(first.getTask("task"))).not.toContain(
				"private-poll-token",
			);
			expect(first.getTask("task")?.status).toBe("completed");
		} finally {
			bus.dispose();
		}
	});

	test("healthy and interrupted SSE streams do not open a replay subscription", async () => {
		const realtime = vi.fn();
		for (const complete of [true, false]) {
			const source = stream();
			const runtime = new ExecutionRuntime({
				fetch: async () => source.response,
				push: async () => {},
				realtime,
			});
			const client = connect(runtime);
			const result = client.execute(metadata(), {
				url: "https://hub.test/invoke",
				body: {},
				realtime: {
					scope: "account-a",
					url: "https://hub.test/api/v1/realtime/poll",
					headers: {},
				},
			});
			const settled = result.catch((error) => error);
			source.emit(event("run", "run_initiated", { run_id: "streamed" }));
			if (complete)
				source.emit(event("done", "completed", { status: "completed" }));
			source.end();
			await settled;
			expect(client.getTask("task")?.status).toBe(
				complete ? "completed" : "failed",
			);
		}
		expect(realtime).not.toHaveBeenCalled();
	});

	test("replays ordered events to another tab and delivers one reply after the initiating tab closes", async () => {
		const source = stream();
		const push = vi.fn(async () => {});
		const fetcher = vi.fn(async () => source.response);
		const runtime = new ExecutionRuntime({ fetch: fetcher, push });
		const first = connect(runtime);
		await first.ready;
		const running = first.execute(metadata(), {
			url: "https://example.test/invoke",
			body: {},
		});
		const abandoned = running.catch(() => undefined);
		await until(() => fetcher.mock.calls.length === 1);
		source.emit(
			event("run", "run_initiated", { run_id: "remote-run" }),
			event("partial", "chat_stream_partial", {
				chunk: { choices: [{ delta: { content: "Hello " } }] },
			}),
			event("request", "interaction_request", {
				id: "approval",
				channel: handle,
			}),
		);
		await until(() => first.getTasks()[0]?.status === "waiting");
		const second = connect(runtime);
		await second.ready;
		expect(second.getTasks()[0].events.map((item) => item.event_id)).toEqual([
			"run",
			"partial",
			"request",
		]);
		first.close();
		await abandoned;
		await Promise.all([
			second.submitReply(handle, "approved"),
			second.submitReply(handle, "approved"),
		]);
		expect(push).toHaveBeenCalledTimes(1);
		expect(second.getTasks()[0].pendingInteractions[0]).toMatchObject({
			status: "submitted",
			response_value: "approved",
		});
		source.emit(
			event("final", "completed", { status: "completed" }),
			event("tail", "chat_out", {
				response: { choices: [{ message: { content: "Finished" } }] },
			}),
		);
		await until(() => second.getTasks()[0]?.status === "completed");
		expect(second.getTasks()[0].events.map((item) => item.event_id)).toEqual([
			"run",
			"partial",
			"request",
			"final",
			"tail",
		]);
		expect(second.getTasks()[0].preview).toBe("Finished");
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	test("isolates account scopes and keeps device claims after their tab disconnects", async () => {
		const source = stream();
		const runtime = new ExecutionRuntime({
			fetch: async () => source.response,
			push: async () => {},
		});
		const first = connect(runtime);
		const other = connect(runtime, "account-b");
		await Promise.all([first.ready, other.ready]);
		const running = first
			.execute(metadata(), { url: "https://example.test/invoke", body: {} })
			.catch(() => undefined);
		source.emit(
			event("request", "a2ui", { type: "deviceCommand", channel: handle }),
		);
		await until(() => first.getTasks()[0]?.status === "waiting");
		expect(other.getTasks()).toEqual([]);
		await expect(other.submitReply(handle, "approved")).rejects.toThrow(
			"current execution session",
		);
		await expect(other.cancel("task")).rejects.toThrow("no longer available");
		expect(await first.claimRequest(handle)).toBe(true);
		expect(await first.claimRequest(handle)).toBe(false);
		first.close();
		const replacement = connect(runtime);
		await replacement.ready;
		expect(await replacement.claimRequest(handle)).toBe(false);
		await expect(first.submitReply(handle, null)).rejects.toThrow(
			"disconnected",
		);
		source.emit(event("done", "completed", { status: "completed" }));
		await until(() => replacement.getTasks()[0]?.status === "completed");
		await running;
	});

	test("a partial clean EOF fails and persists the failure before publishing completion", async () => {
		const source = stream();
		let finishSave!: () => void;
		const saving = new Promise<void>((resolve) => {
			finishSave = resolve;
		});
		const saved: ExecutionTask[] = [];
		const runtime = new ExecutionRuntime({
			fetch: async () => source.response,
			push: async () => {},
			persist: async (task, final) => {
				saved.push(task);
				if (final) await saving;
			},
		});
		const client = connect(runtime);
		await client.ready;
		const running = client.execute(
			{ ...metadata(), chat: {} as ExecutionMetadata["chat"] },
			{ url: "https://example.test/invoke", body: {} },
		);
		const result = running.catch((error) => error);
		source.emit(event("partial", "chat_out", { response: "Partial reply" }));
		source.end();
		await until(() => saved.some((task) => task.status === "failed"));
		expect(client.getTasks()[0].status).toBe("running");
		finishSave();
		expect((await result).message).toContain("before the run completed");
		expect(client.getTasks()[0]).toMatchObject({
			status: "failed",
			events: [expect.objectContaining({ event_id: "partial" })],
		});
	});

	test("cancel sends the backend DELETE and channel signal, then settles once", async () => {
		const source = stream();
		const push = vi.fn(async () => {});
		const fetcher = vi.fn(
			async (_url: string | URL | Request, init?: RequestInit) => {
				if (init?.method === "DELETE")
					return new Response(null, { status: 204 });
				init?.signal?.addEventListener("abort", () =>
					source.fail(init.signal?.reason),
				);
				return source.response;
			},
		);
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push,
		});
		const client = connect(runtime);
		await client.ready;
		const result = client
			.execute(metadata(), {
				url: "https://example.test/invoke",
				body: {},
				cancelUrl: "https://example.test/runs/{runId}",
			})
			.catch((error) => error);
		source.emit(
			event("run", "run_initiated", { run_id: "remote-run", channel: handle }),
		);
		await until(() => client.getTasks()[0]?.runId === "remote-run");
		await client.cancel("task");
		expect((await result).name).toBe("AbortError");
		expect(fetcher).toHaveBeenCalledWith(
			"https://example.test/runs/remote-run",
			expect.objectContaining({ method: "DELETE" }),
		);
		expect(push).toHaveBeenCalledWith(handle, { kind: "cancel", value: null });
		expect(client.getTasks()[0].status).toBe("cancelled");
	});

	test("dispatch timeout settles even if fetch ignores abort", async () => {
		const runtime = new ExecutionRuntime({
			fetch: () => new Promise(() => {}),
			push: async () => {},
		});
		const client = connect(runtime);
		await client.ready;
		await expect(
			client.execute(metadata(), {
				url: "https://example.test/invoke",
				body: {},
				timeoutMs: 5,
			}),
		).rejects.toMatchObject({ name: "RequestTimeoutError", timeoutMs: 5 });
		expect(client.getTasks()[0].status).toBe("failed");
	});

	test("failed cancellation keeps the stream alive and allows another stop attempt", async () => {
		const source = stream();
		let attempts = 0;
		const fetcher = vi.fn(
			async (_url: string | URL | Request, init?: RequestInit) => {
				if (init?.method === "DELETE")
					return new Response(null, { status: ++attempts === 1 ? 503 : 204 });
				init?.signal?.addEventListener("abort", () =>
					source.fail(init.signal?.reason),
				);
				return source.response;
			},
		);
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push: async () => {},
		});
		const client = connect(runtime);
		await client.ready;
		const result = client
			.execute(metadata(), {
				url: "https://example.test/invoke",
				body: {},
				cancelUrl: "https://example.test/runs/{runId}",
			})
			.catch((error) => error);
		source.emit(event("run", "run_initiated", { run_id: "remote-run" }));
		await until(() => client.getTasks()[0]?.runId === "remote-run");
		await expect(client.cancel("task")).rejects.toMatchObject({ status: 503 });
		expect(client.getTasks()[0]).toMatchObject({
			status: "running",
			cancellationRequested: false,
		});
		source.emit(
			event("still-running", "chat_out", { response: "Still running" }),
		);
		await until(() => client.getTasks()[0]?.events.length === 2);
		await client.cancel("task");
		expect((await result).name).toBe("AbortError");
		expect(attempts).toBe(2);
		expect(client.getTasks()[0].status).toBe("cancelled");
	});

	test("accepted cloud cancellation stays stopping until its terminal event", async () => {
		const source = stream();
		const fetcher = async (_url: string | URL | Request, init?: RequestInit) =>
			init?.method === "DELETE"
				? Response.json({ status: "cancellation_requested", cancelled: false })
				: source.response;
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push: async () => {},
		});
		const client = connect(runtime);
		await client.ready;
		const result = client
			.execute(metadata(), {
				url: "https://example.test/invoke",
				body: {},
				cancelUrl: "https://example.test/runs/{runId}",
			})
			.catch((error) => error);
		source.emit(event("run", "run_initiated", { run_id: "remote-run" }));
		await until(() => client.getTasks()[0]?.runId === "remote-run");
		await client.cancel("task");
		expect(client.getTasks()[0]).toMatchObject({
			status: "running",
			cancellationRequested: true,
		});
		source.emit(
			event("cleanup", "chat_out", { response: "Saving before stopping" }),
		);
		await until(() => client.getTasks()[0]?.events.length === 2);
		expect(client.getTasks()[0].status).toBe("running");
		source.emit(event("done", "completed", { status: "cancelled" }));
		expect((await result).name).toBe("AbortError");
		expect(client.getTasks()[0].status).toBe("cancelled");
	});

	test("cancel before run admission waits for the run id without breaking replay order", async () => {
		const source = stream();
		const fetcher = vi.fn(
			async (_url: string | URL | Request, init?: RequestInit) => {
				if (init?.method === "DELETE")
					return new Response(null, { status: 204 });
				init?.signal?.addEventListener("abort", () =>
					source.fail(init.signal?.reason),
				);
				return source.response;
			},
		);
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push: async () => {},
		});
		const first = connect(runtime);
		const second = connect(runtime);
		await Promise.all([first.ready, second.ready]);
		const result = first
			.execute(metadata(), {
				url: "https://example.test/invoke",
				body: {},
				cancelUrl: "https://example.test/runs/{runId}",
			})
			.catch((error) => error);
		await until(() => second.getTasks().length === 1);
		await second.cancel("task");
		source.emit(event("run", "run_initiated", { run_id: "remote-run" }));
		expect((await result).name).toBe("AbortError");
		await until(() => second.getTasks()[0]?.status === "cancelled");
		expect(second.getTasks()[0].events.map((item) => item.event_id)).toEqual([
			"run",
		]);
		await second.dismiss("task");
		expect(second.getTasks()).toEqual([]);
	});

	test("retries only an unauthorized dispatch with renewed headers", async () => {
		const source = stream();
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce(new Response("expired", { status: 401 }))
			.mockResolvedValueOnce(source.response);
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push: async () => {},
		});
		const client = connect(runtime);
		await client.ready;
		const renew = vi.fn(async () => ({ Authorization: "Bearer renewed" }));
		const result = client.execute(
			metadata(),
			{
				url: "https://example.test/invoke",
				body: {},
				headers: { Authorization: "Bearer expired" },
			},
			{ onUnauthorized: renew },
		);
		await until(() => fetcher.mock.calls.length === 2);
		source.emit(event("done", "completed", { status: "completed" }));
		await result;
		expect(renew).toHaveBeenCalledWith("expired");
		expect(fetcher.mock.calls[1][1].headers.Authorization).toBe(
			"Bearer renewed",
		);
	});

	test("expires feedback without accepting a late reply", async () => {
		const source = stream();
		const push = vi.fn(async () => {});
		const runtime = new ExecutionRuntime({
			fetch: async () => source.response,
			push,
		});
		const client = connect(runtime);
		await client.ready;
		const result = client.execute(metadata(), {
			url: "https://example.test/invoke",
			body: {},
		});
		const expired = { ...handle, expires_at: Date.now() / 1000 + 0.02 };
		source.emit(
			event("request", "interaction_request", {
				id: "approval",
				channel: expired,
			}),
		);
		await until(
			() => client.getTasks()[0]?.pendingInteractions[0]?.status === "expired",
		);
		await expect(client.submitReply(expired, "late")).rejects.toThrow(
			"expired",
		);
		expect(push).not.toHaveBeenCalled();
		source.emit(event("done", "completed", { status: "completed" }));
		await result;
	});

	test("renewal network failures preserve their cause instead of becoming sign-out responses", async () => {
		const runtime = new ExecutionRuntime({
			fetch: async () => new Response("expired", { status: 401 }),
			push: async () => {},
		});
		const client = connect(runtime);
		await client.ready;
		await expect(
			client.execute(
				metadata(),
				{ url: "https://example.test/invoke", body: {} },
				{
					onUnauthorized: async () => {
						throw new TypeError("Identity provider is offline");
					},
				},
			),
		).rejects.toBeInstanceOf(TypeError);
		expect(client.getTasks()[0].error).toBe("Identity provider is offline");
	});

	test("de-duplicates execution requests and rejects a second run on the same stream", async () => {
		const source = stream();
		const fetcher = vi.fn(async () => source.response);
		const runtime = new ExecutionRuntime({
			fetch: fetcher,
			push: async () => {},
		});
		const first = connect(runtime);
		const second = connect(runtime);
		await Promise.all([first.ready, second.ready]);
		const args = metadata();
		const a = first.execute(args, {
			url: "https://example.test/invoke",
			body: {},
		});
		const b = second.execute(args, {
			url: "https://example.test/invoke",
			body: {},
		});
		await until(() => fetcher.mock.calls.length === 1);
		await expect(
			second.execute(
				{ ...args, taskId: "duplicate" },
				{ url: "https://example.test/invoke", body: {} },
			),
		).rejects.toThrow("already active");
		source.emit(event("done", "completed", { status: "completed" }));
		await Promise.all([a, b]);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	test("an explicit successful outcome permits handled errors, while an unclassified error fails", async () => {
		for (const successful of [true, false]) {
			const source = stream();
			const runtime = new ExecutionRuntime({
				fetch: async () => source.response,
				push: async () => {},
			});
			const client = connect(runtime);
			await client.ready;
			const result = client
				.execute(metadata(), { url: "https://example.test/invoke", body: {} })
				.then(
					() => "ok",
					(error: Error) => error.message,
				);
			source.emit(
				event("error", "error", { message: "Node failed" }),
				event("done", "completed", successful ? { status: "completed" } : {}),
			);
			expect(await result).toBe(successful ? "ok" : "Node failed");
			expect(client.getTasks()[0].status).toBe(
				successful ? "completed" : "failed",
			);
		}
	});

	test("a queued run keeps waiting through retrying replay states and unavailable replays", async () => {
		const bus = fakeBus();
		const client = connect(queuedRuntime(bus));
		let settled = false;
		const result = client.execute(metadata(), queued()).finally(() => {
			settled = true;
		});
		await bus.emit({ type: "state", state: "unsupported" });
		await bus.emit({ type: "state", state: "unauthorized" });
		await bus.emit({
			type: "snapshot",
			snapshot: {
				protocol_version: 1,
				runs: [],
				errors: [{ run_id: "queued", code: "unavailable" }],
			},
		});
		await new Promise((resolve) => setTimeout(resolve, 5));
		expect(settled).toBe(false);
		expect(client.getTask("task")?.status).toBe("running");
		await bus.emit(replayed());
		await result;
		expect(client.getTask("task")?.status).toBe("completed");
	});

	test("a queued run the API cannot find fails", async () => {
		const bus = fakeBus();
		const client = connect(queuedRuntime(bus));
		const result = client.execute(metadata(), queued()).catch((error) => error);
		await bus.emit({
			type: "snapshot",
			snapshot: {
				protocol_version: 1,
				runs: [],
				errors: [{ run_id: "queued", code: "not_found" }],
			},
		});
		expect((await result).message).toContain("not_found");
		expect(client.getTask("task")?.status).toBe("failed");
	});

	test.each([
		[{ error: "Executor crashed" }, "Executor crashed"],
		[
			{
				events: [
					{
						sequence: 0,
						event_id: "failure",
						event_type: "error",
						payload: { message: "Node 'Fetch' failed" },
						created_at: "2026-11-01T00:00:00Z",
					},
				],
				after_sequence: 0,
			},
			"Node 'Fetch' failed",
		],
		[{ status: "TIMEOUT" }, "The workflow timed out."],
	])("a failed queued run reports why it failed: %j", async (run, message) => {
		const bus = fakeBus();
		const client = connect(queuedRuntime(bus));
		const result = client.execute(metadata(), queued()).catch((error) => error);
		await bus.emit(replayed({ status: "FAILED", ...run }));
		expect((await result).message).toBe(message);
		expect(client.getTask("task")).toMatchObject({
			status: "failed",
			error: message,
		});
	});

	test("following a queued run is not bounded by the dispatch deadline", async () => {
		const bus = fakeBus();
		const client = connect(queuedRuntime(bus));
		const result = client.execute(metadata(), queued({ timeoutMs: 5 }));
		await new Promise((resolve) => setTimeout(resolve, 30));
		expect(client.getTask("task")?.status).toBe("running");
		await bus.emit(replayed());
		await result;
		expect(client.getTask("task")?.status).toBe("completed");
	});

	test("stopping a followed queued run sends the DELETE and ends it as cancelled", async () => {
		const bus = fakeBus();
		const deletes: string[] = [];
		const client = connect(queuedRuntime(bus, deletes));
		const result = client.execute(metadata(), queued()).catch((error) => error);
		await until(() => client.getTask("task")?.runId === "queued");
		await bus.emit({ type: "state", state: "reconnecting" });
		await client.cancel("task");
		expect((await result).name).toBe("AbortError");
		expect(deletes).toEqual(["https://hub.test/runs/queued"]);
		expect(client.getTask("task")?.status).toBe("cancelled");
	});

	test("a closed replay detaches the run without failing it, and it can still be stopped", async () => {
		const bus = fakeBus();
		const deletes: string[] = [];
		const client = connect(queuedRuntime(bus, deletes));
		const result = client.execute(metadata(), queued()).catch((error) => error);
		await bus.emit({ type: "state", state: "closed" });
		const error = await result;
		expect(error).toBeInstanceOf(ExecutionDetachedError);
		expect(error.runId).toBe("queued");
		await until(() => client.getTask("task")?.detached === true);
		expect(client.getTask("task")).toMatchObject({ status: "running" });
		expect(client.getTask("task")?.error).toBeUndefined();
		await client.cancel("task");
		expect(deletes).toEqual(["https://hub.test/runs/queued"]);
		await until(() => client.getTask("task")?.status === "cancelled");
	});

	test("a closed account bus detaches the runs it was replaying", async () => {
		let polls = 0;
		const bus = new RealtimeRuntime({
			fetch: async (url, init) => {
				if (String(url).endsWith("/connect"))
					return Response.json({
						protocol_version: 1,
						expires_at: Math.floor(Date.now() / 1000) + 900,
						expires_in: 900,
						max_poll_wait_ms: 20_000,
						transport: { type: "http" },
					});
				polls++;
				return new Promise<Response>((_, reject) =>
					init?.signal?.addEventListener(
						"abort",
						() => reject(new Error("aborted")),
						{ once: true },
					),
				);
			},
		});
		const runtime = new ExecutionRuntime({
			fetch: async () => Response.json({ run_id: "queued" }),
			push: async () => {},
			realtime: (session, options) =>
				getRealtimeClientForRuntime(bus, session, options),
		});
		const client = connect(runtime);
		const result = client.execute(metadata(), queued()).catch((error) => error);
		await until(() => polls > 0);
		bus.dispose();
		const error = await result;
		expect(error).toBeInstanceOf(ExecutionDetachedError);
		expect(error.runId).toBe("queued");
		await until(() => client.getTask("task")?.detached === true);
	});

	test("closing the last tab detaches a followed run that a later tab can stop", async () => {
		const bus = fakeBus();
		const deletes: string[] = [];
		const runtime = queuedRuntime(bus, deletes);
		const first = connect(runtime);
		const result = first.execute(metadata(), queued()).catch((error) => error);
		await until(() => first.getTask("task")?.runId === "queued");
		await bus.emit({ type: "state", state: "connected" });
		first.close();
		const error = await result;
		expect(error).toBeInstanceOf(ExecutionDetachedError);
		expect(error.runId).toBe("queued");
		const second = connect(runtime);
		await second.ready;
		await until(() => second.getTask("task")?.detached === true);
		expect(second.getTask("task")?.status).toBe("running");
		await second.cancel("task");
		expect(deletes).toEqual(["https://hub.test/runs/queued"]);
		await until(() => second.getTask("task")?.status === "cancelled");
	});

	test("a finished run keeps its outcome for later tabs but releases its events once a tab received it", async () => {
		const source = stream();
		const runtime = new ExecutionRuntime({
			fetch: async () => source.response,
			push: async () => {},
		});
		const first = connect(runtime);
		await first.ready;
		const running = first.execute(metadata(), {
			url: "https://example.test/invoke",
			body: {},
		});
		source.emit(
			event("out", "chat_out", { response: "Done" }),
			event("done", "completed", { status: "completed" }),
		);
		await running;
		expect(first.getTask("task")?.events).toHaveLength(2);
		const late = connect(runtime);
		await late.ready;
		expect(late.getTask("task")).toMatchObject({
			status: "completed",
			preview: "Done",
			events: [],
		});
	});

	test("a run that finished while no tab was connected reaches the next tab in full", async () => {
		const sources = [stream(), stream()];
		const finals: string[] = [];
		let dispatched = 0;
		const runtime = new ExecutionRuntime({
			fetch: async () => sources[dispatched++].response,
			push: async () => {},
			persist: async (task, final) => {
				if (final) finals.push(task.taskId);
			},
		});
		const first = connect(runtime);
		await first.ready;
		const chat = {} as ExecutionMetadata["chat"];
		for (const id of ["a", "b"])
			void first
				.execute(
					{ ...metadata(id), chat },
					{ url: "https://example.test/invoke", body: {} },
				)
				.catch(() => undefined);
		await until(() => dispatched === 2);
		first.close();
		const large = "x".repeat(5 * 1024 * 1024);
		for (const [index, id] of ["a", "b"].entries()) {
			sources[index].emit(
				event(`${id}-out`, "chat_out", { response: large }),
				event(`${id}-done`, "completed", { status: "completed" }),
			);
			await until(() => finals.includes(id));
		}
		const late = connect(runtime);
		await late.ready;
		expect(late.getTask("a")).toMatchObject({
			status: "completed",
			events: [],
		});
		expect(late.getTask("b")?.events.map((item) => item.event_id)).toEqual([
			"b-out",
			"b-done",
		]);
		const later = connect(runtime);
		await later.ready;
		expect(later.getTask("b")?.events).toEqual([]);
	});
});
