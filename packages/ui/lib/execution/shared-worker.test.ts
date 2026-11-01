import { afterEach, expect, test, vi } from "vitest";
import type { RealtimeClient, RealtimePollRequest } from "../realtime/types";
import type { ExecutionRuntimeClient } from "./runtime-client";

vi.mock("./chat-persistence", () => ({ persistExecutionChat: async () => {} }));
vi.mock("../channel/transport", () => ({ rawPushChannel: async () => {} }));

let inbox: RealtimeClient | undefined;
let execution: ExecutionRuntimeClient | undefined;
afterEach(async () => {
	inbox?.close();
	execution?.close();
	await vi.advanceTimersByTimeAsync(1);
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

test("inbox and queued execution clients use one worker and one account transport", async () => {
	vi.useFakeTimers();
	vi.stubGlobal("window", new EventTarget());
	const workers: { url: string; name: string }[] = [];
	const polls: RealtimePollRequest[] = [];
	let grants = 0;
	vi.stubGlobal(
		"fetch",
		vi.fn(async (url: string, init?: RequestInit) => {
			if (String(url).endsWith("/invoke"))
				return Response.json({
					run_id: "run",
					poll_token: "private-capability",
					status: "pending",
				});
			if (String(url).endsWith("/connect")) {
				grants++;
				return Response.json({
					protocol_version: 1,
					expires_at: Math.floor(Date.now() / 1000) + 900,
					transport: { type: "http" },
				});
			}
			const request: RealtimePollRequest = JSON.parse(String(init?.body));
			polls.push(request);
			if (request.runs.length)
				return Response.json({
					protocol_version: 1,
					errors: [],
					notifications: {
						revision: "n1",
						overview: {
							invites_count: 0,
							notifications_count: 1,
							unread_count: 1,
						},
					},
					runs: [
						{
							run_id: "run",
							revision: "r1",
							status: "completed",
							progress: 100,
							after_sequence: -1,
							has_more: false,
							events: [],
						},
					],
				});
			return new Promise<Response>((_, reject) =>
				init?.signal?.addEventListener(
					"abort",
					() => reject(new Error("aborted")),
					{ once: true },
				),
			);
		}),
	);
	await import("../realtime/runtime.shared-worker");
	vi.stubGlobal(
		"SharedWorker",
		class {
			onerror: (() => void) | null = null;
			port: MessagePort;
			constructor(url: URL, options: { name: string }) {
				workers.push({ url: url.href, name: options.name });
				const ports = [
					{ onmessage: null },
					{ onmessage: null },
				] as unknown as MessagePort[];
				for (let index = 0; index < 2; index++) {
					ports[index].start = () => {};
					ports[index].close = () => {};
					ports[index].postMessage = (value) =>
						queueMicrotask(() =>
							ports[1 - index].onmessage?.call(ports[1 - index], {
								data: structuredClone(value),
							} as MessageEvent),
						);
				}
				this.port = ports[0];
				(
					globalThis as unknown as {
						onconnect: (event: { ports: MessagePort[] }) => void;
					}
				).onconnect({ ports: [ports[1]] });
			}
		},
	);
	const { getRealtimeClient } = await import("../realtime/runtime-client");
	const { getExecutionRuntime } = await import("./runtime-client");
	const session = {
		scope: "account",
		url: "https://hub.test/api/v1/realtime/poll",
		headers: { Authorization: "Bearer account" },
	};
	inbox = await getRealtimeClient(session);
	const updates = vi.fn();
	inbox.subscribe(updates);
	inbox.setSubscriptions({ notifications: true, runs: [] });
	execution = await getExecutionRuntime("execution-scope");
	const running = execution.execute(
		{
			taskId: "task",
			streamId: "stream",
			scope: "execution-scope",
			appId: "app",
			eventId: "event",
			title: "Workflow",
			startedAt: new Date().toISOString(),
		},
		{ url: "https://hub.test/invoke", body: {}, realtime: session },
	);
	await vi.advanceTimersByTimeAsync(1);
	await running;
	expect(workers).toHaveLength(2);
	const first = workers[0];
	expect(workers[1]).toEqual(first);
	expect(first.name).toBe("flow-like-realtime-executions-v2");
	expect(grants).toBe(1);
	expect(
		polls.some(
			(request) => request.notifications && request.runs[0]?.run_id === "run",
		),
	).toBe(true);
	expect(updates).toHaveBeenCalledWith(
		expect.objectContaining({
			type: "snapshot",
			snapshot: expect.objectContaining({
				notifications: expect.objectContaining({ revision: "n1" }),
			}),
		}),
	);
});
