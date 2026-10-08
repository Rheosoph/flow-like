import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { base64url, unbase64url } from "./crypto";
import type { ModelAssetStatus } from "./models";
import {
	ConnectError,
	type ConnectProgress,
	DeviceManagementConnection,
	FrameQueue,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
} from "./transport";
import {
	DeviceTunnelDataClient,
	type TunnelModelAssetPush,
} from "./tunnel-data";
import type {
	BrowserController,
	DeviceReceipt,
	ManagementResponse,
	SignalingAdmission,
} from "./types";

const DEVICE = "device-1";
const PROTOCOL = "flowlike.device-management.v1";
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const nowS = () => Math.floor(Date.now() / 1000);

type DeviceScript = {
	/** Reply to the controller's direct connection offer. */
	offer?: (sessionId: string) => Record<string, unknown>;
	/** Reply to the controller's hello; defaults to a matching handshake envelope. */
	hello?: (sessionId: string) => Record<string, unknown>;
	ready?: () => Record<string, unknown>;
	message?: (
		request: Record<string, unknown>,
	) => Record<string, unknown> | null;
	/** Close the socket instead of sending the "ready" frame. */
	refuse?: boolean;
};

let script: DeviceScript = {};
let sockets: FakeSocket[] = [];
let commands: string[] = [];

class FakeSocket {
	static OPEN = 1;
	readyState = 0;
	protocol: string;
	bufferedAmount = 0;
	onopen: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onmessage: ((event: { data: string }) => void) | null = null;
	participant = "";
	constructor(
		readonly url: string,
		protocols: string[],
	) {
		this.protocol = protocols[0] ?? "";
		sockets.push(this);
		queueMicrotask(() => {
			if (script.refuse) {
				this.onclose?.();
				return;
			}
			this.readyState = FakeSocket.OPEN;
			this.onopen?.();
			this.deliver({
				type: "ready",
				participant_id: participants.at(-1),
				role: "controller",
				expires_at: admissionExpiry,
			});
		});
	}
	deliver(frame: Record<string, unknown>) {
		queueMicrotask(() => this.onmessage?.({ data: JSON.stringify(frame) }));
	}
	fromDevice(payload: Record<string, unknown>, channel = "noise") {
		this.deliver({
			type: "frame",
			to: participants.at(-1),
			from: DEVICE,
			from_role: "device",
			channel,
			payload: base64url(encoder.encode(JSON.stringify(payload))),
		});
	}
	send(text: string) {
		const frame = JSON.parse(text);
		if (frame.type !== "frame") return;
		const envelope = JSON.parse(decoder.decode(unbase64url(frame.payload)));
		if (envelope.kind === "offer" && script.offer)
			this.fromDevice(script.offer(envelope.session_id), "signal");
		if (envelope.kind === "hello")
			this.fromDevice(
				script.hello?.(envelope.session_id) ?? {
					kind: "handshake",
					session_id: envelope.session_id,
					data: base64url(new Uint8Array([1])),
				},
			);
		if (envelope.kind === "handshake")
			this.fromDevice({
				kind: "message",
				session_id: envelope.session_id,
				data: base64url(
					encoder.encode(
						JSON.stringify(
							script.ready?.() ?? {
								ready: true,
								device_id: DEVICE,
								expires_at: nowS() + 300,
								boot_id: "boot-1",
							},
						),
					),
				),
			});
		if (envelope.kind === "message") {
			const request = JSON.parse(decoder.decode(unbase64url(envelope.data)));
			commands.push(request.command.type);
			const response = script.message
				? script.message(request)
				: {
						operation_id: request.operation_id,
						state: "completed",
						result: { ok: true },
					};
			if (!response) return;
			this.fromDevice({
				kind: "message",
				session_id: envelope.session_id,
				data: base64url(encoder.encode(JSON.stringify(response))),
			});
		}
	}
	close() {
		this.readyState = 3;
		this.onclose?.();
	}
}

let participants: string[] = [];
let admissionExpiry = 0;

function admission(
	overrides: Partial<SignalingAdmission> = {},
): SignalingAdmission {
	admissionExpiry = nowS() + 300;
	return {
		token: "token",
		expires_at: admissionExpiry,
		device_auth_epoch: 3,
		signaling_urls: ["wss://hub.test/ws/devices"],
		ice_servers: [],
		ice_expires_at: null,
		policy_version: 1,
		policy_digest: null,
		...overrides,
	};
}

function api(answer: () => SignalingAdmission | Promise<never>): IApiState {
	return {
		fetch: async (_profile: IProfile, _path: string, init?: RequestInit) => {
			participants.push(JSON.parse(String(init?.body)).participant_id);
			return answer();
		},
	} as unknown as IApiState;
}

const receipt = {
	device_id: DEVICE,
	auth_epoch: 3,
	identity: { management_key: [1, 2, 3] },
} as unknown as DeviceReceipt;

const controller = {
	beginNoise: () => {
		let message = 0;
		return {
			certificate: () => "certificate",
			sessionId: () => `session-${++message}`,
			write: () => new Uint8Array([7]),
			read: () => {},
			finish: () => ({
				encrypt: (bytes: Uint8Array) => bytes.slice(),
				decrypt: (bytes: Uint8Array) => bytes.slice(),
				close: () => {},
				free: () => {},
			}),
			close: () => {},
			free: () => {},
		};
	},
} as unknown as BrowserController;

async function connect(
	answer: () => SignalingAdmission | Promise<never> = () => admission(),
) {
	const steps: ConnectProgress[] = [];
	const result = await DeviceManagementConnection.connect(
		api(answer),
		{} as IProfile,
		controller,
		receipt,
		"owner",
		undefined,
		{ onStep: (step) => steps.push(step) },
	).catch((error: unknown) => error);
	return { result, steps };
}

const globals = globalThis as unknown as Record<string, unknown>;
let saved: { WebSocket: unknown; RTCPeerConnection: unknown };

beforeEach(() => {
	script = {};
	sockets = [];
	commands = [];
	participants = [];
	saved = {
		WebSocket: globals.WebSocket,
		RTCPeerConnection: globals.RTCPeerConnection,
	};
	globals.WebSocket = FakeSocket;
	globals.RTCPeerConnection = undefined;
});
afterEach(() => {
	globals.WebSocket = saved.WebSocket;
	globals.RTCPeerConnection = saved.RTCPeerConnection;
});

describe("management frame queue", () => {
	for (const limit of ["frames", "bytes"] as const) {
		test(`keeps the ${limit} overflow rejection when the transport closes`, async () => {
			const queue = new FrameQueue<Uint8Array>(
				limit === "frames" ? 1 : 4,
				limit === "bytes" ? 1 : 4,
				(bytes) => bytes.length,
			);
			queue.push(new Uint8Array([1]));
			queue.push(new Uint8Array([2]));
			queue.close();
			await expect(queue.next()).rejects.toMatchObject({
				code: "invalid_reply",
				message: "Management input exceeded its bound.",
			});
		});
	}
});

describe("connect progress and fallback", () => {
	test("refuses service discovery and forwarding before sending to agents without service support", async () => {
		for (const feature of [undefined, 2, "1"]) {
			script.ready = () => ({
				ready: true,
				device_id: DEVICE,
				expires_at: nowS() + 300,
				boot_id: "boot-1",
				data_tunnel: 1,
				service_tunnel: feature,
			});
			const { result } = await connect();
			const connection = result as DeviceManagementConnection;
			const socketCount = sockets.length;
			await expect(
				connection.request({ type: "service_listeners", placement_id: "api" }),
			).rejects.toThrow("Update the device agent");
			expect(() => connection.openService("api", "hosting")).toThrow(
				"Update the device agent",
			);
			expect(sockets).toHaveLength(socketCount);
			expect(commands).toEqual([]);
			connection.close();
		}
	});
	test("requires an authenticated streaming capability before opening a bulk tunnel", async () => {
		for (const feature of [undefined, 2, "1"]) {
			script.ready = () => ({
				ready: true,
				device_id: DEVICE,
				expires_at: nowS() + 300,
				boot_id: "boot-1",
				data_tunnel: feature,
			});
			const { result } = await connect();
			const connection = result as DeviceManagementConnection;
			const socketCount = sockets.length;
			await expect(
				connection.request({ type: "inspect_page" }),
			).rejects.toThrow("Update the device agent");
			expect(sockets).toHaveLength(socketCount);
			expect(commands).toEqual([]);
			connection.close();
		}
	});
	test("routes bulk reads separately so an active read does not block a control request", async () => {
		script.ready = () => ({
			ready: true,
			device_id: DEVICE,
			expires_at: nowS() + 300,
			boot_id: "boot-1",
			data_tunnel: 1,
		});
		const { result } = await connect();
		const connection = result as DeviceManagementConnection;
		let reply: (response: ManagementResponse) => void = () => {};
		class Bulk extends DeviceTunnelDataClient {
			calls: string[] = [];
			override async request(
				command: Record<string, unknown>,
			): Promise<ManagementResponse> {
				this.calls.push(String(command.type));
				return new Promise((resolve) => {
					reply = resolve;
				});
			}
		}
		const bulk = new Bulk({
			api: api(() => admission()),
			profile: {} as IProfile,
			controller,
			receipt,
			grantId: "owner",
		});
		connection.adoptDataTunnel(bulk);
		const reading = connection.request({ type: "logs" }, "logs-op");
		await Promise.resolve();
		expect(
			(await connection.request({ type: "restart" }, "restart-op")).state,
		).toBe("completed");
		expect(bulk.calls).toEqual(["logs"]);
		expect(commands).toEqual(["restart"]);
		reply({
			operation_id: "logs-op",
			state: "completed",
			result: { lines: [] },
		});
		await reading;
		connection.close();
	});
	test("routes model statistics over the data tunnel and other models commands over the management connection", async () => {
		script.ready = () => ({
			ready: true,
			device_id: DEVICE,
			expires_at: nowS() + 300,
			boot_id: "boot-1",
			data_tunnel: 1,
		});
		const { result } = await connect();
		const connection = result as DeviceManagementConnection;
		class Bulk extends DeviceTunnelDataClient {
			calls: Record<string, unknown>[] = [];
			override async request(
				command: Record<string, unknown>,
				operationId: string,
			): Promise<ManagementResponse> {
				this.calls.push(command);
				return { operation_id: operationId, state: "completed", result: {} };
			}
		}
		const bulk = new Bulk({
			api: api(() => admission()),
			profile: {} as IProfile,
			controller,
			receipt,
			grantId: "owner",
		});
		connection.adoptDataTunnel(bulk);
		const stats = { type: "models", request: { kind: "stats" } };
		await connection.request(stats, "stats-op");
		await connection.request(
			{ type: "models", request: { kind: "overview" } },
			"overview-op",
		);
		expect(bulk.calls).toEqual([stats]);
		expect(commands).toEqual(["models"]);
		connection.close();
	});
	test("sends model asset pushes over the data tunnel", async () => {
		script.ready = () => ({
			ready: true,
			device_id: DEVICE,
			expires_at: nowS() + 300,
			boot_id: "boot-1",
			data_tunnel: 1,
		});
		const { result } = await connect();
		const connection = result as DeviceManagementConnection;
		class Bulk extends DeviceTunnelDataClient {
			pushes: TunnelModelAssetPush[] = [];
			override async pushModelAsset(
				input: TunnelModelAssetPush,
			): Promise<ModelAssetStatus> {
				this.pushes.push(input);
				return {
					digest: { algorithm: "sha256", hex: "a".repeat(64) },
					job_id: input.jobId,
					state: "awaiting_push",
					bytes: 7,
				};
			}
		}
		const bulk = new Bulk({
			api: api(() => admission()),
			profile: {} as IProfile,
			controller,
			receipt,
			grantId: "owner",
		});
		connection.adoptDataTunnel(bulk);
		const probe = { jobId: "12345678-1234-4234-8234-123456789abc", offset: 0 };
		const status = await connection.pushModelAsset(probe);
		expect(status).toMatchObject({ state: "awaiting_push", bytes: 7 });
		expect(bulk.pushes).toEqual([probe]);
		expect(commands).toEqual([]);
		connection.close();
	});
	test("reports every step and why the session runs over the relay", async () => {
		const { result, steps } = await connect();
		expect(result).toBeInstanceOf(DeviceManagementConnection);
		const connection = result as DeviceManagementConnection;
		expect(connection.transport).toBe("websocket");
		expect(connection.fallbackReason).toBe("webrtc_unavailable");
		expect(connection.bootId).toBe("boot-1");
		expect(steps[1].admission?.expiresAt).toBe(admissionExpiry);
		expect(steps[1].admission?.receivedAtMs).toBeNumber();
		expect(
			steps.map(({ admission: _admission, ...progress }) => progress),
		).toEqual([
			{ step: "getting_pass", state: "active" },
			{ step: "getting_pass", state: "done" },
			{ step: "reaching_device", state: "active" },
			{ step: "reaching_device", state: "done" },
			{ step: "trying_direct", state: "active" },
			{
				step: "trying_direct",
				state: "skipped",
				fallbackReason: "webrtc_unavailable",
			},
			{ step: "securing", state: "active" },
			{ step: "securing", state: "done" },
		]);
		connection.close();
	});

	test("a failing direct connection names missing TURN servers or a WebRTC failure", async () => {
		class FailingPeer {
			createDataChannel() {
				return { close() {} };
			}
			createOffer() {
				return Promise.reject(new Error("offer failed"));
			}
			close() {}
		}
		globals.RTCPeerConnection = FailingPeer;
		const bare = await connect();
		expect((bare.result as DeviceManagementConnection).fallbackReason).toBe(
			"no_turn_servers",
		);
		const turn = await connect(() =>
			admission({
				ice_servers: [{ urls: ["turn:turn.test:3478"] }],
			}),
		);
		expect((turn.result as DeviceManagementConnection).fallbackReason).toBe(
			"webrtc_failed",
		);
	});

	test("connected ICE without an open data channel times out and starts a fresh relay handshake", async () => {
		class StalledChannel extends EventTarget {
			readyState = "connecting";
			onclose: (() => void) | null = null;
			close() {
				this.readyState = "closed";
				this.dispatchEvent(new Event("close"));
				this.onclose?.();
			}
		}
		const channel = new StalledChannel();
		let awaitingChannel = false;
		const peers: ConnectedPeer[] = [];
		class ConnectedPeer extends EventTarget {
			iceGatheringState = "complete";
			iceConnectionState = "new";
			connectionState = "new";
			localDescription = { type: "offer", sdp: "test offer" };
			constructor() {
				super();
				peers.push(this);
			}
			createDataChannel() {
				return channel;
			}
			async createOffer() {
				return this.localDescription;
			}
			async setLocalDescription() {}
			async setRemoteDescription() {
				this.iceConnectionState = "connected";
				this.connectionState = "connected";
				awaitingChannel = true;
			}
			close() {
				this.iceConnectionState = "closed";
				this.connectionState = "closed";
			}
		}
		globals.RTCPeerConnection = ConnectedPeer;
		const handshakes: { id: string; closed: boolean; freed: boolean }[] = [];
		const freshController = {
			beginNoise: () => {
				const state = {
					id: `attempt-${handshakes.length + 1}`,
					closed: false,
					freed: false,
				};
				handshakes.push(state);
				return {
					...controller.beginNoise("owner", new Uint8Array([1]), nowS()),
					sessionId: () => state.id,
					close: () => {
						state.closed = true;
					},
					free: () => {
						state.freed = true;
					},
				};
			},
		} as unknown as BrowserController;
		const offered: string[] = [];
		const authenticated: string[] = [];
		script.offer = (sessionId) => {
			offered.push(sessionId);
			return { kind: "answer", session_id: sessionId, sdp: "test answer" };
		};
		script.hello = (sessionId) => {
			authenticated.push(sessionId);
			return {
				kind: "handshake",
				session_id: sessionId,
				data: base64url(new Uint8Array([1])),
			};
		};
		type Deadline = { delay: number; fire: () => void };
		let captureDeadline: (deadline: Deadline) => void = () => {};
		const waitingForChannel = new Promise<Deadline>((resolve) => {
			captureDeadline = resolve;
		});
		const nativeSetTimeout = globalThis.setTimeout;
		globals.setTimeout = (callback: () => void, delay: number) => {
			const timer = nativeSetTimeout(callback, delay);
			if (awaitingChannel) {
				awaitingChannel = false;
				captureDeadline({ delay, fire: callback });
			}
			return timer;
		};
		let connection: DeviceManagementConnection | undefined;
		const steps: ConnectProgress[] = [];
		try {
			const connecting = DeviceManagementConnection.connect(
				api(() =>
					admission({ ice_servers: [{ urls: ["turn:turn.test:3478"] }] }),
				),
				{} as IProfile,
				freshController,
				receipt,
				"owner",
				undefined,
				{ onStep: (step) => steps.push(step) },
			);
			const deadline = await waitingForChannel;
			const peer = peers[0];
			expect(deadline.delay).toBe(15_000);
			expect(peer.iceConnectionState).toBe("connected");
			expect(peer.connectionState).toBe("connected");
			expect(channel.readyState).toBe("connecting");
			expect(authenticated).toEqual([]);
			expect(steps.at(-1)).toEqual({ step: "trying_direct", state: "active" });
			deadline.fire();
			connection = await connecting;
			expect(connection.transport).toBe("websocket");
			expect(channel.readyState).toBe("closed");
			expect(peer.connectionState).toBe("closed");
			expect(sockets).toHaveLength(1);
			expect(offered).toEqual(["attempt-1"]);
			expect(authenticated).toEqual(["attempt-2"]);
			expect(handshakes).toEqual([
				{ id: "attempt-1", closed: true, freed: true },
				{ id: "attempt-2", closed: false, freed: false },
			]);
			expect(steps.at(-1)).toEqual({ step: "securing", state: "done" });
		} finally {
			globals.setTimeout = nativeSetTimeout;
			connection?.close();
		}
	});
});

describe("typed connect errors", () => {
	const httpError = (status: number, serverMessage: string) => () =>
		Promise.reject(
			Object.assign(new Error(`[HTTP_${status}] ${serverMessage}`), {
				status,
				serverMessage,
			}),
		);

	test("admission failures map to the connection pass step", async () => {
		const cases: [
			() => SignalingAdmission | Promise<never>,
			string,
			number?,
		][] = [
			[httpError(503, "Device signaling is not configured"), "not_configured"],
			[httpError(503, "Device signaling requires a WSS endpoint"), "needs_wss"],
			[httpError(401, "Unauthorized"), "access_expired"],
			[httpError(403, "Forbidden"), "access_expired"],
			[httpError(500, "Internal"), "http", 500],
			[() => admission({ device_auth_epoch: 9 }), "epoch_mismatch"],
			[() => admission({ signaling_urls: [] }), "invalid_admission"],
		];
		for (const [answer, code, status] of cases) {
			const { result, steps } = await connect(answer);
			expect(result).toBeInstanceOf(ConnectError);
			const error = result as ConnectError;
			expect<unknown>([error.step, error.code]).toEqual(["getting_pass", code]);
			if (status) expect(error.detail.status).toBe(status);
			expect(steps.at(-1)).toEqual({ step: "getting_pass", state: "failed" });
		}
		const { result } = await connect(() => admission({ device_auth_epoch: 9 }));
		expect((result as Error).message).toBe(
			"Invalid device signaling admission.",
		);
	});

	test("an unreachable relay names the step and the endpoint", async () => {
		script.refuse = true;
		const { result, steps } = await connect();
		const error = result as ConnectError;
		expect([error.step, error.code]).toEqual([
			"reaching_device",
			"relay_unreachable",
		]);
		expect(error.detail.url).toBe("wss://hub.test/ws/devices");
		expect(error.message).toBe("Device signaling could not be reached.");
		expect(steps.at(-1)).toEqual({ step: "reaching_device", state: "failed" });
	});

	test("handshake and identity failures map to the securing step", async () => {
		script.hello = (sessionId) => ({
			kind: "message",
			session_id: sessionId,
			data: "",
		});
		const handshake = (await connect()).result as ConnectError;
		expect([handshake.step, handshake.code, handshake.message]).toEqual([
			"securing",
			"handshake_failed",
			"Unexpected Noise handshake.",
		]);
		expect(handshake.diagnostic).toEqual({
			transport: "websocket",
			phase: "securing",
			cause: "handshake_failed",
			fallbackReason: "webrtc_unavailable",
		});
		script = {
			ready: () => ({
				ready: true,
				device_id: "other",
				expires_at: nowS() + 300,
				boot_id: "boot",
			}),
		};
		const identity = (await connect()).result as ConnectError;
		expect([identity.step, identity.code]).toEqual([
			"securing",
			"identity_confirmation_failed",
		]);
	});
});

describe("close notifications", () => {
	async function connectionWithData() {
		script.ready = () => ({
			ready: true,
			device_id: DEVICE,
			expires_at: nowS() + 300,
			boot_id: "boot-1",
			data_tunnel: 1,
		});
		const connection = (await connect()).result as DeviceManagementConnection;
		class Data extends DeviceTunnelDataClient {
			closes = 0;
			override close() {
				this.closes++;
				super.close();
			}
		}
		const data = new Data({
			api: api(() => admission()),
			profile: {} as IProfile,
			controller,
			receipt,
			grantId: "owner",
		});
		connection.adoptDataTunnel(data);
		return { connection, data };
	}

	test("a management timeout lets the workspace retain its independent data tunnel", async () => {
		const { connection, data } = await connectionWithData();
		script.message = () => null;
		const reasons: string[] = [];
		let detached: DeviceTunnelDataClient | undefined;
		connection.onClosed((reason) => {
			reasons.push(reason);
			if (reason === "remote") detached = connection.detachDataTunnel();
		});
		const nativeSetTimeout = globalThis.setTimeout;
		let expire: (() => void) | undefined;
		globals.setTimeout = (callback: () => void, delay: number) => {
			expect(delay).toBe(15_000);
			const timer = nativeSetTimeout(callback, delay);
			expire = () => {
				clearTimeout(timer);
				callback();
			};
			return timer;
		};
		try {
			const polling = connection
				.request({ type: "models", request: { kind: "overview" } })
				.catch((error: unknown) => error);
			expect(expire).toBeDefined();
			expire?.();
			const error = await polling;
			expect(error).toBeInstanceOf(ManagementUnconfirmedError);
			expect((error as ManagementUnconfirmedError).diagnostic).toMatchObject({
				phase: "wait_reply",
				cause: "timeout",
			});
			expect(reasons).toEqual(["remote"]);
			expect(detached).toBe(data);
			expect(data.closes).toBe(0);
			expect(connection.open).toBe(false);
		} finally {
			globals.setTimeout = nativeSetTimeout;
			connection.close();
			detached?.close();
		}
	});

	test("a transport send failure preserves data without claiming the management request was sent", async () => {
		const { connection, data } = await connectionWithData();
		script.message = () => {
			throw new Error("The management channel stopped accepting messages.");
		};
		let detached: DeviceTunnelDataClient | undefined;
		connection.onClosed((reason) => {
			if (reason === "remote") detached = connection.detachDataTunnel();
		});
		await expect(connection.request({ type: "models" })).rejects.toBeInstanceOf(
			ManagementRequestNotSentError,
		);
		expect(detached).toBe(data);
		expect(data.closes).toBe(0);
		detached?.close();
	});

	for (const stop of [
		"local",
		"remote",
		"invalid_reply",
		"listener_error",
	] as const)
		test(`closes unclaimed data after ${stop}`, async () => {
			const { connection, data } = await connectionWithData();
			if (stop === "local") connection.close();
			else if (stop === "invalid_reply") {
				const reasons: string[] = [];
				connection.onClosed((reason) => reasons.push(reason));
				script.message = () => ({ operation_id: "wrong" });
				await expect(
					connection.request({ type: "models" }),
				).rejects.toBeInstanceOf(ManagementUnconfirmedError);
				expect(reasons).toEqual(["local"]);
			} else if (stop === "listener_error") {
				connection.onClosed(() => {
					throw new Error("listener failed");
				});
				expect(() => sockets.at(-1)?.close()).toThrow("listener failed");
			} else sockets.at(-1)?.close();
			expect(data.closes).toBe(1);
			expect(connection.open).toBe(false);
			connection.close();
			expect(data.closes).toBe(1);
		});

	test("a dropped relay tells listeners once with a remote reason", async () => {
		const connection = (await connect()).result as DeviceManagementConnection;
		const reasons: string[] = [];
		connection.onClosed((reason) => reasons.push(reason));
		sockets.at(-1)?.close();
		expect(reasons).toEqual(["remote"]);
		expect(connection.open).toBe(false);
		connection.close();
		expect(reasons).toEqual(["remote"]);
	});

	test("a local close reports local, and late listeners still hear it", async () => {
		const connection = (await connect()).result as DeviceManagementConnection;
		const reasons: string[] = [];
		const stop = connection.onClosed((reason) => reasons.push(`a:${reason}`));
		connection.onClosed((reason) => reasons.push(`b:${reason}`));
		stop();
		connection.close();
		connection.onClosed((reason) => reasons.push(`late:${reason}`));
		await Promise.resolve();
		expect(reasons).toEqual(["b:local", "late:local"]);
	});

	test("the relay socket is opened with the management subprotocol", async () => {
		const connection = (await connect()).result as DeviceManagementConnection;
		expect(sockets.at(-1)?.protocol).toBe(PROTOCOL);
		connection.close();
	});
});
