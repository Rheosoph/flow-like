import { afterEach, describe, expect, test, vi } from "vitest";
import type { IChannelHandle } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import { ExecutionRuntime } from "./runtime";
import { ExecutionRuntimeClient } from "./runtime-client";
import type {
	ExecutionAuthorization,
	ExecutionCommand,
	ExecutionMessage,
	ExecutionMetadata,
	ExecutionTask,
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

describe("execution runtime", () => {
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
			fetch: fetcher as typeof fetch,
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
			fetch: fetcher as typeof fetch,
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
});
