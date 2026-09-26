import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { base64url, unbase64url } from "./crypto";
import type {
	BrowserController,
	DeviceReceipt,
	ManagementResponse,
	NoiseHandshake,
	NoiseSession,
	SignalingAdmission,
} from "./types";

const PROTOCOL = "flowlike.device-management.v1";
const decoder = new TextDecoder("utf-8", { fatal: true });
const encoder = new TextEncoder();
const now = () => Math.floor(Date.now() / 1000);
type Envelope = {
	kind: "hello" | "handshake" | "message";
	session_id: string;
	data: string;
	grant_id?: string;
	certificate_jws?: string;
};
type Channel = "signal" | "noise";

export class FrameQueue<T> {
	private values: T[] = [];
	private waiting?: {
		resolve: (value: T) => void;
		reject: (error: Error) => void;
		timer: ReturnType<typeof setTimeout>;
	};
	private failure?: Error;
	push(value: T): void {
		if (this.failure) return;
		if (this.waiting) {
			const waiter = this.waiting;
			this.waiting = undefined;
			clearTimeout(waiter.timer);
			waiter.resolve(value);
			return;
		}
		if (this.values.length >= 32) {
			this.close(new Error("Management input exceeded its bound."));
			return;
		}
		this.values.push(value);
	}
	next(timeout = 15_000): Promise<T> {
		if (this.failure) return Promise.reject(this.failure);
		if (this.values.length)
			return Promise.resolve(this.values.splice(0, 1)[0] as T);
		if (this.waiting)
			return Promise.reject(
				new Error("Management frames require an ordered reader."),
			);
		return new Promise((resolve, reject) => {
			const timer = setTimeout(() => {
				this.waiting = undefined;
				reject(new Error("Management connection timed out."));
			}, timeout);
			this.waiting = { resolve, reject, timer };
		});
	}
	close(error = new Error("Management connection closed.")): void {
		this.failure = error;
		this.values = [];
		if (this.waiting) {
			clearTimeout(this.waiting.timer);
			this.waiting.reject(error);
			this.waiting = undefined;
		}
	}
}

export function signalingUrl(value: string): string {
	const url = new URL(value);
	if (
		url.protocol !== "wss:" ||
		url.username ||
		url.password ||
		url.search ||
		url.hash ||
		!url.pathname.endsWith("/ws/devices")
	)
		throw new Error("Invalid device signaling endpoint.");
	return url.href;
}

function object(value: unknown): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value))
		throw new Error("Invalid management frame.");
	return value as Record<string, unknown>;
}

function parseEnvelope(text: string): Envelope {
	if (encoder.encode(text).length > 32_768)
		throw new Error("Management frame exceeds its bound.");
	const value = object(JSON.parse(text));
	if (
		!["handshake", "message"].includes(String(value.kind)) ||
		typeof value.session_id !== "string" ||
		typeof value.data !== "string"
	)
		throw new Error("Invalid management envelope.");
	return value as unknown as Envelope;
}

class Relay {
	readonly noise = new FrameQueue<string>();
	readonly signal = new FrameQueue<string>();
	private readonly ready = new FrameQueue<Record<string, unknown>>();
	private readonly opened = new FrameQueue<void>();
	private readonly socket: WebSocket;
	private heartbeat?: ReturnType<typeof setInterval>;
	private lastPong = Date.now();
	private admitted = false;
	private closed = false;
	constructor(
		url: string,
		private readonly admission: SignalingAdmission,
		private readonly participant: string,
		private readonly deviceId: string,
	) {
		this.socket = new WebSocket(signalingUrl(url), [
			PROTOCOL,
			`flowlike.jwt.${admission.token}`,
		]);
		this.socket.onopen = () => this.opened.push();
		this.socket.onclose = () => this.close();
		this.socket.onerror = () => this.close();
		this.socket.onmessage = (event) => {
			try {
				if (
					typeof event.data !== "string" ||
					encoder.encode(event.data).length > 49_152
				)
					throw new Error("Invalid signaling frame.");
				const frame = object(JSON.parse(event.data));
				if (frame.type === "ready" && !this.admitted) {
					this.ready.push(frame);
					return;
				}
				if (frame.type === "pong") {
					this.lastPong = Date.now();
					return;
				}
				if (frame.type !== "frame")
					throw new Error("Unexpected signaling frame.");
				if (
					frame.to !== participant ||
					frame.from !== deviceId ||
					frame.from_role !== "device"
				)
					return;
				if (typeof frame.payload !== "string")
					throw new Error("Missing signaling payload.");
				const payload = decoder.decode(unbase64url(frame.payload));
				if (frame.channel === "noise") this.noise.push(payload);
				else if (frame.channel === "signal") this.signal.push(payload);
			} catch {
				this.close();
			}
		};
	}
	async connect(): Promise<void> {
		await this.opened.next();
		if (this.socket.protocol !== PROTOCOL)
			throw new Error("Device signaling protocol differs.");
		const frame = await this.ready.next();
		if (
			frame.participant_id !== this.participant ||
			frame.role !== "controller" ||
			frame.expires_at !== this.admission.expires_at ||
			this.admission.expires_at <= now()
		)
			throw new Error("Device signaling admission differs.");
		this.admitted = true;
		this.heartbeat = setInterval(() => {
			if (
				this.admission.expires_at <= now() ||
				Date.now() - this.lastPong > 60_000
			) {
				this.close();
				return;
			}
			try {
				this.socket.send('{"type":"ping"}');
			} catch {
				this.close();
			}
		}, 20_000);
	}
	send(channel: Channel, payload: unknown): void {
		const bytes = encoder.encode(JSON.stringify(payload));
		if (
			bytes.length > 32_768 ||
			this.closed ||
			this.socket.readyState !== WebSocket.OPEN ||
			this.socket.bufferedAmount > 98_304
		)
			throw new Error("Management connection is unavailable or busy.");
		this.socket.send(
			JSON.stringify({
				type: "frame",
				to: this.deviceId,
				channel,
				payload: base64url(bytes),
			}),
		);
	}
	close(): void {
		if (this.closed) return;
		this.closed = true;
		clearInterval(this.heartbeat);
		this.noise.close();
		this.signal.close();
		this.ready.close();
		this.opened.close();
		this.socket.onclose = null;
		this.socket.onmessage = null;
		this.socket.onerror = null;
		this.socket.close();
	}
}

interface Pipe {
	send(envelope: Envelope): void;
	next(): Promise<string>;
	close(): void;
	kind: "webrtc" | "websocket";
}

async function eventReady(
	target: EventTarget,
	event: string,
	ready: () => boolean,
): Promise<void> {
	if (ready()) return;
	await new Promise<void>((resolve, reject) => {
		const finish = () => {
			if (!ready()) return;
			cleanup();
			resolve();
		};
		const failed = () => {
			cleanup();
			reject(new Error("WebRTC connection failed."));
		};
		const timer = setTimeout(failed, 15_000);
		const cleanup = () => {
			clearTimeout(timer);
			target.removeEventListener(event, finish);
			target.removeEventListener("error", failed);
			target.removeEventListener("close", failed);
		};
		target.addEventListener(event, finish);
		target.addEventListener("error", failed);
		target.addEventListener("close", failed);
	});
}

async function rtcPipe(
	relay: Relay,
	handshake: NoiseHandshake,
	grantId: string,
	admission: SignalingAdmission,
): Promise<Pipe> {
	if (typeof RTCPeerConnection === "undefined")
		throw new Error("WebRTC is unavailable.");
	const iceServers =
		admission.ice_expires_at !== null && admission.ice_expires_at <= now() + 30
			? []
			: admission.ice_servers;
	if (iceServers.length > 16)
		throw new Error("Invalid device ICE configuration.");
	const peer = new RTCPeerConnection({ iceServers });
	const channel = peer.createDataChannel(PROTOCOL, {
		ordered: true,
		protocol: PROTOCOL,
	});
	const input = new FrameQueue<string>();
	channel.onmessage = (event) => {
		if (
			typeof event.data !== "string" ||
			encoder.encode(event.data).length > 32_768
		) {
			input.close();
			return;
		}
		input.push(event.data);
	};
	channel.onclose = () => input.close();
	channel.onerror = () => input.close();
	try {
		await peer.setLocalDescription(await peer.createOffer());
		await eventReady(
			peer,
			"icegatheringstatechange",
			() => peer.iceGatheringState === "complete",
		);
		relay.send("signal", {
			kind: "offer",
			session_id: handshake.sessionId(),
			grant_id: grantId,
			certificate_jws: handshake.certificate(),
			sdp: peer.localDescription?.sdp,
		});
		const answer = object(JSON.parse(await relay.signal.next()));
		if (
			answer.kind !== "answer" ||
			answer.session_id !== handshake.sessionId() ||
			typeof answer.sdp !== "string"
		)
			throw new Error("WebRTC answer does not match this session.");
		await peer.setRemoteDescription({ type: "answer", sdp: answer.sdp });
		await eventReady(channel, "open", () => channel.readyState === "open");
		return {
			kind: "webrtc",
			next: () => input.next(),
			send: (envelope) => {
				const text = JSON.stringify(envelope);
				if (
					channel.readyState !== "open" ||
					channel.bufferedAmount > 98_304 ||
					encoder.encode(text).length > 32_768
				)
					throw new Error("WebRTC channel is unavailable or busy.");
				channel.send(text);
			},
			close: () => {
				input.close();
				channel.close();
				peer.close();
			},
		};
	} catch (error) {
		input.close();
		channel.close();
		peer.close();
		throw error;
	}
}

async function authenticate(
	pipe: Pipe,
	handshake: NoiseHandshake,
	grantId: string,
	deviceId: string,
	consumed: () => void,
): Promise<{ session: NoiseSession; expiresAt: number; bootId: string }> {
	const sessionId = handshake.sessionId();
	pipe.send({
		kind: "hello",
		session_id: sessionId,
		grant_id: grantId,
		certificate_jws: handshake.certificate(),
		data: base64url(handshake.write(now())),
	});
	const response = parseEnvelope(await pipe.next());
	if (response.kind !== "handshake" || response.session_id !== sessionId)
		throw new Error("Unexpected Noise handshake.");
	handshake.read(unbase64url(response.data, 128), now());
	pipe.send({
		kind: "handshake",
		session_id: sessionId,
		data: base64url(handshake.write(now())),
	});
	consumed();
	const session = handshake.finish(now());
	try {
		const response = parseEnvelope(await pipe.next());
		if (response.kind !== "message" || response.session_id !== sessionId)
			throw new Error("Device did not confirm encrypted management.");
		const bytes = session.decrypt(unbase64url(response.data, 16_400), now());
		let ready: Record<string, unknown>;
		try {
			ready = object(JSON.parse(decoder.decode(bytes)));
		} finally {
			bytes.fill(0);
		}
		if (
			ready.ready !== true ||
			ready.device_id !== deviceId ||
			typeof ready.expires_at !== "number" ||
			ready.expires_at <= now() ||
			ready.expires_at > now() + 305 ||
			typeof ready.boot_id !== "string"
		)
			throw new Error("Encrypted device identity confirmation failed.");
		return { session, expiresAt: ready.expires_at, bootId: ready.boot_id };
	} catch (error) {
		session.close();
		session.free();
		throw error;
	}
}

export function matchesOperationResponse(
	command: Record<string, unknown>,
	requestId: string,
	response: Record<string, unknown>,
): boolean {
	if (command.type === "operation" && typeof command.operation_id === "string")
		return (
			response.operation_id === command.operation_id ||
			(response.operation_id === requestId && response.state === "rejected")
		);
	return response.operation_id === requestId;
}

export class DeviceManagementConnection {
	private busy = false;
	private closed = false;
	private constructor(
		private readonly pipe: Pipe,
		private readonly relay: Relay,
		private readonly session: NoiseSession,
		private readonly sessionId: string,
		readonly deviceId: string,
		readonly expiresAt: number,
		readonly bootId: string,
	) {}
	get transport(): "webrtc" | "websocket" {
		return this.pipe.kind;
	}
	static async connect(
		api: IApiState,
		profile: IProfile,
		controller: BrowserController,
		receipt: DeviceReceipt,
		grantId: string,
		signal?: AbortSignal,
	): Promise<DeviceManagementConnection> {
		const participant = crypto.randomUUID();
		const admission = await api.fetch<SignalingAdmission>(
			profile,
			`devices/${encodeURIComponent(receipt.device_id)}/signaling/controller`,
			{
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ participant_id: participant }),
				signal,
			},
		);
		if (
			admission.device_auth_epoch !== receipt.auth_epoch ||
			admission.expires_at <= now() + 10 ||
			admission.expires_at > now() + 305 ||
			admission.signaling_urls.length === 0 ||
			admission.signaling_urls.length > 4
		)
			throw new Error("Invalid device signaling admission.");
		let relay: Relay | undefined;
		for (const url of admission.signaling_urls) {
			if (signal?.aborted) throw new Error("Management connection cancelled.");
			const candidate = new Relay(
				url,
				admission,
				participant,
				receipt.device_id,
			);
			try {
				await candidate.connect();
				relay = candidate;
				break;
			} catch {
				candidate.close();
			}
		}
		if (!relay) throw new Error("Device signaling could not be reached.");
		let pipe: Pipe | undefined;
		let handshake: NoiseHandshake | undefined;
		let finished = false;
		const cancel = () => {
			pipe?.close();
			relay?.close();
		};
		signal?.addEventListener("abort", cancel, { once: true });
		try {
			handshake = controller.beginNoise(
				grantId,
				Uint8Array.from(receipt.identity.management_key),
				now(),
			);
			try {
				pipe = await rtcPipe(relay, handshake, grantId, admission);
			} catch {
				handshake.close();
				handshake.free();
				if (signal?.aborted)
					throw new Error("Management connection cancelled.");
				handshake = controller.beginNoise(
					grantId,
					Uint8Array.from(receipt.identity.management_key),
					now(),
				);
				const ws = relay;
				pipe = {
					kind: "websocket",
					send: (envelope) => ws.send("noise", envelope),
					next: () => ws.noise.next(),
					close: () => ws.close(),
				};
			}
			const sessionId = handshake.sessionId();
			const authenticated = await authenticate(
				pipe,
				handshake,
				grantId,
				receipt.device_id,
				() => {
					finished = true;
				},
			);
			if (signal?.aborted) {
				authenticated.session.close();
				authenticated.session.free();
				throw new Error("Management connection cancelled.");
			}
			return new DeviceManagementConnection(
				pipe,
				relay,
				authenticated.session,
				sessionId,
				receipt.device_id,
				authenticated.expiresAt,
				authenticated.bootId,
			);
		} catch (error) {
			cancel();
			throw error;
		} finally {
			signal?.removeEventListener("abort", cancel);
			if (handshake && !finished) {
				handshake.close();
				handshake.free();
			}
		}
	}
	async request(
		command: Record<string, unknown>,
		operationId: string = crypto.randomUUID(),
	): Promise<ManagementResponse> {
		if (this.closed || this.expiresAt <= now())
			throw new Error(
				"Management session expired. Reconnect before continuing.",
			);
		if (this.busy)
			throw new Error("Wait for the current device operation to finish.");
		this.busy = true;
		try {
			const issued = now();
			const bytes = encoder.encode(
				JSON.stringify({
					operation_id: operationId,
					device_id: this.deviceId,
					issued_at: issued,
					expires_at: issued + 60,
					command,
				}),
			);
			let encrypted: Uint8Array;
			try {
				encrypted = this.session.encrypt(bytes, now());
			} finally {
				bytes.fill(0);
			}
			this.pipe.send({
				kind: "message",
				session_id: this.sessionId,
				data: base64url(encrypted),
			});
			const response = parseEnvelope(await this.pipe.next());
			if (response.kind !== "message" || response.session_id !== this.sessionId)
				throw new Error("Management response does not match this session.");
			const plaintext = this.session.decrypt(
				unbase64url(response.data, 16_400),
				now(),
			);
			let value: Record<string, unknown>;
			try {
				value = object(JSON.parse(decoder.decode(plaintext)));
			} finally {
				plaintext.fill(0);
			}
			if (
				!matchesOperationResponse(command, operationId, value) ||
				typeof value.state !== "string" ||
				!value.result ||
				typeof value.result !== "object"
			)
				throw new Error(
					"Management response does not match the requested operation.",
				);
			return value as unknown as ManagementResponse;
		} catch {
			this.close();
			throw new Error(
				`Device operation ${operationId} has no confirmed result. Reconnect and inspect its status before retrying.`,
			);
		} finally {
			this.busy = false;
		}
	}
	close(): void {
		if (this.closed) return;
		this.closed = true;
		this.pipe.close();
		this.relay.close();
		this.session.close();
		this.session.free();
	}
}
