import { describe, expect, test } from "bun:test";
import type { ArtifactTransferStatus } from "../artifacts";
import { waitForDeploymentRollout } from "../deployment";
import type { InventoryWriter } from "../inventory";
import type { ModelAssetStatus } from "../models";
import {
	ConnectError,
	type ConnectProgress,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
} from "../transport";
import type { DeviceServiceStream } from "../tunnel";
import type {
	DeviceTunnelDataClient,
	TunnelModelAssetPush,
} from "../tunnel-data";
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
	openService?: LiveConnection["openService"];
	openModelGateway?: LiveConnection["openModelGateway"];
	requestData?: LiveConnection["requestData"];
	uploadArtifact?: LiveConnection["uploadArtifact"];
	pushModelAsset?: LiveConnection["pushModelAsset"];
	detachDataTunnel?: LiveConnection["detachDataTunnel"];
	adoptDataTunnel?: LiveConnection["adoptDataTunnel"];
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

function serviceStream() {
	let resolve!: () => void;
	let resets = 0;
	const stream = {
		closed: new Promise<void>((done) => {
			resolve = done;
		}),
		reset() {
			resets++;
			resolve();
		},
	} as unknown as DeviceServiceStream;
	return { stream, resets: () => resets, finish: () => resolve() };
}

function modelTunnel(env: Env) {
	const { stream, resets, finish } = serviceStream();
	const data = { close: () => stream.reset() } as DeviceTunnelDataClient;
	const adopted: DeviceTunnelDataClient[] = [];
	env.configureConnection = (conn) => {
		let held: DeviceTunnelDataClient | undefined =
			conn.serial === 1 ? data : undefined;
		conn.openModelGateway = async () => stream;
		conn.detachDataTunnel = () => {
			const previous = held;
			held = undefined;
			return previous;
		};
		conn.adoptDataTunnel = (next) => {
			held = next;
			adopted.push(next);
		};
		const close = conn.close.bind(conn);
		conn.close = () => {
			held?.close();
			held = undefined;
			close();
		};
		const drop = conn.drop.bind(conn);
		conn.drop = () => {
			drop();
			held?.close();
			held = undefined;
		};
	};
	return { stream, resets, finish, data, adopted };
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
	configureConnection?: (connection: FakeConn) => void;
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
		this.configureConnection?.(conn);
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
	test("a grant change during the handshake closes the attempted connection", async () => {
		const { env, manager } = setup((env) => {
			env.configureConnection = () => {
				env.grantId = "replacement-grant";
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		expect(env.conns).toHaveLength(1);
		expect(env.conns[0].closed).toBe(true);
		expect(env.inspected).toEqual([]);
		manager.dispose();
	});
	test("a failed reconnect preserves safe connection diagnostics through the live-call wrapper", async () => {
		const { env, manager } = setup((env) => {
			env.outcomes.push(
				new ConnectError(
					"securing",
					"handshake_failed",
					"private native failure",
					{
						transport: "websocket",
						fallbackReason: "ice_timeout",
						url: "wss://private.example/ws/devices",
					},
				),
			);
		});
		const error = await manager
			.call(DEVICE)({ type: "artifact" })
			.catch((error) => error);
		expect(error).toBeInstanceOf(LiveCallError);
		expect(error.diagnostic).toEqual({
			transport: "websocket",
			phase: "securing",
			cause: "handshake_failed",
			fallbackReason: "ice_timeout",
		});
		expect(JSON.stringify(error.diagnostic)).not.toContain("private");
		expect(env.sent).toHaveLength(0);
		manager.dispose();
	});

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
	test("a model answer survives a failed scheduled renewal and its retry", async () => {
		const { env, manager } = setup((env) => {
			env.outcomes = [120];
		});
		const model = modelTunnel(env);
		await manager.openModelGateway(DEVICE);
		env.outcomes = [
			new ConnectError("getting_pass", "http", "The hub is unavailable."),
			300,
		];
		await env.advance(75_000);
		expect(manager.state(DEVICE).kind).toBe("reconnecting");
		expect(model.resets()).toBe(0);
		await env.advance(1_000);
		expect(manager.state(DEVICE).kind).toBe("live");
		expect(model.adopted).toEqual([model.data]);
		expect(model.resets()).toBe(0);
		manager.close(DEVICE);
		expect(model.resets()).toBe(1);
		manager.dispose();
	});

	test("keeps service demand and adopts its tunnel across management renewal", async () => {
		const { stream, resets, finish } = serviceStream();
		const owner = { close: () => stream.reset() } as DeviceTunnelDataClient;
		const adopted: DeviceTunnelDataClient[] = [];
		const { env, manager } = setup((env) => {
			env.outcomes = [120, 300];
			env.configureConnection = (conn) => {
				let held: DeviceTunnelDataClient | undefined =
					conn.serial === 1 ? owner : undefined;
				conn.openService = async () => stream;
				conn.detachDataTunnel = () => {
					const previous = held;
					held = undefined;
					return previous;
				};
				conn.adoptDataTunnel = (next) => {
					held = next;
					adopted.push(next);
				};
				const close = conn.close.bind(conn);
				conn.close = () => {
					held?.close();
					close();
				};
			};
		});
		expect(
			await manager.openService(DEVICE, "placement", "hosting", {
				mode: "http",
			}),
		).toBe(stream);
		await env.advance(75_000);
		expect(env.conns).toHaveLength(2);
		expect(adopted).toEqual([owner]);
		expect(resets()).toBe(0);
		finish();
		await flush();
		await env.advance(LIVE_TIMING.lingerMs);
		expect(env.conns[1].closed).toBe(true);
		manager.dispose();
	});

	for (const stop of ["close", "lock"] as const)
		test(`service streams reset on explicit ${stop}`, async () => {
			const { stream, resets } = serviceStream();
			const { env, manager } = setup((env) => {
				env.configureConnection = (conn) => {
					conn.openService = async () => stream;
					const close = conn.close.bind(conn);
					conn.close = () => {
						stream.reset();
						close();
					};
				};
			});
			await manager.openService(DEVICE, "placement");
			if (stop === "close") manager.close(DEVICE);
			else env.setController(undefined);
			await stream.closed;
			await flush();
			expect(resets()).toBeGreaterThan(0);
			expect(env.conns).toHaveLength(1);
			expect(env.conns[0].closed).toBe(true);
			manager.dispose();
		});

	test("service completion releases demand after a late open is cancelled by close", async () => {
		const { stream, resets } = serviceStream();
		let opened!: (stream: DeviceServiceStream) => void;
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.openService = () =>
					new Promise((done) => {
						opened = done;
					});
			};
		});
		const pending = manager.openService(DEVICE, "placement");
		await flush();
		manager.close(DEVICE);
		opened(stream);
		await expect(pending).rejects.toThrow("session was closed");
		expect(resets()).toBe(1);
		await env.advance(LIVE_TIMING.lingerMs * 2);
		expect(env.conns).toHaveLength(1);
		manager.dispose();
	});

	test("a model gateway stream holds the session until it closes; an agent without model_host is never asked", async () => {
		const { stream, finish } = serviceStream();
		let opens = 0;
		let features: Record<string, number> = { task_health: 1 };
		const { env, manager } = setup((env) => {
			env.handler = (command, conn) =>
				command.type === "inspect_page"
					? completed({ ...inspectPage().result, features })
					: defaultHandler(command, conn);
			env.configureConnection = (conn) => {
				if (conn.serial > 1) return;
				conn.openModelGateway = async () => {
					opens++;
					return stream;
				};
			};
		});
		const release = manager.acquire(DEVICE, "view");
		await env.advance(0);
		const refused = await manager
			.openModelGateway(DEVICE)
			.catch((error: unknown) => error);
		expect(refused).toBeInstanceOf(LiveCallError);
		expect(opens).toBe(0);
		features = { task_health: 1, model_host: 1 };
		await manager.refreshInspection(DEVICE);
		expect(await manager.openModelGateway(DEVICE)).toBe(stream);
		expect(opens).toBe(1);
		release();
		await env.advance(LIVE_TIMING.lingerMs * 2);
		expect(env.conns[0].closed).toBe(false);
		finish();
		await flush();
		await env.advance(LIVE_TIMING.lingerMs);
		expect(env.conns[0].closed).toBe(true);
		const older = await manager
			.openModelGateway(DEVICE)
			.catch((error: unknown) => error);
		expect(older).toBeInstanceOf(Error);
		expect((older as Error).message).toBe(
			"Update Studio to send requests to models on devices.",
		);
		manager.dispose();
	});

	test("a model asset push goes over the session and holds it until the device answered", async () => {
		const pushes: TunnelModelAssetPush[] = [];
		let answer: (status: ModelAssetStatus) => void = () => {};
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				if (conn.serial > 1) return;
				conn.pushModelAsset = (input) => {
					pushes.push(input);
					return new Promise((resolve) => {
						answer = resolve;
					});
				};
			};
		});
		const input = { jobId: "12345678-1234-4234-8234-123456789abc", offset: 0 };
		const pushing = manager.pushModelAsset(DEVICE, input);
		await env.advance(LIVE_TIMING.lingerMs * 2);
		expect(pushes).toEqual([input]);
		expect(env.conns[0].closed).toBe(false);
		const present: ModelAssetStatus = {
			digest: { algorithm: "sha256", hex: "a".repeat(64) },
			state: "present",
		};
		answer(present);
		expect(await pushing).toEqual(present);
		await env.advance(LIVE_TIMING.lingerMs);
		expect(env.conns[0].closed).toBe(true);
		const older = await manager
			.pushModelAsset(DEVICE, input)
			.catch((error: unknown) => error);
		expect((older as Error).message).toBe(
			"This device connection does not support streamed uploads.",
		);
		env.setController(undefined);
		const locked = await manager
			.pushModelAsset(DEVICE, input)
			.catch((error: unknown) => error);
		expect(locked).toBeInstanceOf(LiveCallError);
		expect(pushes).toHaveLength(1);
		manager.dispose();
	});

	test("service opening does not proceed after cancellation while connecting", async () => {
		const controller = new AbortController();
		let opens = 0;
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.openService = async () => {
					opens++;
					return serviceStream().stream;
				};
			};
		});
		const pending = manager.openService(DEVICE, "placement", "hosting", {
			signal: controller.signal,
		});
		controller.abort();
		await expect(pending).rejects.toThrow();
		expect(opens).toBe(0);
		await env.advance(LIVE_TIMING.lingerMs);
		expect(env.conns.every((conn) => conn.closed)).toBe(true);
		manager.dispose();
	});

	test("hands the bulk session to the replacement connection while an upload runs", async () => {
		let ended = false;
		let complete: (value: ArtifactTransferStatus) => void = () => {};
		const transfer = new Promise<ArtifactTransferStatus>((resolve) => {
			complete = resolve;
		});
		const owner = {
			close() {
				ended = true;
			},
		} as DeviceTunnelDataClient;
		const { env, manager } = setup((env) => {
			env.outcomes = [120, 300];
			env.configureConnection = (conn) => {
				let held: DeviceTunnelDataClient | undefined =
					conn.serial === 1 ? owner : undefined;
				conn.uploadArtifact = () => transfer;
				conn.detachDataTunnel = () => {
					const previous = held;
					held = undefined;
					return previous;
				};
				conn.adoptDataTunnel = (next) => {
					held = next;
				};
				const close = conn.close.bind(conn);
				conn.close = () => {
					held?.close();
					close();
				};
			};
		});
		const uploading = manager.uploadArtifact(DEVICE, {
			projectId: "project",
			transferId: "transfer",
			fileIndex: 0,
			offset: 0,
			file: new Blob(["data"]),
		});
		await env.advance(0);
		await manager.call(DEVICE)({ type: "stop", placement_id: "service" });
		expect(types(env)).toContain("stop");
		await env.advance(75_000);
		expect(env.conns).toHaveLength(2);
		expect(env.conns[0].closed).toBe(true);
		expect(ended).toBe(false);
		complete({ transfer_id: "transfer" } as ArtifactTransferStatus);
		await uploading;
		manager.close(DEVICE);
		expect(ended).toBe(true);
	});
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
	test("a failed management poll and transient reconnect failure leave the model answer running", async () => {
		const { env, manager } = setup();
		const model = modelTunnel(env);
		await manager.openModelGateway(DEVICE);
		await flush();
		env.outcomes = [
			new ConnectError("getting_pass", "http", "The hub is unavailable."),
			300,
		];
		env.handler = (command, conn) => {
			if (command.type === "models") {
				conn.drop();
				throw new ManagementUnconfirmedError("overview");
			}
			return defaultHandler(command, conn);
		};
		await expect(
			manager.call(DEVICE, { lane: "poll" })({
				type: "models",
				request: { kind: "overview" },
			}),
		).rejects.toThrow("The hub is unavailable.");
		expect(model.resets()).toBe(0);
		await env.advance(2_000);
		expect(model.adopted).toEqual([model.data]);
		expect(model.resets()).toBe(0);
		manager.close(DEVICE);
		expect(model.resets()).toBe(1);
		manager.dispose();
	});

	for (const failure of [
		new ConnectError("getting_pass", "access_expired", "Access ended."),
		new ConnectError(
			"getting_pass",
			"epoch_mismatch",
			"Device identity changed.",
		),
		new ConnectError(
			"securing",
			"identity_confirmation_failed",
			"Identity failed.",
		),
	])
		test(`a reconnect refused with ${failure.code} closes the retained model tunnel`, async () => {
			const { env, manager } = setup();
			const model = modelTunnel(env);
			await manager.openModelGateway(DEVICE);
			await flush();
			env.outcomes = [failure];
			env.conns[0].drop();
			expect(model.resets()).toBe(0);
			await env.advance(1_000);
			expect(model.resets()).toBe(1);
			expect(manager.state(DEVICE).kind).toBe("failed");
			manager.dispose();
		});

	for (const stop of ["close", "lock", "dispose", "finish"] as const)
		test(`a retained model tunnel closes on ${stop} before management reconnects`, async () => {
			const { env, manager } = setup();
			const model = modelTunnel(env);
			await manager.openModelGateway(DEVICE);
			await flush();
			env.conns[0].drop();
			expect(model.resets()).toBe(0);
			if (stop === "close") manager.close(DEVICE);
			else if (stop === "lock") env.setController(undefined);
			else if (stop === "dispose") manager.dispose();
			else model.finish();
			await flush();
			expect(model.resets()).toBe(1);
			await env.advance(2_000);
			expect(env.conns).toHaveLength(1);
			manager.dispose();
		});

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
	test("exclusive sections wait for active data reads while ordinary controls can proceed", async () => {
		let release: () => void = () => {};
		const blocked = new Promise<void>((resolve) => {
			release = resolve;
		});
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async (command) => {
					if (command.type === "inspect_page") return inspectPage();
					await blocked;
					return completed();
				};
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		const read = manager.call(DEVICE)({ type: "logs" });
		await flush();
		let entered = false;
		const exclusive = manager.exclusive(DEVICE, async () => {
			entered = true;
		});
		await manager.call(DEVICE)({ type: "metrics" });
		expect(entered).toBe(false);
		release();
		await read;
		await exclusive;
		expect(entered).toBe(true);
		manager.dispose();
	});
	test("bulk reads leave the control lane available and cap concurrent data requests", async () => {
		let release: () => void = () => {};
		const blocked = new Promise<void>((resolve) => {
			release = resolve;
		});
		let running = 0;
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async (command) => {
					if (command.type === "inspect_page") return inspectPage();
					running++;
					await blocked;
					return completed({ lines: [] });
				};
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		const reads = Array.from({ length: 9 }, (_, index) =>
			manager.call(DEVICE)({ type: "logs", placement_id: `service-${index}` }),
		);
		await flush();
		expect(running).toBe(8);
		await manager.call(DEVICE)({ type: "stop", placement_id: "service" });
		expect(types(env)).toContain("stop");
		expect(running).toBe(8);
		release();
		await Promise.all(reads);
		expect(running).toBe(9);
		manager.dispose();
	});
	test("model statistics take the data stream; other models commands the management connection", async () => {
		const bulk: Record<string, unknown>[] = [];
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async (command) => {
					if (command.type === "inspect_page") return inspectPage();
					bulk.push(command);
					return completed();
				};
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		const stats = {
			type: "models",
			request: { kind: "stats", from: 3_600, to: 7_200, step: "hour" },
		};
		const overview = { type: "models", request: { kind: "overview" } };
		await manager.call(DEVICE, { lane: "poll" })(stats);
		await manager.call(DEVICE, { lane: "poll" })(overview);
		expect(bulk).toEqual([stats]);
		expect(env.sent.map((row) => row.command)).toEqual([overview]);
		manager.dispose();
	});
	test("a models read is retried once after a lost reply; a models write never is", async () => {
		const { env, manager } = setup();
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		const lost = new Set<string>();
		const kindOf = (command: Record<string, unknown>) =>
			String((command.request as { kind?: unknown } | undefined)?.kind);
		env.handler = (command, conn, operationId) => {
			const kind = kindOf(command);
			if (command.type !== "models" || lost.has(kind))
				return defaultHandler(command, conn);
			lost.add(kind);
			conn.close();
			throw new ManagementUnconfirmedError(operationId ?? kind);
		};
		const read = await manager.call(DEVICE)({
			type: "models",
			request: { kind: "jobs", after: null, limit: 16 },
		});
		expect(read.state).toBe("completed");
		const write = await manager
			.call(DEVICE)({
				type: "models",
				request: { kind: "unload", model_id: "qwen" },
			})
			.catch((error: unknown) => error);
		expect(write).toBeInstanceOf(ManagementUnconfirmedError);
		const sent = env.sent.filter((row) => row.command.type === "models");
		expect(sent.map((row) => kindOf(row.command))).toEqual([
			"jobs",
			"jobs",
			"unload",
		]);
		manager.dispose();
	});
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
	test("an upload waiting for its session cannot start after an explicit close", async () => {
		let uploads = 0;
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.uploadArtifact = async () => {
					uploads++;
					return {} as ArtifactTransferStatus;
				};
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		const uploading = manager
			.uploadArtifact(DEVICE, {
				projectId: "project",
				transferId: "transfer",
				fileIndex: 0,
				offset: 0,
				file: new Blob(),
			})
			.catch((error) => error);
		manager.close(DEVICE);
		expect(await uploading).toBeInstanceOf(LiveCallError);
		expect(uploads).toBe(0);
		manager.dispose();
	});
	test("explicit close does not reconnect to retry an active data read", async () => {
		let rejectRead: (error: Error) => void = () => {};
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async (command) => {
					if (command.type === "inspect_page") return inspectPage();
					return new Promise((_, reject) => {
						rejectRead = reject;
					});
				};
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		const read = manager
			.call(DEVICE)({ type: "logs" })
			.catch((error) => error);
		await flush();
		manager.close(DEVICE);
		rejectRead(new ManagementUnconfirmedError("read"));
		expect(await read).toBeInstanceOf(LiveCallError);
		await env.advance(60_000);
		expect(env.conns).toHaveLength(1);
		expect(manager.state(DEVICE).kind).toBe("idle");
		manager.dispose();
	});
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
	test("a stalled service read times out, cancels its request and can be refreshed", async () => {
		let blocked = true;
		let readSignal: AbortSignal | undefined;
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async (_command, _operationId, signal) => {
					if (!blocked) return inspectPage();
					readSignal = signal;
					return new Promise((_, reject) => {
						signal?.addEventListener("abort", () => reject(signal.reason), {
							once: true,
						});
					});
				};
			};
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(0);
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("active");
		await env.advance(LIVE_TIMING.inspectionPageMs);
		expect(manager.steps(DEVICE).at(-1)).toEqual({
			id: "reading_services",
			state: "failed",
			detail: { code: "timeout" },
		});
		expect(readSignal?.aborted).toBe(true);
		expect(manager.inspection(DEVICE)).toBeUndefined();
		blocked = false;
		await manager.refreshInspection(DEVICE);
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("done");
		expect(manager.inspection(DEVICE)?.value.device_id).toBe(DEVICE);
		manager.dispose();
	});

	test("the service-read deadline includes waiting behind an exclusive operation", async () => {
		const { env, manager } = setup();
		let finish!: () => void;
		const exclusive = manager.exclusive(
			DEVICE,
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		manager.acquire(DEVICE, "stream");
		await env.advance(LIVE_TIMING.inspectionPageMs);
		expect(manager.steps(DEVICE).at(-1)).toEqual({
			id: "reading_services",
			state: "failed",
			detail: { code: "timeout" },
		});
		expect(types(env)).toEqual([]);
		finish();
		await exclusive;
		await flush();
		await manager.refreshInspection(DEVICE);
		expect(types(env)).toEqual(["inspect_page"]);
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("done");
		manager.dispose();
	});

	test("each completed service page renews the inspection deadline", async () => {
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
		const { env, manager } = setup((env) => {
			env.handler = (command) =>
				new Promise((resolve) => {
					env.schedule(
						() =>
							resolve(
								command.after === null
									? completed({
											device_id: DEVICE,
											boot_id: "boot-1",
											placements: [row],
											next: row.id,
										})
									: inspectPage(),
							),
						100_000,
					);
				});
		});
		manager.acquire(DEVICE, "stream");
		await env.advance(LIVE_TIMING.inspectionPageMs);
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("active");
		await env.advance(200_000 - LIVE_TIMING.inspectionPageMs);
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("done");
		expect(manager.inspection(DEVICE)?.value.placements).toEqual([row]);
		manager.dispose();
		expect(env.pendingTimers).toBe(0);
	});

	test("closing a stalled inspection releases refresh and ignores its late reply", async () => {
		let reply!: (response: ManagementResponse) => void;
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async () =>
					conn.serial === 1
						? new Promise((resolve) => {
								reply = resolve;
							})
						: inspectPage();
			};
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		let settled = false;
		const refreshing = manager.refreshInspection(DEVICE).then(() => {
			settled = true;
		});
		manager.close(DEVICE);
		await flush();
		expect(settled).toBe(true);
		await refreshing;
		await manager.refreshInspection(DEVICE);
		expect(manager.inspection(DEVICE)?.value.device_id).toBe(DEVICE);
		const inspection = manager.inspection(DEVICE);
		reply(inspectPage());
		await flush();
		expect(manager.inspection(DEVICE)).toBe(inspection);
		expect(manager.steps(DEVICE).at(-1)?.state).toBe("done");
		manager.dispose();
	});

	test("a reply completing in the lock turn cannot repopulate the cleared inspection", async () => {
		let waiting = false;
		let reply: (value: ManagementResponse) => void = () => {};
		const { env, manager } = setup((env) => {
			env.configureConnection = (conn) => {
				conn.requestData = async () =>
					waiting
						? new Promise((resolve) => {
								reply = resolve;
							})
						: inspectPage();
			};
		});
		manager.acquire(DEVICE, "view");
		await env.advance(0);
		expect(env.inspected).toHaveLength(1);
		waiting = true;
		const refreshing = manager.refreshInspection(DEVICE);
		await flush();
		reply(inspectPage());
		env.setController(undefined);
		await refreshing;
		expect(manager.inspection(DEVICE)).toBeUndefined();
		expect(env.inspected).toHaveLength(1);
		manager.dispose();
	});
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
