import { describe, expect, test } from "bun:test";
import { waitForDeploymentRollout } from "../deployment";
import type { InventoryWriter } from "../inventory";
import {
	ConnectError,
	type ConnectProgress,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
} from "../transport";
import type {
	BrowserController,
	DeviceReceipt,
	ManagementResponse,
} from "../types";
import { LiveCallError } from "./errors";
import {
	LIVE_TIMING,
	type LiveConnectInput,
	type LiveConnection,
	createLiveSessionManager,
} from "./live";
import type { KeyPort, WorkspaceDeps } from "./types";

const DEVICE = "device-1";
const BUSY = "Wait for the current device operation to finish.";

type Handler = (
	command: Record<string, unknown>,
	conn: FakeConn,
	operationId?: string,
) => ManagementResponse | Promise<ManagementResponse>;

interface Sent {
	command: Record<string, unknown>;
	conn: number;
	remainingS: number;
}

class FakeConn implements LiveConnection {
	readonly transport = "websocket" as const;
	readonly fallbackReason = "no_turn_servers" as const;
	readonly bootId = "boot-1";
	closed = false;
	private inFlight = false;
	private readonly listeners = new Set<(reason: "local" | "remote") => void>();
	constructor(
		readonly serial: number,
		readonly expiresAt: number,
		private readonly env: Env,
	) {}
	get open() {
		return !this.closed && this.expiresAt > this.env.nowS();
	}
	async request(command: Record<string, unknown>, operationId?: string) {
		if (this.inFlight) throw new Error(BUSY);
		if (!this.open)
			throw new Error(
				"Management session expired. Reconnect before continuing.",
			);
		this.inFlight = true;
		this.env.sent.push({
			command,
			conn: this.serial,
			remainingS: this.expiresAt - this.env.nowS(),
		});
		try {
			await Promise.resolve();
			return await this.env.handler(command, this, operationId);
		} finally {
			this.inFlight = false;
		}
	}
	close() {
		this.finish("local");
	}
	drop() {
		this.finish("remote");
	}
	private finish(reason: "local" | "remote") {
		if (this.closed) return;
		this.closed = true;
		for (const listener of this.listeners) listener(reason);
	}
	onClosed(listener: (reason: "local" | "remote") => void) {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}
}

function completed(result: Record<string, unknown> = {}): ManagementResponse {
	return { operation_id: "op", state: "completed", result };
}

function inspectPage(): ManagementResponse {
	return completed({
		device_id: DEVICE,
		boot_id: "boot-1",
		placements: [],
		next: null,
		features: { task_health: 1 },
	});
}

const defaultHandler: Handler = (command) =>
	command.type === "inspect_page" ? inspectPage() : completed({ ok: true });

class Env {
	nowMs = 1_700_000_000_000;
	sent: Sent[] = [];
	conns: FakeConn[] = [];
	connects: LiveConnectInput[] = [];
	handler: Handler = defaultHandler;
	/** Next connect outcomes; when empty a fresh 300 s session is returned. */
	outcomes: (Error | number)[] = [];
	presence: "online" | "late" | "offline" | "never" | "revoked" = "online";
	controller: BrowserController | undefined = {} as BrowserController;
	activity: { start: unknown[]; finish: unknown[] } = { start: [], finish: [] };
	written: unknown[] = [];
	inspected: string[] = [];
	observed: unknown[][] = [];
	grantId = "owner";
	inventoryWriter: () => Promise<InventoryWriter> =
		async () => async (inspection) => {
			this.written.push(inspection);
		};
	private timers: { at: number; run: () => void; id: number }[] = [];
	private nextId = 0;
	private keyListeners = new Set<() => void>();

	nowS() {
		return Math.floor(this.nowMs / 1000);
	}
	get pendingTimers() {
		return this.timers.length;
	}
	schedule = (run: () => void, ms: number) => {
		const id = ++this.nextId;
		this.timers.push({ at: this.nowMs + ms, run, id });
		return () => {
			this.timers = this.timers.filter((timer) => timer.id !== id);
		};
	};
	async advance(ms: number) {
		const end = this.nowMs + ms;
		for (;;) {
			await flush();
			this.timers.sort((a, b) => a.at - b.at);
			const next = this.timers[0];
			if (!next || next.at > end) break;
			this.timers.shift();
			this.nowMs = Math.max(this.nowMs, next.at);
			next.run();
		}
		this.nowMs = end;
		await flush();
	}
	setController(controller: BrowserController | undefined) {
		this.controller = controller;
		for (const listener of this.keyListeners) listener();
	}
	keys(): KeyPort {
		return {
			controller: () => this.controller,
			vault: () =>
				this.controller
					? ({ grantId: this.grantId } as ReturnType<KeyPort["vault"]>)
					: undefined,
			receipt: () =>
				this.controller ? ({ device_id: DEVICE } as DeviceReceipt) : undefined,
			snapshot: () => {
				throw new Error("unused");
			},
			subscribe: (listener) => {
				this.keyListeners.add(listener);
				return () => this.keyListeners.delete(listener);
			},
			touch: () => {},
		};
	}
	connect = async (input: LiveConnectInput): Promise<LiveConnection> => {
		this.connects.push(input);
		const step = (progress: ConnectProgress) => input.onStep(progress);
		step({ step: "getting_pass", state: "active" });
		await Promise.resolve();
		const outcome = this.outcomes.shift() ?? 300;
		if (outcome instanceof Error) throw outcome;
		step({
			step: "getting_pass",
			state: "done",
			admission: { expiresAt: this.nowS() + 300, receivedAtMs: this.nowMs },
		});
		step({ step: "reaching_device", state: "active" });
		step({ step: "reaching_device", state: "done" });
		step({ step: "trying_direct", state: "active" });
		step({
			step: "trying_direct",
			state: "skipped",
			fallbackReason: "no_turn_servers",
		});
		step({ step: "securing", state: "active" });
		step({ step: "securing", state: "done" });
		const conn = new FakeConn(
			this.conns.length + 1,
			this.nowS() + outcome,
			this,
		);
		this.conns.push(conn);
		return conn;
	};
}

async function flush() {
	for (let round = 0; round < 6; round++)
		await new Promise<void>((resolve) => setImmediate(resolve));
}

function setup(configure: (env: Env) => void = () => {}) {
	const env = new Env();
	configure(env);
	const deps = { now: () => env.nowMs } as unknown as WorkspaceDeps;
	const manager = createLiveSessionManager(
		deps,
		{
			keys: env.keys(),
			hub: { fetch: async () => undefined as never },
			clock: {
				now: () => env.nowMs,
				deviceSkewS: () => undefined,
				observe: (...args) => {
					env.observed.push(args);
				},
			},
			activity: {
				start: (item) => {
					env.activity.start.push(item);
					return "activity-1";
				},
				finish: (...args) => {
					env.activity.finish.push(args);
				},
			},
			presence: () => env.presence,
		},
		{
			connect: env.connect,
			schedule: env.schedule,
			inventoryWriter: () => env.inventoryWriter(),
			onInspection: (deviceId) => {
				env.inspected.push(deviceId);
			},
		},
	);
	return { env, manager };
}

const types = (env: Env) => env.sent.map((row) => row.command.type);

describe("connecting", () => {
	test("demand connects, reports steps, reads services and feeds the saved inventory", async () => {
		const { env, manager } = setup();
		const states: string[] = [];
		manager.subscribe(() => states.push(manager.state(DEVICE).kind));
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		const state = manager.state(DEVICE);
		expect(state).toMatchObject({
			kind: "live",
			transport: "websocket",
			fallbackReason: "no_turn_servers",
			bootId: "boot-1",
			expiresAt: env.nowS() + 300,
		});
		expect(states).toContain("connecting");
		expect(manager.steps(DEVICE).map((step) => step.state)).toEqual([
			"done",
			"done",
			"skipped",
			"done",
			"done",
		]);
		expect(manager.steps(DEVICE)[2].detail).toEqual({
			code: "relay_no_turn_servers",
		});
		expect(types(env)).toEqual(["inspect_page"]);
		const inspection = manager.inspection(DEVICE);
		expect(inspection?.value.features).toEqual({ task_health: 1 });
		expect(inspection?.readAt).toBe(env.nowS());
		expect(env.written).toHaveLength(1);
	});

	test("an owner's admission feeds the hub clock with the raw local time; a grantee's does not", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(env.observed).toEqual([["admission", env.nowS(), env.nowMs]]);
		const shared = setup((env) => {
			env.grantId = "grant-7";
		});
		shared.manager.acquire(DEVICE, "view");
		await shared.env.advance(0);
		expect(shared.env.observed).toEqual([]);
	});

	test("a locked device refuses calls without connecting", async () => {
		const { env, manager } = setup((env) => {
			env.controller = undefined;
		});
		const error = await manager
			.call(DEVICE)({ type: "stop" })
			.catch((failure: unknown) => failure);
		expect(error).toBeInstanceOf(LiveCallError);
		expect((error as LiveCallError).code).toBe("keys_locked");
		expect(env.connects).toHaveLength(0);
	});

	test("ended access, re-enrolment and identity failures stop retrying", async () => {
		const { env, manager } = setup((env) => {
			env.outcomes = [
				new ConnectError("getting_pass", "access_expired", "x", {
					status: 401,
				}),
			];
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(manager.state(DEVICE)).toEqual({
			kind: "failed",
			cause: { step: "getting_pass", code: "access_expired", status: 401 },
		});
		await env.advance(120_000);
		expect(env.connects).toHaveLength(1);
		const error = await manager
			.call(DEVICE)({ type: "metrics" })
			.catch((failure: unknown) => failure);
		expect((error as LiveCallError).code).toBe("access_expired");
		expect(manager.steps(DEVICE)[0]).toEqual({
			id: "getting_pass",
			state: "failed",
			detail: { code: "access_expired" },
		});
	});
});

describe("renewal", () => {
	test("renews 45 s before expiry, break-then-make, and holds requests meanwhile", async () => {
		const { env, manager } = setup((env) => {
			env.outcomes = [120, 300];
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		expect(env.conns).toHaveLength(1);
		await env.advance((120 - LIVE_TIMING.renewBeforeS) * 1000 - 1000);
		expect(env.conns).toHaveLength(1);
		await env.advance(1000);
		expect(env.conns[0].closed).toBe(true);
		expect(env.conns).toHaveLength(2);
		expect(manager.state(DEVICE)).toMatchObject({ kind: "live" });
		const response = await manager.call(DEVICE)({ type: "metrics" });
		expect(response.state).toBe("completed");
		expect(env.sent.at(-1)?.conn).toBe(2);
	});

	test("never sends a request on a session that expires inside its reply window", async () => {
		const { env, manager } = setup((env) => {
			env.outcomes = [25];
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		env.nowMs += 10_000;
		await manager.call(DEVICE)({ type: "metrics" });
		expect(env.sent.at(-1)).toMatchObject({ conn: 2 });
		expect(env.conns[0].closed).toBe(true);
		for (const row of env.sent)
			expect(row.remainingS).toBeGreaterThan(LIVE_TIMING.replyWindowS);
	});
});

describe("reconnecting", () => {
	test("an unexpected close reconnects with 1, 2, 5, 10, 30, 60 s backoff", async () => {
		const failure = new ConnectError(
			"reaching_device",
			"relay_unreachable",
			"Device signaling could not be reached.",
		);
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.outcomes = Array.from({ length: 7 }, () => failure);
		const delays: number[] = [];
		env.conns[0].drop();
		for (let attempt = 1; attempt <= 7; attempt++) {
			const state = manager.state(DEVICE);
			if (state.kind !== "reconnecting") throw new Error(state.kind);
			expect(state.attempt).toBe(attempt);
			delays.push(state.retryAt - env.nowS());
			await env.advance((state.retryAt - env.nowS()) * 1000);
		}
		expect(delays).toEqual([1, 2, 5, 10, 30, 60, 60]);
		await env.advance(60_000);
		expect(manager.state(DEVICE)).toMatchObject({ kind: "live" });
		const reconnected = manager.state(DEVICE);
		if (reconnected.kind === "live")
			expect(reconnected.connectedAt).toBe(env.nowS());
	});

	test("an offline device retries every 60 s and at once when it comes back", async () => {
		const failure = new ConnectError(
			"reaching_device",
			"relay_unreachable",
			"Device signaling could not be reached.",
		);
		const { env, manager } = setup((env) => {
			env.presence = "offline";
			env.outcomes = [failure, failure];
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(manager.state(DEVICE)).toMatchObject({
			kind: "unreachable",
			retryAt: env.nowS() + 60,
			cause: { step: "reaching_device", code: "relay_unreachable" },
		});
		await env.advance(60_000);
		expect(env.connects).toHaveLength(2);
		expect(manager.state(DEVICE).kind).toBe("unreachable");
		env.presence = "online";
		await env.advance(LIVE_TIMING.presenceCheckMs);
		expect(env.connects).toHaveLength(3);
		expect(manager.state(DEVICE).kind).toBe("live");
	});

	test("a full connection slot on the first read reconnects and says why", async () => {
		const { env, manager } = setup((env) => {
			let first = true;
			env.handler = (command) => {
				if (command.type === "inspect_page" && first) {
					first = false;
					return {
						operation_id: "op",
						state: "rejected",
						result: { code: "limit", error: "slots", retryable: true },
					};
				}
				return defaultHandler(command, env.conns[0]);
			};
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(manager.state(DEVICE)).toMatchObject({
			kind: "reconnecting",
			cause: { step: "reading_services", code: "rejected" },
		});
		expect(manager.steps(DEVICE).at(-1)).toEqual({
			id: "reading_services",
			state: "skipped",
			detail: { code: "slots_in_use" },
		});
		await env.advance(1000);
		expect(manager.state(DEVICE).kind).toBe("live");
		expect(manager.inspection(DEVICE)).toBeDefined();
	});
});

describe("request queue", () => {
	test("a Stop click passes a 90 MiB upload within one 8 KiB chunk", async () => {
		const chunks = (90 * 1024 * 1024) / (8 * 1024);
		const { env, manager } = setup();
		manager.acquire(DEVICE, "operation");
		await env.advance(0);
		const user = manager.call(DEVICE, { lane: "user" });
		let stop: Promise<ManagementResponse> | undefined;
		env.handler = (command) => {
			if (command.type === "artifact" && command.chunk === 5000)
				stop = user({ type: "stop" });
			return completed();
		};
		const upload = manager.call(DEVICE, { lane: "operation" });
		for (let chunk = 0; chunk < chunks; chunk++)
			await upload({ type: "artifact", chunk });
		await stop;
		const order = env.sent.map((row) =>
			row.command.type === "stop" ? "stop" : row.command.chunk,
		);
		const at = order.indexOf("stop");
		expect(order[at - 1]).toBe(5000);
		expect(order[at + 1]).toBe(5001);
		expect(
			env.sent.filter((row) => row.command.type === "artifact"),
		).toHaveLength(chunks);
	});

	test("user calls go before operations, operations before polls", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.sent = [];
		const calls = [
			manager.call(DEVICE, { lane: "poll" })({ type: "metrics" }),
			manager.call(DEVICE, { lane: "operation" })({ type: "artifact" }),
			manager.call(DEVICE, { lane: "user" })({ type: "rollout" }),
			manager.call(DEVICE, { lane: "poll" })({ type: "logs" }),
			manager.call(DEVICE, { lane: "user" })({ type: "operation" }),
		];
		await Promise.all(calls);
		expect(types(env)).toEqual([
			"rollout",
			"operation",
			"artifact",
			"metrics",
			"logs",
		]);
	});

	test("the transport's busy guard never triggers under concurrent load", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		const lanes = ["user", "operation", "poll"] as const;
		const calls: Promise<unknown>[] = [];
		for (let index = 0; index < 60; index++)
			calls.push(
				manager.call(DEVICE, { lane: lanes[index % 3] })({
					type: "metrics",
					index,
				}),
			);
		calls.push(
			manager.exclusive(DEVICE, (call) =>
				Promise.all([
					call({ type: "telemetry_read", n: 1 }),
					call({ type: "telemetry_read", n: 2 }),
				]),
			),
		);
		const results = await Promise.allSettled(calls);
		expect(results.every((result) => result.status === "fulfilled")).toBe(true);
	});

	test("identical polls coalesce while one is still queued", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.sent = [];
		const poll = manager.call(DEVICE, { lane: "poll" });
		const blocker = manager.call(DEVICE, { lane: "user" })({ type: "stop" });
		const first = poll({ type: "metrics", placement_id: "a" });
		const second = poll({ type: "metrics", placement_id: "a" });
		const other = poll({ type: "metrics", placement_id: "b" });
		expect(second).toBe(first);
		await Promise.all([blocker, first, other]);
		expect(
			env.sent.filter((row) => row.command.type === "metrics"),
		).toHaveLength(2);
	});

	test("exclusive sections run alone; user calls queue behind them", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.sent = [];
		let release!: () => void;
		const gate = new Promise<void>((resolve) => {
			release = resolve;
		});
		const section = manager.exclusive(DEVICE, async (call) => {
			await call({ type: "telemetry_roster_read" });
			await gate;
			await call({ type: "telemetry_policy" });
			return "done";
		});
		await flush();
		const stop = manager.call(DEVICE, { lane: "user" })({ type: "stop" });
		await flush();
		expect(types(env)).toEqual(["telemetry_roster_read"]);
		release();
		expect(await section).toBe("done");
		await stop;
		expect(types(env).slice(0, 3)).toEqual([
			"telemetry_roster_read",
			"telemetry_policy",
			"stop",
		]);
	});

	test("an aborted queued call never reaches the device", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.sent = [];
		const abort = new AbortController();
		const blocker = manager.call(DEVICE)({ type: "rollout" });
		const cancelled = manager
			.call(DEVICE, { signal: abort.signal })({ type: "start" })
			.catch((error: unknown) => error);
		abort.abort(new Error("cancelled by user"));
		await blocker;
		expect(((await cancelled) as Error).message).toBe("cancelled by user");
		expect(types(env)).toEqual(["rollout"]);
	});
});

describe("delivery guarantees", () => {
	test("a request that was not sent is retried once on the next session", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		let failures = 0;
		env.handler = (command, conn) => {
			if (command.type === "stop" && failures++ === 0) {
				conn.close();
				throw new ManagementRequestNotSentError("not sent");
			}
			return completed();
		};
		const response = await manager.call(DEVICE)({ type: "stop" });
		expect(response.state).toBe("completed");
		expect(env.conns).toHaveLength(2);
		expect(
			env.sent
				.filter((row) => row.command.type === "stop")
				.map((row) => row.conn),
		).toEqual([1, 2]);
	});

	test("an oversized request fails without reconnecting", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.handler = () => {
			throw new ManagementRequestNotSentError("too large");
		};
		const error = await manager
			.call(DEVICE)({ type: "apply" })
			.catch((failure: unknown) => failure);
		expect(error).toBeInstanceOf(ManagementRequestNotSentError);
		expect(env.conns).toHaveLength(1);
	});

	test("an unconfirmed command is never retried and becomes an unknown tray item", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		env.handler = (command, conn, operationId) => {
			if (command.type === "stop") {
				conn.close();
				throw new ManagementUnconfirmedError(operationId ?? "op-stop");
			}
			return defaultHandler(command, conn);
		};
		const error = await manager
			.call(DEVICE, {
				trackUnconfirmed: {
					kind: "command",
					target: { deviceId: DEVICE, serviceId: "svc" },
				},
			})({ type: "stop" }, "op-stop")
			.catch((failure: unknown) => failure);
		expect(error).toBeInstanceOf(ManagementUnconfirmedError);
		expect(env.sent.filter((row) => row.command.type === "stop")).toHaveLength(
			1,
		);
		expect(env.activity.start[0]).toMatchObject({
			kind: "command",
			state: "unknown",
			target: { deviceId: DEVICE, serviceId: "svc" },
			detail: { code: "no_reply" },
			resume: { type: "operation", operationId: "op-stop", command: "stop" },
		});
		expect(env.activity.finish[0]).toEqual([
			"activity-1",
			"unknown",
			{ code: "no_reply" },
		]);
	});

	test("background work that loses a reply puts nothing in the tray", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		env.handler = (command, conn, operationId) => {
			if (command.type !== "telemetry_receipt")
				return defaultHandler(command, conn);
			conn.close();
			throw new ManagementUnconfirmedError(operationId ?? "op-receipt");
		};
		const receipt = { type: "telemetry_receipt" };
		const failure = (attempt: Promise<unknown>) =>
			attempt.catch((error: unknown) => error);
		expect(
			await failure(
				manager.exclusive(DEVICE, (call) => call(receipt), { lane: "poll" }),
			),
		).toBeInstanceOf(ManagementUnconfirmedError);
		expect(
			await failure(manager.call(DEVICE, { lane: "poll" })(receipt)),
		).toBeInstanceOf(ManagementUnconfirmedError);
		expect(env.activity.start).toHaveLength(0);
		expect(
			await failure(manager.exclusive(DEVICE, (call) => call(receipt))),
		).toBeInstanceOf(ManagementUnconfirmedError);
		expect(env.activity.start).toMatchObject([
			{ kind: "command", state: "unknown", target: { deviceId: DEVICE } },
		]);
	});

	test("waitForDeploymentRollout survives one lost reply", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "operation");
		await env.advance(0);
		let lost = false;
		env.handler = (command, conn, operationId) => {
			if (command.type !== "rollout") return defaultHandler(command, conn);
			if (!lost) {
				lost = true;
				conn.close();
				throw new ManagementUnconfirmedError(operationId ?? "op");
			}
			return completed({
				rollout_id: "rollout-1",
				placement_id: "placement-1",
				project_id: "project-1",
				state: "healthy",
			});
		};
		const status = await waitForDeploymentRollout(
			manager.call(DEVICE, { lane: "operation" }),
			{
				rollout_id: "rollout-1",
				placement_id: "placement-1",
				project_id: "project-1",
			},
		);
		expect(status.state).toBe("healthy");
		expect(
			env.sent.filter((row) => row.command.type === "rollout"),
		).toHaveLength(2);
		expect(env.activity.start).toHaveLength(0);
	});
});

describe("inspection cache and demand", () => {
	test("re-reads after a state-changing user command and every 20 s under view demand", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		await manager.call(DEVICE)({ type: "restart" });
		await env.advance(0);
		expect(types(env)).toEqual(["inspect_page", "restart", "inspect_page"]);
		await manager.call(DEVICE)({ type: "metrics" });
		await env.advance(0);
		expect(types(env).filter((type) => type === "inspect_page")).toHaveLength(
			2,
		);
		await env.advance(LIVE_TIMING.inspectionEveryMs);
		expect(types(env).filter((type) => type === "inspect_page")).toHaveLength(
			3,
		);
	});

	test("stream demand alone does not poll the inspection", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "stream");
		await env.advance(LIVE_TIMING.inspectionEveryMs * 2);
		expect(types(env)).toEqual(["inspect_page"]);
	});

	test("the session closes 60 s after demand ends unless demand returns", async () => {
		const { env, manager } = setup();
		const release = manager.acquire(DEVICE, "view");
		await env.advance(0);
		release();
		await env.advance(30_000);
		const again = manager.acquire(DEVICE, "stream");
		await env.advance(60_000);
		expect(env.conns[0].closed).toBe(false);
		again();
		await env.advance(LIVE_TIMING.lingerMs);
		expect(env.conns[0].closed).toBe(true);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
		expect(manager.inspection(DEVICE)).toBeDefined();
	});

	test("a session that expired while nobody needed it is closed and cannot disturb its successor", async () => {
		const { env, manager } = setup((env) => {
			env.outcomes = [50, 300];
		});
		manager.acquire(DEVICE, "view")();
		await env.advance(55_000);
		expect(env.conns[0].open).toBe(false);
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(env.conns).toHaveLength(2);
		expect(env.conns[0].closed).toBe(true);
		expect(manager.state(DEVICE).kind).toBe("live");
		env.conns[0].drop();
		await env.advance(10_000);
		expect(manager.state(DEVICE).kind).toBe("live");
		expect(env.conns).toHaveLength(2);
	});

	test("a failed read drops the page progress of that read", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		const row = {
			id: "placement-1",
			project_id: "project-1",
			deployment_id: "deployment-1",
			revision: "r1",
			desired_state: "running",
			observed_state: "running",
			config_revision: 1,
			intent_revision: 1,
			applied_revision: 1,
			desired_replicas: 1,
			running_replicas: 1,
			ready_replicas: 1,
			max_replicas: 1,
			replicas: [],
		};
		env.handler = (command) =>
			command.type !== "inspect_page"
				? completed()
				: command.after === null
					? completed({
							device_id: DEVICE,
							boot_id: "boot-1",
							placements: [row],
							next: row.id,
						})
					: {
							operation_id: "op",
							state: "rejected",
							result: { code: "failed", error: "busy disk", retryable: false },
						};
		await manager.refreshInspection(DEVICE);
		const inspection = manager.inspection(DEVICE);
		expect(inspection?.error).toMatchObject({
			step: "reading_services",
			code: "rejected",
		});
		expect(inspection?.progress).toBeUndefined();
		expect(inspection?.value.placements).toEqual([]);
	});

	test("locking the keys ends the session, clears live data and refuses queued calls", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		let release!: () => void;
		env.handler = async (command) => {
			if (command.type === "stop")
				await new Promise<void>((resolve) => {
					release = resolve;
				});
			return completed();
		};
		const inFlight = manager.call(DEVICE)({ type: "stop" });
		await flush();
		const queued = manager
			.call(DEVICE)({ type: "start" })
			.catch((error: unknown) => error);
		env.setController(undefined);
		release();
		await inFlight.catch(() => {});
		expect(((await queued) as LiveCallError).code).toBe("keys_locked");
		expect(env.conns[0].closed).toBe(true);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
		expect(manager.inspection(DEVICE)).toBeUndefined();
	});

	test("close() disconnects and refuses what was still queued", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		manager.close(DEVICE);
		expect(env.conns[0].closed).toBe(true);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
		await env.advance(10_000);
		expect(env.conns).toHaveLength(1);
		env.setController(env.controller);
		await env.advance(10_000);
		expect(env.conns).toHaveLength(1);
	});

	test("demand taken while the device is locked is served as soon as its keys open", async () => {
		const { env, manager } = setup((env) => {
			env.controller = undefined;
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		expect(env.connects).toHaveLength(0);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
		env.setController({} as BrowserController);
		await env.advance(0);
		expect(manager.state(DEVICE).kind).toBe("live");

		env.setController(undefined);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
		env.setController({} as BrowserController);
		await env.advance(0);
		expect(manager.state(DEVICE).kind).toBe("live");
		expect(env.conns).toHaveLength(2);
	});

	test("opening the keys of a device nobody watches connects nothing", async () => {
		const { env, manager } = setup((env) => {
			env.controller = undefined;
		});
		manager.acquire(DEVICE, "view")();
		env.setController({} as BrowserController);
		await env.advance(120_000);
		expect(env.connects).toHaveLength(0);
	});

	test("a lock and unlock during a connection attempt reconnects with the new keys", async () => {
		let finish!: () => void;
		const { env, manager } = setup((env) => {
			const connect = env.connect;
			env.connect = async (input) => {
				if (env.connects.length === 0)
					await new Promise<void>((resolve) => {
						finish = resolve;
					});
				return connect(input);
			};
		});
		manager.acquire(DEVICE, "view");
		await flush();
		const next = {} as BrowserController;
		env.setController(undefined);
		env.setController(next);
		finish();
		await env.advance(0);
		expect(manager.state(DEVICE).kind).toBe("live");
		expect(env.connects.at(-1)?.controller).toBe(next);
		expect(env.conns.filter((conn) => !conn.closed)).toHaveLength(1);
	});

	test("an attempt ended by a lock reports no further progress", async () => {
		let finish!: () => void;
		const { env, manager } = setup((env) => {
			const connect = env.connect;
			env.connect = async (input) => {
				await new Promise<void>((resolve) => {
					finish = resolve;
				});
				return connect(input);
			};
		});
		manager.acquire(DEVICE, "view");
		await flush();
		env.setController(undefined);
		const states: string[] = [];
		manager.subscribe(() => states.push(manager.state(DEVICE).kind));
		finish();
		await env.advance(0);
		expect(states.every((kind) => kind === "idle")).toBe(true);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
		expect(
			manager.steps(DEVICE).every((step) => step.state === "pending"),
		).toBe(true);
		expect(env.conns.every((conn) => conn.closed)).toBe(true);
	});

	test("dispose stops every timer, also while a view still holds demand", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		manager.dispose();
		expect(env.pendingTimers).toBe(0);
		expect(manager.state(DEVICE)).toEqual({ kind: "idle" });
	});
});

describe("saved inventory", () => {
	test("a failed write never fails the inspection, and the next session gets a new writer", async () => {
		let writers = 0;
		const { env, manager } = setup((env) => {
			env.outcomes = [120, 300];
			env.inventoryWriter = async () => {
				const serial = ++writers;
				let failed = false;
				return (inspection) => {
					if (failed)
						throw new Error("Reconnect before retaining another inspection.");
					if (serial > 1) {
						env.written.push(inspection);
						return Promise.resolve();
					}
					failed = true;
					return Promise.reject(new Error("Inventory upload failed."));
				};
			};
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(manager.inspection(DEVICE)?.error).toBeUndefined();
		await env.advance(LIVE_TIMING.inspectionEveryMs);
		expect(env.inspected).toHaveLength(2);
		expect(manager.inspection(DEVICE)?.error).toBeUndefined();
		expect(manager.steps(DEVICE).at(-1)).toEqual({
			id: "reading_services",
			state: "done",
		});
		expect(writers).toBe(1);
		await env.advance((120 - LIVE_TIMING.renewBeforeS) * 1000);
		expect(env.conns).toHaveLength(2);
		expect(writers).toBe(2);
		expect(env.written.length).toBeGreaterThan(0);
		expect(manager.inspection(DEVICE)?.error).toBeUndefined();
	});

	test("a writer that refuses an inspection outright is contained too", async () => {
		const { env, manager } = setup((env) => {
			env.inventoryWriter = async () => () => {
				throw new Error("Invalid retained observation.");
			};
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(env.inspected).toEqual([DEVICE]);
		expect(manager.inspection(DEVICE)?.error).toBeUndefined();
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("done");
	});

	test("a writer that could not be prepared is prepared again on the next session", async () => {
		let attempts = 0;
		const { env, manager } = setup((env) => {
			env.outcomes = [120, 300];
			env.inventoryWriter = async () => {
				if (attempts++ === 0) throw new Error("Inventory view unavailable.");
				return async (inspection) => {
					env.written.push(inspection);
				};
			};
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(manager.inspection(DEVICE)).toBeDefined();
		expect(env.written).toHaveLength(0);
		await env.advance((120 - LIVE_TIMING.renewBeforeS) * 1000);
		expect(attempts).toBe(2);
		expect(env.written.length).toBeGreaterThan(0);
	});

	test("the observation time is a whole number of milliseconds even when the hub clock is not", async () => {
		const { env, manager } = setup();
		env.nowMs += 0.5;
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(
			Number.isSafeInteger(manager.inspection(DEVICE)?.value.observed_at),
		).toBe(true);
	});
});
