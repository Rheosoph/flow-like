import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import type { ArtifactTransferStatus } from "./artifacts";
import { base64url, unbase64url } from "./crypto";
import type { ModelAssetStatus } from "./models";
import type { DeviceServiceStream, TunnelServiceOptions } from "./tunnel";
import {
	DeviceTunnelDataClient,
	type TunnelArtifactUpload,
	TunnelDataRequestError,
	type TunnelModelAssetPush,
	isTunnelReadCommand,
} from "./tunnel-data";
import type { TunnelConnectOptions } from "./tunnel-transport";
import {
	type BrowserController,
	type DeviceReceipt,
	type ManagementResponse,
	type NoiseHandshake,
	type NoiseSession,
	type SignalingAdmission,
	managementRejection,
} from "./types";
import type { RelayFallbackReason } from "./workspace/types";

const PROTOCOL = "flowlike.device-management.v1";
const TIMED_OUT = "Management connection timed out.";
const CANCELLED = "Management connection cancelled.";
const INVALID_ADMISSION = "Invalid device signaling admission.";
const IDENTITY_FAILED = "Encrypted device identity confirmation failed.";
const decoder = new TextDecoder("utf-8", { fatal: true });
const encoder = new TextEncoder();
const now = () => Math.floor(Date.now() / 1000);

export class ManagementReadError extends Error {
	constructor(
		readonly code: "timeout" | "connection_closed" | "invalid_reply",
		message: string,
	) {
		super(message);
	}
}
type Envelope = {
	kind: "hello" | "handshake" | "message";
	session_id: string;
	data: string;
	grant_id?: string;
	certificate_jws?: string;
};
type Channel = "signal" | "noise";

export class FrameQueue<T> {
	constructor(
		private readonly capacity = 32,
		private readonly byteLimit = Number.POSITIVE_INFINITY,
		private readonly size: (value: T) => number = () => 0,
	) {}
	private bytes = 0;
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
		if (
			this.values.length >= this.capacity ||
			this.bytes + this.size(value) > this.byteLimit
		) {
			this.close(
				new ManagementReadError(
					"invalid_reply",
					"Management input exceeded its bound.",
				),
			);
			return;
		}
		this.values.push(value);
		this.bytes += this.size(value);
	}
	next(timeout = 15_000): Promise<T> {
		if (this.failure) return Promise.reject(this.failure);
		if (this.values.length) {
			const value = this.values.splice(0, 1)[0] as T;
			this.bytes -= this.size(value);
			return Promise.resolve(value);
		}
		if (this.waiting)
			return Promise.reject(
				new Error("Management frames require an ordered reader."),
			);
		return new Promise((resolve, reject) => {
			const timer = setTimeout(() => {
				this.waiting = undefined;
				reject(new ManagementReadError("timeout", TIMED_OUT));
			}, timeout);
			this.waiting = { resolve, reject, timer };
		});
	}
	close(
		error: Error = new ManagementReadError(
			"connection_closed",
			"Management connection closed.",
		),
	): void {
		if (this.failure) return;
		this.failure = error;
		this.values = [];
		this.bytes = 0;
		if (this.waiting) {
			clearTimeout(this.waiting.timer);
			this.waiting.reject(error);
			this.waiting = undefined;
		}
	}
}

/** IA §6.4.3 connection progress steps owned by the transport; "reading_services" follows in the live layer. */
export type ConnectStepId =
	| "getting_pass"
	| "reaching_device"
	| "trying_direct"
	| "securing";

export interface ConnectProgress {
	step: ConnectStepId;
	state: "active" | "done" | "skipped" | "failed";
	/** With `trying_direct` skipped: why the session runs over the relay. */
	fallbackReason?: RelayFallbackReason;
	/** With `getting_pass` done: the admission's expiry and this computer's raw clock at arrival (hub-offset fallback). */
	admission?: { expiresAt: number; receivedAtMs: number };
}

export interface ConnectOptions {
	onStep?: (progress: ConnectProgress) => void;
}

export type ConnectErrorCode =
	| "not_configured"
	| "needs_wss"
	| "access_expired"
	| "epoch_mismatch"
	| "invalid_admission"
	| "http"
	| "relay_unreachable"
	| "handshake_failed"
	| "identity_confirmation_failed"
	| "cancelled";

/** Fixed client facts only; never includes a frame, command, credential or raw error. */
export interface ManagementFailureDiagnostic {
	transport?: "webrtc" | "websocket";
	phase: ConnectStepId | "wait_reply" | "envelope" | "decrypt" | "response";
	cause:
		| ConnectErrorCode
		| "timeout"
		| "connection_closed"
		| "invalid_reply"
		| "decrypt_failed"
		| "transport_failed";
	fallbackReason?: RelayFallbackReason;
}

/** A connect failure tied to its progress step; the message is the transport's original sentence. */
export class ConnectError extends Error {
	constructor(
		readonly step: ConnectStepId,
		readonly code: ConnectErrorCode,
		message: string,
		readonly detail: {
			status?: number;
			url?: string;
			transport?: "webrtc" | "websocket";
			fallbackReason?: RelayFallbackReason;
		} = {},
	) {
		super(message);
		this.name = "ConnectError";
	}
	get diagnostic(): ManagementFailureDiagnostic {
		return {
			phase: this.step,
			cause: this.code,
			...(this.detail.transport ? { transport: this.detail.transport } : {}),
			...(this.detail.fallbackReason
				? { fallbackReason: this.detail.fallbackReason }
				: {}),
		};
	}
}

function errorField(
	error: unknown,
	field: "status" | "serverMessage",
): unknown {
	if (!error || typeof error !== "object") return undefined;
	return (error as Record<string, unknown>)[field];
}

function admissionCode(
	status: number | undefined,
	message: string,
): ConnectErrorCode {
	if (status === 401 || status === 403) return "access_expired";
	if (status !== 503) return "http";
	return message.includes("WSS") ? "needs_wss" : "not_configured";
}

function admissionFailure(error: unknown): ConnectError {
	const status = errorField(error, "status");
	const server = errorField(error, "serverMessage");
	const message =
		typeof server === "string" && server
			? server
			: error instanceof Error
				? error.message
				: String(error);
	const known = typeof status === "number" ? status : undefined;
	return new ConnectError(
		"getting_pass",
		admissionCode(known, message),
		message,
		{
			status: known,
		},
	);
}

function hasTurnServer(servers: RTCIceServer[]): boolean {
	return servers
		.flatMap((server) => server.urls)
		.some((url) => url.toLowerCase().startsWith("turn"));
}

class DirectConnectionError extends Error {
	constructor(
		readonly reason: RelayFallbackReason,
		message: string,
	) {
		super(message);
		this.name = "DirectConnectionError";
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

export class Relay {
	readonly noise = new FrameQueue<string>();
	readonly signal = new FrameQueue<string>();
	readonly tunnel = new FrameQueue<Uint8Array>(512, 4_259_840, (v) => v.length);
	private readonly ready = new FrameQueue<Record<string, unknown>>();
	private readonly reauthorized = new FrameQueue<number>();
	private readonly opened = new FrameQueue<void>();
	private readonly socket: WebSocket;
	private heartbeat?: ReturnType<typeof setInterval>;
	private lastPong = Date.now();
	private admitted = false;
	private closed = false;
	private readonly closeListeners = new Set<() => void>();
	constructor(
		url: string,
		private admission: SignalingAdmission,
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
				if (
					frame.type === "reauthorized" &&
					typeof frame.expires_at === "number"
				) {
					this.reauthorized.push(frame.expires_at);
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
				const bytes = unbase64url(frame.payload);
				if (frame.channel === "tunnel") {
					this.tunnel.push(bytes);
					return;
				}
				const payload = decoder.decode(bytes);
				if (frame.channel === "noise") this.noise.push(payload);
				else if (frame.channel === "signal") this.signal.push(payload);
			} catch {
				this.close(
					new ManagementReadError("invalid_reply", "Invalid signaling frame."),
				);
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
		this.sendBytes(channel, bytes);
	}
	sendTunnel(bytes: Uint8Array): void {
		this.sendBytes("tunnel", bytes);
	}
	private sendBytes(channel: Channel | "tunnel", bytes: Uint8Array): void {
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
	async waitWritable(): Promise<void> {
		const deadline = Date.now() + 15_000;
		while (this.socket.bufferedAmount > 32_768) {
			if (this.closed || Date.now() >= deadline)
				throw new Error("Device tunnel transport stalled.");
			await new Promise((resolve) => setTimeout(resolve, 10));
		}
		if (this.closed) throw new Error("Device tunnel transport closed.");
	}
	async reauthorize(admission: SignalingAdmission): Promise<void> {
		if (this.closed || admission.expires_at <= this.admission.expires_at)
			throw new Error("Device signaling renewal did not extend admission.");
		await this.waitWritable();
		this.socket.send(
			JSON.stringify({ type: "reauthorize", token: admission.token }),
		);
		const expiresAt = await this.reauthorized.next();
		if (expiresAt !== admission.expires_at) {
			this.close();
			throw new Error("Device signaling renewal differs.");
		}
		this.admission = admission;
	}
	onClosed(listener: () => void): void {
		if (this.closed) queueMicrotask(listener);
		else this.closeListeners.add(listener);
	}
	close(error?: Error): void {
		if (this.closed) return;
		this.closed = true;
		clearInterval(this.heartbeat);
		this.noise.close(error);
		this.signal.close(error);
		this.tunnel.close(error);
		this.ready.close(error);
		this.reauthorized.close(error);
		this.opened.close(error);
		this.socket.onclose = null;
		this.socket.onmessage = null;
		this.socket.onerror = null;
		this.socket.close();
		for (const listener of this.closeListeners) listener();
		this.closeListeners.clear();
	}
}

interface Pipe {
	send(envelope: Envelope): void;
	next(): Promise<string>;
	close(): void;
	/** Fires once when the channel underneath closes. */
	onClose?(listener: () => void): void;
	kind: "webrtc" | "websocket";
}

export async function eventReady(
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
		const timer = setTimeout(() => {
			cleanup();
			reject(new Error(TIMED_OUT));
		}, 15_000);
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
		throw new DirectConnectionError(
			"webrtc_unavailable",
			"WebRTC is unavailable.",
		);
	const iceServers =
		admission.ice_expires_at !== null && admission.ice_expires_at <= now() + 30
			? []
			: admission.ice_servers;
	if (iceServers.length > 16)
		throw new DirectConnectionError(
			"webrtc_failed",
			"Invalid device ICE configuration.",
		);
	const peer = new RTCPeerConnection({ iceServers });
	const channel = peer.createDataChannel(PROTOCOL, {
		ordered: true,
		protocol: PROTOCOL,
	});
	const input = new FrameQueue<string>();
	const closeListeners = new Set<() => void>();
	let ended = false;
	const end = (error?: Error) => {
		input.close(error);
		if (ended) return;
		ended = true;
		for (const listener of closeListeners) listener();
		closeListeners.clear();
	};
	channel.onmessage = (event) => {
		if (
			typeof event.data !== "string" ||
			encoder.encode(event.data).length > 32_768
		) {
			end(
				new ManagementReadError("invalid_reply", "Invalid management frame."),
			);
			return;
		}
		input.push(event.data);
	};
	channel.onclose = () => end();
	channel.onerror = () => end();
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
			onClose: (listener) => {
				if (ended) queueMicrotask(listener);
				else closeListeners.add(listener);
			},
		};
	} catch (error) {
		input.close();
		channel.close();
		peer.close();
		const message = error instanceof Error ? error.message : String(error);
		throw new DirectConnectionError(
			!hasTurnServer(iceServers)
				? "no_turn_servers"
				: message === TIMED_OUT
					? "ice_timeout"
					: "webrtc_failed",
			message,
		);
	}
}

async function authenticate(
	pipe: Pipe,
	handshake: NoiseHandshake,
	grantId: string,
	deviceId: string,
	consumed: () => void,
): Promise<{
	session: NoiseSession;
	expiresAt: number;
	bootId: string;
	dataTunnel: boolean;
	serviceTunnel: boolean;
}> {
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
			throw new Error(IDENTITY_FAILED);
		return {
			session,
			expiresAt: ready.expires_at,
			bootId: ready.boot_id,
			dataTunnel: ready.data_tunnel === 1,
			serviceTunnel: ready.service_tunnel === 1,
		};
	} catch (error) {
		session.close();
		session.free();
		throw error;
	}
}

/** Matches the device's Noise plaintext bound; larger requests are refused before sending. */
export const MAX_MANAGEMENT_PLAINTEXT = 16 * 1024;

/** Coded rejections carry the device's reason; older agents send none and yield undefined. */
export function rejectionMessage(
	response: Pick<ManagementResponse, "state" | "result">,
): string | undefined {
	const rejection = managementRejection(response);
	if (!rejection) return undefined;
	return rejection.code === "unsupported"
		? `${rejection.error} Update the device's standalone agent to use this operation.`
		: rejection.error;
}

/** Nothing reached the device, so the same request may be retried after fixing its cause. */
export class ManagementRequestNotSentError extends Error {
	constructor(message: string) {
		super(message);
		this.name = "ManagementRequestNotSentError";
	}
}

/** The request left this app but no authenticated reply arrived; its outcome is unknown. */
export class ManagementUnconfirmedError extends Error {
	constructor(
		readonly operationId: string,
		readonly diagnostic?: ManagementFailureDiagnostic,
	) {
		super(
			`Device operation ${operationId} has no confirmed result. Reconnect and inspect its status before retrying.`,
		);
		this.name = "ManagementUnconfirmedError";
	}
}

export function managementFailureDiagnostic(
	error: unknown,
): ManagementFailureDiagnostic | undefined {
	return error instanceof ConnectError ||
		error instanceof ManagementUnconfirmedError
		? error.diagnostic
		: undefined;
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

export async function requestAdmission(
	api: IApiState,
	profile: IProfile,
	receipt: DeviceReceipt,
	participant: string,
	signal?: AbortSignal,
): Promise<SignalingAdmission> {
	let admission: SignalingAdmission;
	try {
		admission = await api.fetch<SignalingAdmission>(
			profile,
			`devices/${encodeURIComponent(receipt.device_id)}/signaling/controller`,
			{
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ participant_id: participant }),
				signal,
			},
		);
	} catch (error) {
		if (signal?.aborted)
			throw new ConnectError("getting_pass", "cancelled", CANCELLED);
		throw admissionFailure(error);
	}
	if (admission.device_auth_epoch !== receipt.auth_epoch)
		throw new ConnectError("getting_pass", "epoch_mismatch", INVALID_ADMISSION);
	if (
		admission.expires_at <= now() + 10 ||
		admission.expires_at > now() + 305 ||
		admission.signaling_urls.length === 0 ||
		admission.signaling_urls.length > 4
	)
		throw new ConnectError(
			"getting_pass",
			"invalid_admission",
			INVALID_ADMISSION,
		);
	return admission;
}

export async function openRelay(
	admission: SignalingAdmission,
	participant: string,
	deviceId: string,
	signal?: AbortSignal,
): Promise<Relay> {
	for (const url of admission.signaling_urls) {
		if (signal?.aborted)
			throw new ConnectError("reaching_device", "cancelled", CANCELLED);
		const candidate = new Relay(url, admission, participant, deviceId);
		try {
			await candidate.connect();
			return candidate;
		} catch {
			candidate.close();
		}
	}
	throw new ConnectError(
		"reaching_device",
		"relay_unreachable",
		"Device signaling could not be reached.",
		{ url: admission.signaling_urls[0] },
	);
}

function relayPipe(relay: Relay): Pipe {
	return {
		kind: "websocket",
		send: (envelope) => relay.send("noise", envelope),
		next: () => relay.noise.next(),
		close: () => relay.close(),
		onClose: (listener) => relay.onClosed(listener),
	};
}

function securingFailure(error: unknown, signal?: AbortSignal): ConnectError {
	if (error instanceof ConnectError) return error;
	if (signal?.aborted)
		return new ConnectError("securing", "cancelled", CANCELLED);
	const message = error instanceof Error ? error.message : String(error);
	return new ConnectError(
		"securing",
		message === IDENTITY_FAILED
			? "identity_confirmation_failed"
			: "handshake_failed",
		message,
	);
}

interface SecureContext {
	relay: Relay;
	admission: SignalingAdmission;
	controller: BrowserController;
	receipt: DeviceReceipt;
	grantId: string;
	signal?: AbortSignal;
	begin: (step: ConnectStepId) => void;
	report: (progress: ConnectProgress) => void;
}

const STEP_FAILURE: Record<ConnectStepId, ConnectErrorCode> = {
	getting_pass: "http",
	reaching_device: "relay_unreachable",
	trying_direct: "handshake_failed",
	securing: "handshake_failed",
};

function asConnectError(
	error: unknown,
	step: ConnectStepId,
	signal?: AbortSignal,
): ConnectError {
	if (error instanceof ConnectError) return error;
	if (signal?.aborted) return new ConnectError(step, "cancelled", CANCELLED);
	return new ConnectError(
		step,
		STEP_FAILURE[step],
		error instanceof Error ? error.message : String(error),
	);
}

export type ConnectionCloseReason = "local" | "remote";

export class DeviceManagementConnection {
	private dataOptions?: TunnelConnectOptions;
	private dataClient?: DeviceTunnelDataClient;
	private busy = false;
	private closed = false;
	private closeReason: ConnectionCloseReason = "local";
	private readonly closeListeners = new Set<
		(reason: ConnectionCloseReason) => void
	>();
	private constructor(
		private readonly pipe: Pipe,
		private readonly relay: Relay,
		private readonly session: NoiseSession,
		private readonly sessionId: string,
		readonly deviceId: string,
		readonly expiresAt: number,
		readonly bootId: string,
		/** Set when the session runs over the relay because the direct connection failed. */
		readonly fallbackReason?: RelayFallbackReason,
		private readonly supportsDataTunnel = false,
		private readonly supportsServiceTunnel = false,
	) {
		pipe.onClose?.(() => this.shutdown("remote"));
	}
	get transport(): "webrtc" | "websocket" {
		return this.pipe.kind;
	}
	get open(): boolean {
		return !this.closed && this.expiresAt > now();
	}
	/** Fires once on close: "remote" for a dropped channel or unanswered request; "local" for explicit closure or an invalid reply. */
	onClosed(listener: (reason: ConnectionCloseReason) => void): () => void {
		if (this.closed) {
			const reason = this.closeReason;
			queueMicrotask(() => listener(reason));
			return () => {};
		}
		this.closeListeners.add(listener);
		return () => this.closeListeners.delete(listener);
	}
	static async connect(
		api: IApiState,
		profile: IProfile,
		controller: BrowserController,
		receipt: DeviceReceipt,
		grantId: string,
		signal?: AbortSignal,
		options: ConnectOptions = {},
	): Promise<DeviceManagementConnection> {
		const report = options.onStep ?? (() => {});
		let step: ConnectStepId = "getting_pass";
		const begin = (next: ConnectStepId) => {
			step = next;
			report({ step, state: "active" });
		};
		try {
			const participant = crypto.randomUUID();
			begin("getting_pass");
			const admission = await requestAdmission(
				api,
				profile,
				receipt,
				participant,
				signal,
			);
			report({
				step,
				state: "done",
				admission: {
					expiresAt: admission.expires_at,
					receivedAtMs: Date.now(),
				},
			});
			begin("reaching_device");
			const relay = await openRelay(
				admission,
				participant,
				receipt.device_id,
				signal,
			);
			report({ step, state: "done" });
			const connection = await DeviceManagementConnection.secure({
				relay,
				admission,
				controller,
				receipt,
				grantId,
				signal,
				begin,
				report,
			});
			connection.dataOptions = { api, profile, controller, receipt, grantId };
			return connection;
		} catch (error) {
			report({ step, state: "failed" });
			throw asConnectError(error, step, signal);
		}
	}
	private static async secure({
		relay,
		admission,
		controller,
		receipt,
		grantId,
		signal,
		begin,
		report,
	}: SecureContext): Promise<DeviceManagementConnection> {
		let pipe: Pipe | undefined;
		let handshake: NoiseHandshake | undefined;
		let finished = false;
		let fallbackReason: RelayFallbackReason | undefined;
		const cancel = () => {
			pipe?.close();
			relay.close();
		};
		const noise = () =>
			controller.beginNoise(
				grantId,
				Uint8Array.from(receipt.identity.management_key),
				now(),
			);
		signal?.addEventListener("abort", cancel, { once: true });
		try {
			begin("trying_direct");
			handshake = noise();
			try {
				pipe = await rtcPipe(relay, handshake, grantId, admission);
				report({ step: "trying_direct", state: "done" });
			} catch (error) {
				handshake.close();
				handshake.free();
				handshake = undefined;
				if (signal?.aborted)
					throw new ConnectError("trying_direct", "cancelled", CANCELLED);
				fallbackReason =
					error instanceof DirectConnectionError
						? error.reason
						: "webrtc_failed";
				report({ step: "trying_direct", state: "skipped", fallbackReason });
				handshake = noise();
				pipe = relayPipe(relay);
			}
			begin("securing");
			const sessionId = handshake.sessionId();
			const authenticated = await authenticate(
				pipe,
				handshake,
				grantId,
				receipt.device_id,
				() => {
					finished = true;
				},
			).catch((error: unknown) => {
				const failure = securingFailure(error, signal);
				failure.detail.transport = pipe?.kind;
				failure.detail.fallbackReason = fallbackReason;
				throw failure;
			});
			if (signal?.aborted) {
				authenticated.session.close();
				authenticated.session.free();
				throw new ConnectError("securing", "cancelled", CANCELLED);
			}
			report({ step: "securing", state: "done" });
			return new DeviceManagementConnection(
				pipe,
				relay,
				authenticated.session,
				sessionId,
				receipt.device_id,
				authenticated.expiresAt,
				authenticated.bootId,
				fallbackReason,
				authenticated.dataTunnel,
				authenticated.serviceTunnel,
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
		if (command.type === "service_listeners" && !this.supportsServiceTunnel)
			throw new ManagementRequestNotSentError(
				"Update the device agent to connect to deployed services.",
			);
		if (isTunnelReadCommand(command))
			return this.requestData(command, operationId);
		if (this.closed || this.expiresAt <= now())
			throw new Error(
				"Management session expired. Reconnect before continuing.",
			);
		if (this.busy)
			throw new Error("Wait for the current device operation to finish.");
		this.busy = true;
		let sent = false;
		let readyToSend = false;
		let phase: ManagementFailureDiagnostic["phase"] = "wait_reply";
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
			if (bytes.length > MAX_MANAGEMENT_PLAINTEXT) {
				bytes.fill(0);
				throw new ManagementRequestNotSentError(
					`Device operation ${operationId} was not sent: it needs ${bytes.length} bytes, but one management message carries at most ${MAX_MANAGEMENT_PLAINTEXT}. Reduce its size and retry.`,
				);
			}
			let encrypted: Uint8Array;
			try {
				encrypted = this.session.encrypt(bytes, now());
			} finally {
				bytes.fill(0);
			}
			readyToSend = true;
			this.pipe.send({
				kind: "message",
				session_id: this.sessionId,
				data: base64url(encrypted),
			});
			sent = true;
			const reply = await this.pipe.next();
			phase = "envelope";
			const response = parseEnvelope(reply);
			if (response.kind !== "message" || response.session_id !== this.sessionId)
				throw new Error("Management response does not match this session.");
			const ciphertext = unbase64url(response.data, 16_400);
			phase = "decrypt";
			const plaintext = this.session.decrypt(ciphertext, now());
			phase = "response";
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
		} catch (error) {
			if (error instanceof ManagementRequestNotSentError) throw error;
			// An encrypted but unsent message has consumed a Noise nonce the device never saw.
			const interrupted =
				readyToSend &&
				phase === "wait_reply" &&
				(!(error instanceof ManagementReadError) ||
					error.code !== "invalid_reply");
			this.shutdown(interrupted ? "remote" : "local");
			if (!sent)
				throw new ManagementRequestNotSentError(
					`Device operation ${operationId} was not sent because the management session failed locally. Reconnect and retry.`,
				);
			throw new ManagementUnconfirmedError(operationId, {
				transport: this.pipe.kind,
				phase,
				cause:
					error instanceof ManagementReadError
						? error.code
						: phase === "decrypt"
							? "decrypt_failed"
							: phase === "envelope" || phase === "response"
								? "invalid_reply"
								: "transport_failed",
				...(this.fallbackReason ? { fallbackReason: this.fallbackReason } : {}),
			});
		} finally {
			this.busy = false;
		}
	}
	async requestData(
		command: Record<string, unknown>,
		operationId: string = crypto.randomUUID(),
		signal?: AbortSignal,
	): Promise<ManagementResponse> {
		if (!isTunnelReadCommand(command))
			throw new ManagementRequestNotSentError(
				"This operation requires the management connection.",
			);
		try {
			return await this.bulk().request(command, operationId, signal);
		} catch (error) {
			if (error instanceof TunnelDataRequestError && error.sent)
				throw new ManagementUnconfirmedError(operationId, {
					phase: "wait_reply",
					cause: "transport_failed",
				});
			throw new ManagementRequestNotSentError(
				error instanceof Error
					? error.message
					: "The device data request could not be sent.",
			);
		}
	}
	uploadArtifact(input: TunnelArtifactUpload): Promise<ArtifactTransferStatus> {
		return this.bulk().uploadArtifact(input);
	}
	/** One model asset push over the data tunnel (plan §3.1); the device checks the digest. */
	pushModelAsset(input: TunnelModelAssetPush): Promise<ModelAssetStatus> {
		return this.bulk().pushModelAsset(input);
	}
	openService(
		placementId: string,
		serviceId: string,
		options?: TunnelServiceOptions,
	): Promise<DeviceServiceStream> {
		if (!this.supportsServiceTunnel)
			throw new Error(
				"Update the device agent to connect to deployed services.",
			);
		return this.bulk().openService(placementId, serviceId, options);
	}
	/** Only for agents with `model_host`: an older one closes the whole tunnel on the unknown target. */
	openModelGateway(
		options: { signal?: AbortSignal } = {},
	): Promise<DeviceServiceStream> {
		if (!this.supportsServiceTunnel)
			throw new Error(
				"Update the device agent to send requests to its models.",
			);
		return this.bulk().openModelGateway(options);
	}
	detachDataTunnel(): DeviceTunnelDataClient | undefined {
		const client = this.dataClient;
		this.dataClient = undefined;
		return client;
	}
	adoptDataTunnel(client: DeviceTunnelDataClient): void {
		if (
			this.closed ||
			!this.supportsDataTunnel ||
			!this.dataOptions ||
			!client.matches(this.dataOptions)
		) {
			client.close();
			return;
		}
		this.dataClient?.close();
		this.dataClient = client;
	}
	private bulk(): DeviceTunnelDataClient {
		if (!this.supportsDataTunnel)
			throw new Error(
				"Update the device agent to use encrypted streaming transfers.",
			);
		if (this.closed || this.expiresAt <= now())
			throw new Error(
				"Management session expired. Reconnect before continuing.",
			);
		if (!this.dataClient) {
			if (!this.dataOptions)
				throw new Error("Device data transport is unavailable.");
			this.dataClient = new DeviceTunnelDataClient(this.dataOptions);
		}
		return this.dataClient;
	}
	close(): void {
		this.shutdown("local");
	}
	private shutdown(reason: ConnectionCloseReason): void {
		if (this.closed) return;
		this.closed = true;
		this.closeReason = reason;
		try {
			// The workspace can move an independent data tunnel to its replacement
			// management connection before the remaining resources are closed.
			if (reason === "remote")
				for (const listener of this.closeListeners) listener(reason);
		} finally {
			this.dataClient?.close();
			this.dataClient = undefined;
			this.pipe.close();
			this.relay.close();
			this.session.close();
			this.session.free();
			try {
				if (reason === "local")
					for (const listener of this.closeListeners) listener(reason);
			} finally {
				this.closeListeners.clear();
			}
		}
	}
}
