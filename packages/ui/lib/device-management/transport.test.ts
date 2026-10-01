import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { base64url, unbase64url } from "./crypto";
import {
	ConnectError,
	type ConnectProgress,
	DeviceManagementConnection,
} from "./transport";
import type {
	BrowserController,
	DeviceReceipt,
	SignalingAdmission,
} from "./types";

const DEVICE = "device-1";
const PROTOCOL = "flowlike.device-management.v1";
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const nowS = () => Math.floor(Date.now() / 1000);

type DeviceScript = {
	/** Reply to the controller's hello; defaults to a matching handshake envelope. */
	hello?: (sessionId: string) => Record<string, unknown>;
	ready?: () => Record<string, unknown>;
	/** Close the socket instead of sending the "ready" frame. */
	refuse?: boolean;
};

let script: DeviceScript = {};
let sockets: FakeSocket[] = [];

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
	fromDevice(payload: Record<string, unknown>) {
		this.deliver({
			type: "frame",
			to: participants.at(-1),
			from: DEVICE,
			from_role: "device",
			channel: "noise",
			payload: base64url(encoder.encode(JSON.stringify(payload))),
		});
	}
	send(text: string) {
		const frame = JSON.parse(text);
		if (frame.type !== "frame") return;
		const envelope = JSON.parse(decoder.decode(unbase64url(frame.payload)));
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

describe("connect progress and fallback", () => {
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
