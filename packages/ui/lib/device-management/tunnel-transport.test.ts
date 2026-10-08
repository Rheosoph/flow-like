import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { base64url, unbase64url } from "./crypto";
import {
	TunnelKind,
	decodeTunnelEnvelope,
	encodeTunnelEnvelope,
	encodeTunnelFrame,
	tunnelJson,
} from "./tunnel-protocol";
import {
	type TunnelTransport,
	connectTunnelTransport,
} from "./tunnel-transport";
import type {
	BrowserController,
	DeviceReceipt,
	SignalingAdmission,
} from "./types";

const DEVICE = "device-1";
const now = () => Math.floor(Date.now() / 1000);
const globals = globalThis as unknown as Record<string, unknown>;
let saved: { socket: unknown; rtc: unknown };
let participant = "";
let admissionExpiry = 0;
let admitCount = 0;
let opens: Socket[] = [];
let readyKind = TunnelKind.Renewed;
let noiseCount = 0;
let relayHellos: string[] = [];
let rtcMode:
	| "ready"
	| "closed"
	| "confirmation_closed"
	| "timeout"
	| "wrong_route"
	| "invalid_frame" = "ready";
let peers: Peer[] = [];
let onRtcHello: (() => void) | undefined;
let closedSessions: string[] = [];
let freedSessions: string[] = [];
const active: TunnelTransport[] = [];

class Channel extends EventTarget {
	readyState = "open";
	bufferedAmount = 0;
	binaryType = "arraybuffer";
	onclose?: () => void;
	onerror?: () => void;
	onmessage?: (event: { data: ArrayBuffer | string }) => void;
	sent: ReturnType<typeof decodeTunnelEnvelope>[] = [];

	send(bytes: Uint8Array) {
		const envelope = decodeTunnelEnvelope(bytes);
		this.sent.push(envelope);
		if (envelope.kind === "hello") onRtcHello?.();
		queueMicrotask(() => {
			if (this.readyState !== "open") return;
			if (envelope.kind === "hello") {
				if (rtcMode === "closed") return this.close();
				if (rtcMode === "timeout") return;
				if (rtcMode === "invalid_frame") {
					this.onmessage?.({ data: "invalid binary frame" });
					return;
				}
				this.onmessage?.({
					data: encodeTunnelEnvelope(
						"handshake",
						rtcMode === "wrong_route" ? "another-session" : envelope.sessionId,
						new Uint8Array([2]),
					).buffer as ArrayBuffer,
				});
			}
			if (envelope.kind === "handshake") {
				if (rtcMode === "confirmation_closed") return this.close();
				this.onmessage?.({
					data: encodeTunnelEnvelope(
						"message",
						envelope.sessionId,
						encodeTunnelFrame({
							kind: readyKind,
							stream: 0,
							sequence: 0n,
							body:
								readyKind === TunnelKind.Ping
									? new Uint8Array(8)
									: tunnelJson({ expires_at: now() + 300 }),
						}),
					).buffer as ArrayBuffer,
				});
			}
		});
	}

	close() {
		this.readyState = "closed";
		this.onclose?.();
	}
}

class Peer extends EventTarget {
	iceGatheringState = "complete";
	localDescription = { type: "offer", sdp: "offer" };
	channel = new Channel();
	closed = false;
	constructor() {
		super();
		peers.push(this);
	}
	createDataChannel() {
		return this.channel;
	}
	async createOffer() {
		return this.localDescription;
	}
	async setLocalDescription() {}
	async setRemoteDescription() {}
	close() {
		this.closed = true;
	}
}

class Socket {
	static OPEN = 1;
	readyState = 1;
	bufferedAmount = 0;
	protocol: string;
	onopen?: () => void;
	onclose?: () => void;
	onmessage?: (event: { data: string }) => void;
	onerror?: () => void;
	reauthorizations: string[] = [];
	constructor(_url: string, protocols: string[]) {
		this.protocol = protocols[0];
		opens.push(this);
		queueMicrotask(() => {
			this.onopen?.();
			this.deliver({
				type: "ready",
				participant_id: participant,
				role: "controller",
				expires_at: admissionExpiry,
			});
		});
	}
	deliver(value: unknown) {
		queueMicrotask(() => this.onmessage?.({ data: JSON.stringify(value) }));
	}
	frame(bytes: Uint8Array) {
		this.deliver({
			type: "frame",
			from: DEVICE,
			from_role: "device",
			to: participant,
			channel: "tunnel",
			payload: base64url(bytes),
		});
	}
	send(text: string) {
		const frame = JSON.parse(text);
		if (frame.type === "reauthorize") {
			this.reauthorizations.push(frame.token);
			this.deliver({ type: "reauthorized", expires_at: admissionExpiry });
			return;
		}
		if (frame.type === "ping") {
			this.deliver({ type: "pong" });
			return;
		}
		if (frame.type !== "frame") return;
		if (frame.channel === "signal") {
			const offer = JSON.parse(
				new TextDecoder().decode(unbase64url(frame.payload)),
			);
			this.deliver({
				type: "frame",
				from: DEVICE,
				from_role: "device",
				to: participant,
				channel: "signal",
				payload: base64url(
					new TextEncoder().encode(
						JSON.stringify({
							kind: "answer",
							session_id: offer.session_id,
							sdp: "answer",
						}),
					),
				),
			});
			return;
		}
		expect(frame.channel).toBe("tunnel");
		const envelope = decodeTunnelEnvelope(unbase64url(frame.payload));
		if (envelope.kind === "hello") {
			relayHellos.push(envelope.sessionId);
			this.frame(
				encodeTunnelEnvelope(
					"handshake",
					envelope.sessionId,
					new Uint8Array([2]),
				),
			);
		}
		if (envelope.kind === "handshake")
			this.frame(
				encodeTunnelEnvelope(
					"message",
					envelope.sessionId,
					encodeTunnelFrame({
						kind: readyKind,
						stream: 0,
						sequence: 0n,
						body: tunnelJson({ expires_at: now() + 300 }),
					}),
				),
			);
	}
	close() {
		this.readyState = 3;
		this.onclose?.();
	}
}

function admission(): SignalingAdmission {
	admissionExpiry = now() + (admitCount++ === 0 ? 120 : 300);
	return {
		token: `token-${admitCount}`,
		expires_at: admissionExpiry,
		device_auth_epoch: 1,
		signaling_urls: ["wss://relay.test/ws/devices"],
		ice_servers: [],
		ice_expires_at: null,
		policy_version: 1,
		policy_digest: null,
	};
}
const controller = {
	beginTunnelNoise() {
		const sessionId = `session-${++noiseCount}`;
		return {
			certificate: () => "signed-certificate",
			sessionId: () => sessionId,
			write: () => new Uint8Array([1]),
			read() {},
			finish: () => ({
				encrypt: (bytes: Uint8Array) => bytes.slice(),
				decrypt: (bytes: Uint8Array) => bytes.slice(),
				close() {
					closedSessions.push(sessionId);
				},
				free() {
					freedSessions.push(sessionId);
				},
			}),
			close() {},
			free() {},
		};
	},
} as unknown as BrowserController;
const receipt = {
	device_id: DEVICE,
	auth_epoch: 1,
	identity: { management_key: [1, 2, 3] },
} as unknown as DeviceReceipt;
function options() {
	return {
		api: {
			async fetch(_profile: IProfile, _path: string, init: RequestInit) {
				const incoming = JSON.parse(String(init.body)).participant_id;
				if (participant) expect(incoming).toBe(participant);
				participant = incoming;
				return admission();
			},
		} as unknown as IApiState,
		profile: {} as IProfile,
		controller,
		receipt,
	};
}
beforeEach(() => {
	saved = { socket: globals.WebSocket, rtc: globals.RTCPeerConnection };
	globals.WebSocket = Socket;
	globals.RTCPeerConnection = undefined;
	participant = "";
	admitCount = 0;
	opens = [];
	readyKind = TunnelKind.Renewed;
	noiseCount = 0;
	relayHellos = [];
	peers = [];
	rtcMode = "ready";
	onRtcHello = undefined;
	closedSessions = [];
	freedSessions = [];
});
afterEach(() => {
	for (const transport of active.splice(0)) transport.close();
	globals.WebSocket = saved.socket;
	globals.RTCPeerConnection = saved.rtc;
});

describe("encrypted tunnel transport", () => {
	test("keeps WebRTC after encrypted tunnel confirmation", async () => {
		globals.RTCPeerConnection = Peer;
		const transport = await connectTunnelTransport(options());
		active.push(transport);
		expect(transport.kind).toBe("webrtc");
		expect(noiseCount).toBe(1);
		expect(relayHellos).toEqual([]);
	});

	for (const mode of ["closed", "confirmation_closed", "timeout"] as const) {
		test(`retries an RTC ${mode} during initial authentication with fresh relay keys`, async () => {
			globals.RTCPeerConnection = Peer;
			rtcMode = mode;
			const original = globalThis.setTimeout;
			globalThis.setTimeout = ((callback: () => void, delay: number) =>
				original(callback, delay === 45_000 ? 1 : delay)) as typeof setTimeout;
			try {
				const transport = await connectTunnelTransport(options());
				active.push(transport);
				expect(transport.kind).toBe("websocket");
				expect(noiseCount).toBe(2);
				expect(relayHellos).toEqual(["session-2"]);
				expect(peers[0].channel.sent[0].sessionId).toBe("session-1");
				expect(
					peers[0].channel.sent.every((frame) => frame.kind !== "message"),
				).toBe(true);
				expect(peers[0].closed).toBe(true);
				expect(opens).toHaveLength(1);
				if (mode === "confirmation_closed") {
					expect(closedSessions).toEqual(["session-1"]);
					expect(freedSessions).toEqual(["session-1"]);
				}
			} finally {
				globalThis.setTimeout = original;
			}
		});
	}

	for (const mode of ["wrong_route", "invalid_frame"] as const) {
		test(`does not retry an RTC ${mode} through the relay`, async () => {
			globals.RTCPeerConnection = Peer;
			rtcMode = mode;
			await expect(connectTunnelTransport(options())).rejects.toThrow();
			expect(noiseCount).toBe(1);
			expect(relayHellos).toEqual([]);
			expect(peers[0].closed).toBe(true);
			expect(opens[0].readyState).toBe(3);
		});
	}

	test("does not retry rejected encrypted confirmation through the relay", async () => {
		globals.RTCPeerConnection = Peer;
		readyKind = TunnelKind.Ping;
		await expect(connectTunnelTransport(options())).rejects.toThrow(
			"did not confirm",
		);
		expect(noiseCount).toBe(1);
		expect(relayHellos).toEqual([]);
	});

	test("does not retry a pinned device identity failure through the relay", async () => {
		globals.RTCPeerConnection = Peer;
		const untrusted = {
			beginTunnelNoise() {
				return {
					...controller.beginTunnelNoise?.("owner", new Uint8Array(32), now()),
					read() {
						throw new Error("Pinned device identity mismatch.");
					},
				};
			},
		} as unknown as BrowserController;
		await expect(
			connectTunnelTransport({ ...options(), controller: untrusted }),
		).rejects.toThrow("Pinned device identity mismatch");
		expect(noiseCount).toBe(1);
		expect(relayHellos).toEqual([]);
	});

	test("does not replay an established RTC tunnel through the relay", async () => {
		globals.RTCPeerConnection = Peer;
		const transport = await connectTunnelTransport(options());
		active.push(transport);
		const received = transport.next();
		peers[0].channel.close();
		await expect(received).rejects.toThrow();
		expect(noiseCount).toBe(1);
		expect(relayHellos).toEqual([]);
	});

	test("does not retry an aborted RTC handshake through the relay", async () => {
		globals.RTCPeerConnection = Peer;
		const signal = new AbortController();
		onRtcHello = () => signal.abort();
		await expect(
			connectTunnelTransport({ ...options(), signal: signal.signal }),
		).rejects.toThrow();
		expect(noiseCount).toBe(1);
		expect(relayHellos).toEqual([]);
		expect(opens[0].readyState).toBe(3);
	});

	test("uses a binary relay handshake with a distinct tunnel Noise session", async () => {
		const transport = await connectTunnelTransport(options());
		active.push(transport);
		expect(transport.kind).toBe("websocket");
		expect(transport.expiresAt).toBe(now() + 300);
		expect(opens).toHaveLength(1);
		expect(opens[0].protocol).toBe("flowlike.device-management.v1");
		const data = encodeTunnelFrame({
			kind: TunnelKind.Ping,
			stream: 0,
			sequence: 0n,
			body: new Uint8Array(8),
		});
		await transport.send(data);
	});
	test("renews relay admission in place using the same participant", async () => {
		const original = globalThis.setTimeout;
		let renew: (() => void) | undefined;
		globalThis.setTimeout = ((callback: () => void, delay: number) => {
			if (delay === 60_000) renew = callback;
			return original(callback, delay);
		}) as typeof setTimeout;
		try {
			const transport = await connectTunnelTransport(options());
			active.push(transport);
			expect(renew).toBeFunction();
			renew?.();
			for (let index = 0; index < 20; index++) await Promise.resolve();
			expect(opens).toHaveLength(1);
			expect(opens[0].reauthorizations).toEqual(["token-2"]);
			expect(opens[0].readyState).toBe(Socket.OPEN);
		} finally {
			globalThis.setTimeout = original;
		}
	});
	test("refuses an outdated crypto bundle before requesting network access", async () => {
		await expect(
			connectTunnelTransport({
				...options(),
				controller: {} as BrowserController,
			}),
		).rejects.toThrow("Update Studio");
		expect(opens).toHaveLength(0);
	});
	test("a dropped relay rejects the pending receiver", async () => {
		const transport = await connectTunnelTransport(options());
		active.push(transport);
		const received = transport.next().catch((error) => error);
		opens[0].close();
		expect(await received).toBeInstanceOf(Error);
		await expect(transport.send(new Uint8Array(20))).rejects.toThrow();
	});
	test("cleans up transport when the consuming WASM handshake finish throws", async () => {
		const failing = {
			beginTunnelNoise() {
				let consumed = false;
				return {
					...controller.beginTunnelNoise?.("owner", new Uint8Array(32), now()),
					finish() {
						consumed = true;
						throw new Error("Handshake expired while finishing.");
					},
					close() {
						if (consumed)
							throw new Error("Cannot close a consumed WASM handle.");
					},
					free() {
						if (consumed)
							throw new Error("Cannot free a consumed WASM handle.");
					},
				};
			},
		} as unknown as BrowserController;
		await expect(
			connectTunnelTransport({ ...options(), controller: failing }),
		).rejects.toThrow("expired while finishing");
		expect(opens[0].readyState).toBe(3);
	});
});
