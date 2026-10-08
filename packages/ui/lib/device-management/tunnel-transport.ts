import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { base64url } from "./crypto";
import {
	FrameQueue,
	ManagementReadError,
	type Relay,
	eventReady,
	openRelay,
	requestAdmission,
} from "./transport";
import {
	TUNNEL_MAX_BUFFER,
	TUNNEL_PROTOCOL,
	TunnelKind,
	decodeTunnelEnvelope,
	decodeTunnelFrame,
	encodeTunnelEnvelope,
	readTunnelJson,
	tunnelJson,
} from "./tunnel-protocol";
import type {
	BrowserController,
	DeviceReceipt,
	NoiseHandshake,
	NoiseSession,
	SignalingAdmission,
} from "./types";

const now = () => Math.floor(Date.now() / 1000);

export interface TunnelConnectOptions {
	api: IApiState;
	profile: IProfile;
	controller: BrowserController;
	receipt: DeviceReceipt;
	grantId?: string;
	signal?: AbortSignal;
}

/** One ordered encrypted session. A transport loss never replays service bytes. */
export interface TunnelTransport {
	readonly kind: "webrtc" | "websocket";
	readonly expiresAt: number;
	send(plaintext: Uint8Array): Promise<void>;
	next(): Promise<Uint8Array>;
	beginRenewal(): NoiseHandshake;
	installSession(session: NoiseSession): void;
	confirmRenewal(expiresAt: number): void;
	close(): void;
}

interface RawPipe {
	kind: "webrtc" | "websocket";
	send(bytes: Uint8Array): void;
	next(): Promise<Uint8Array>;
	writable(): Promise<void>;
	close(): void;
}

function relayTunnel(connection: Relay): RawPipe {
	return {
		kind: "websocket",
		send: (bytes) => connection.sendTunnel(bytes),
		next: () => connection.tunnel.next(45_000),
		writable: () => connection.waitWritable(),
		close: () => connection.close(),
	};
}

async function rtcTunnel(
	relay: Relay,
	handshake: NoiseHandshake,
	grantId: string,
	admission: SignalingAdmission,
): Promise<RawPipe> {
	if (typeof RTCPeerConnection === "undefined")
		throw new Error("WebRTC is unavailable.");
	const iceServers =
		admission.ice_expires_at !== null && admission.ice_expires_at <= now() + 30
			? []
			: admission.ice_servers;
	if (iceServers.length > 16)
		throw new Error("Invalid device ICE configuration.");
	const peer = new RTCPeerConnection({ iceServers });
	const channel = peer.createDataChannel(TUNNEL_PROTOCOL, {
		ordered: true,
		protocol: TUNNEL_PROTOCOL,
	});
	channel.binaryType = "arraybuffer";
	const input = new FrameQueue<Uint8Array>(
		512,
		TUNNEL_MAX_BUFFER,
		(v) => v.length,
	);
	let ended = false;
	const end = (error?: Error) => {
		if (ended) return;
		ended = true;
		input.close(error);
	};
	const close = () => {
		end();
		channel.close();
		peer.close();
	};
	channel.onclose = () => end();
	channel.onerror = () => end();
	channel.onmessage = (event) => {
		if (
			!(event.data instanceof ArrayBuffer) ||
			event.data.byteLength > 32_768
		) {
			end(
				new ManagementReadError(
					"invalid_reply",
					"Invalid device tunnel frame.",
				),
			);
			close();
			return;
		}
		input.push(new Uint8Array(event.data));
	};
	try {
		await peer.setLocalDescription(await peer.createOffer());
		await eventReady(
			peer,
			"icegatheringstatechange",
			() => peer.iceGatheringState === "complete",
		);
		relay.send("signal", {
			kind: "offer",
			protocol: TUNNEL_PROTOCOL,
			session_id: handshake.sessionId(),
			grant_id: grantId,
			certificate_jws: handshake.certificate(),
			sdp: peer.localDescription?.sdp,
		});
		const answer = JSON.parse(await relay.signal.next());
		if (
			answer.kind !== "answer" ||
			answer.session_id !== handshake.sessionId() ||
			typeof answer.sdp !== "string"
		)
			throw new Error("Device tunnel answer does not match this session.");
		await peer.setRemoteDescription({ type: "answer", sdp: answer.sdp });
		await eventReady(channel, "open", () => channel.readyState === "open");
		return {
			kind: "webrtc",
			close,
			next: () => input.next(45_000),
			send(bytes) {
				if (channel.readyState !== "open")
					throw new ManagementReadError(
						"connection_closed",
						"Device tunnel transport closed.",
					);
				if (channel.bufferedAmount > 98_304 || bytes.length > 32_768)
					throw new Error("Device tunnel transport is unavailable.");
				channel.send(bytes as Uint8Array<ArrayBuffer>);
			},
			async writable() {
				const deadline = Date.now() + 15_000;
				while (channel.bufferedAmount > 32_768) {
					if (channel.readyState !== "open")
						throw new ManagementReadError(
							"connection_closed",
							"Device tunnel transport closed.",
						);
					if (Date.now() >= deadline)
						throw new Error("Device tunnel transport stalled.");
					await new Promise((resolve) => setTimeout(resolve, 10));
				}
				if (channel.readyState !== "open")
					throw new ManagementReadError(
						"connection_closed",
						"Device tunnel transport closed.",
					);
			},
		};
	} catch (error) {
		close();
		throw error;
	}
}

function freshHandshake(options: TunnelConnectOptions): NoiseHandshake {
	if (!options.controller.beginTunnelNoise)
		throw new Error(
			"Update Studio's device cryptography to connect to services.",
		);
	return options.controller.beginTunnelNoise(
		options.grantId ?? "owner",
		Uint8Array.from(options.receipt.identity.management_key),
		now(),
	);
}

function validExpiry(expiresAt: unknown): asserts expiresAt is number {
	if (
		typeof expiresAt !== "number" ||
		!Number.isInteger(expiresAt) ||
		expiresAt <= now() ||
		expiresAt > now() + 305
	)
		throw new Error("Invalid device tunnel authorization expiry.");
}

export async function connectTunnelTransport(
	options: TunnelConnectOptions,
): Promise<TunnelTransport> {
	if (!options.controller.beginTunnelNoise)
		throw new Error(
			"Update Studio's device cryptography to connect to services.",
		);
	const participant = crypto.randomUUID();
	const signal = new AbortController();
	let relay: Relay | undefined;
	let pipe: RawPipe | undefined;
	let session: NoiseSession | undefined;
	let handshake: NoiseHandshake | undefined;
	let admissionTimer: ReturnType<typeof setTimeout> | undefined;
	let closed = false;
	const closeSession = () => {
		session?.close();
		session?.free();
		session = undefined;
	};
	const close = () => {
		if (closed) return;
		closed = true;
		signal.abort();
		clearTimeout(admissionTimer);
		options.signal?.removeEventListener("abort", close);
		pipe?.close();
		relay?.close();
		closeSession();
	};
	options.signal?.addEventListener("abort", close, { once: true });
	if (options.signal?.aborted) close();
	try {
		let admission = await requestAdmission(
			options.api,
			options.profile,
			options.receipt,
			participant,
			signal.signal,
		);
		relay = await openRelay(
			admission,
			participant,
			options.receipt.device_id,
			signal.signal,
		);
		if (closed) {
			relay.close();
			throw new Error("Device tunnel connection cancelled.");
		}
		handshake = freshHandshake(options);
		try {
			pipe = await rtcTunnel(
				relay,
				handshake,
				options.grantId ?? "owner",
				admission,
			);
		} catch {
			handshake.close();
			handshake.free();
			handshake = undefined;
			if (signal.signal.aborted)
				throw new Error("Device tunnel connection cancelled.");
			handshake = freshHandshake(options);
			pipe = relayTunnel(relay);
		}
		if (closed) {
			pipe.close();
			throw new Error("Device tunnel connection cancelled.");
		}
		let route = handshake.sessionId();
		const sendRaw = async (
			kind: "hello" | "handshake" | "message",
			body: Uint8Array,
		) => {
			if (closed || !pipe) throw new Error("Device tunnel closed.");
			await pipe.writable();
			pipe.send(encodeTunnelEnvelope(kind, route, body));
		};
		const receiveRaw = async (kind: "handshake" | "message") => {
			if (closed || !pipe) throw new Error("Device tunnel closed.");
			const envelope = decodeTunnelEnvelope(await pipe.next());
			if (envelope.sessionId !== route || envelope.kind !== kind)
				throw new Error("Device tunnel envelope does not match this session.");
			return envelope.body;
		};
		const authenticate = async () => {
			if (!handshake)
				throw new Error("Device tunnel handshake is unavailable.");
			await sendRaw(
				"hello",
				tunnelJson({
					grant_id: options.grantId ?? "owner",
					certificate_jws: handshake.certificate(),
					data: base64url(handshake.write(now())),
				}),
			);
			handshake.read(await receiveRaw("handshake"), now());
			await sendRaw("handshake", handshake.write(now()));
			const finishing = handshake;
			handshake = undefined;
			session = finishing.finish(now());
			const initial = session.decrypt(await receiveRaw("message"), now());
			try {
				const ready = decodeTunnelFrame(initial);
				if (
					ready.kind !== TunnelKind.Renewed ||
					ready.stream !== 0 ||
					ready.sequence !== 0n
				)
					throw new Error("Device did not confirm its encrypted tunnel.");
				const expiry = readTunnelJson(ready.body).expires_at;
				validExpiry(expiry);
				return expiry;
			} finally {
				initial.fill(0);
			}
		};
		let expiresAt: number;
		try {
			expiresAt = await authenticate();
		} catch (error) {
			if (
				closed ||
				pipe.kind !== "webrtc" ||
				!(error instanceof ManagementReadError) ||
				(error.code !== "timeout" && error.code !== "connection_closed")
			)
				throw error;
			// No application stream has opened. Retry transport setup with fresh Noise keys.
			pipe.close();
			handshake?.close();
			handshake?.free();
			handshake = undefined;
			closeSession();
			handshake = freshHandshake(options);
			route = handshake.sessionId();
			pipe = relayTunnel(relay);
			expiresAt = await authenticate();
		}
		const activeRelay = relay;
		const activePipe = pipe;
		const renewAdmission = () => {
			admissionTimer = setTimeout(
				() => {
					void (async () => {
						admission = await requestAdmission(
							options.api,
							options.profile,
							options.receipt,
							participant,
							signal.signal,
						);
						await activeRelay.reauthorize(admission);
						if (!closed) renewAdmission();
					})().catch(close);
				},
				Math.max(1_000, (admission.expires_at - now() - 60) * 1_000),
			);
		};
		renewAdmission();
		return {
			kind: pipe.kind,
			get expiresAt() {
				return expiresAt;
			},
			async send(plaintext) {
				try {
					await activePipe.writable();
					if (closed || !session) throw new Error("Device tunnel closed.");
					const encrypted = session.encrypt(plaintext, now());
					activePipe.send(encodeTunnelEnvelope("message", route, encrypted));
				} catch (error) {
					close();
					throw error;
				}
			},
			async next() {
				try {
					const bytes = await receiveRaw("message");
					if (!session) throw new Error("Device tunnel closed.");
					return session.decrypt(bytes, now());
				} catch (error) {
					close();
					throw error;
				}
			},
			beginRenewal: () => freshHandshake(options),
			installSession(next) {
				if (closed) {
					next.close();
					next.free();
					throw new Error("Device tunnel closed.");
				}
				session?.close();
				session?.free();
				session = next;
			},
			confirmRenewal(expiry) {
				validExpiry(expiry);
				expiresAt = expiry;
			},
			close,
		};
	} catch (error) {
		try {
			if (handshake) {
				handshake.close();
				handshake.free();
			}
		} finally {
			close();
		}
		throw error;
	}
}
