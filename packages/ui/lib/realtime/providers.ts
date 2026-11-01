import {
	amzDate,
	canonicalQuery,
	hmacSha256,
	sha256Hex,
	signingKey,
} from "../channel/aws-sigv4";
import type { RealtimeTransport, RealtimeWakeup } from "./types";

const MAX_FRAME_BYTES = 64 * 1024;
const NATS_CONNECT_TIMEOUT_MS = 10_000;
const NATS_FLUSH_TIMEOUT_MS = 10_000;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export class RealtimeProviderProtocolError extends Error {}

export interface RealtimeProviderConnection {
	close(): void;
}

export interface RealtimeProviderCallbacks {
	wakeup(wakeup: RealtimeWakeup): void;
	disconnected(): void;
}

export interface RealtimeProviderOptions {
	/** Server clock minus local clock, for credentials signed with a timestamp. */
	clockOffsetMs?: number;
}

export type RealtimeProviderOpener = (
	transport: Exclude<RealtimeTransport, { type: "http" }>,
	callbacks: RealtimeProviderCallbacks,
	signal: AbortSignal,
	options?: RealtimeProviderOptions,
) => Promise<RealtimeProviderConnection>;

function validUrl(value: string, protocols: string[]): URL {
	const url = new URL(value);
	if (
		!protocols.includes(url.protocol) ||
		url.username ||
		url.password ||
		url.hash
	)
		throw new Error("Invalid realtime provider endpoint");
	return url;
}

/** Provider messages can only invalidate a snapshot; the API authenticates the replay. */
export function parseRealtimeWakeup(
	input: unknown,
): RealtimeWakeup | undefined {
	let value = input;
	if (typeof value === "string") {
		if (encoder.encode(value).byteLength > 4096) return;
		try {
			value = JSON.parse(value);
		} catch {
			return;
		}
	}
	if (!value || typeof value !== "object" || Array.isArray(value)) return;
	if (
		Object.keys(value).some(
			(key) => !["protocol_version", "kind", "nonce"].includes(key),
		)
	)
		return;
	const wakeup = value as Partial<RealtimeWakeup>;
	if (
		wakeup.protocol_version !== 1 ||
		wakeup.kind !== "invalidate" ||
		typeof wakeup.nonce !== "string" ||
		!wakeup.nonce ||
		encoder.encode(wakeup.nonce).byteLength > 128 ||
		/\p{Cc}/u.test(wakeup.nonce)
	)
		return;
	return { protocol_version: 1, kind: "invalidate", nonce: wakeup.nonce };
}

function wake(callbacks: RealtimeProviderCallbacks, value: unknown): void {
	const message = parseRealtimeWakeup(value);
	if (message) callbacks.wakeup(message);
}

interface SocketHandlers {
	opened(socket: WebSocket): void;
	message(data: unknown, socket: WebSocket, ready: () => void): void;
	tick?(socket: WebSocket): void;
}

function socketConnection(
	url: string,
	protocol: string,
	handlers: SocketHandlers,
	callbacks: RealtimeProviderCallbacks,
	signal: AbortSignal,
): Promise<RealtimeProviderConnection> {
	if (signal.aborted)
		return Promise.reject(new Error("Realtime provider connection aborted"));
	return new Promise((resolve, reject) => {
		const socket = new WebSocket(url, protocol);
		socket.binaryType = "arraybuffer";
		let settled = false;
		let closed = false;
		let tick: ReturnType<typeof setInterval> | undefined;
		const timer = setTimeout(() => fail(), 10_000);
		const close = () => {
			if (closed) return;
			closed = true;
			clearTimeout(timer);
			clearInterval(tick);
			signal.removeEventListener("abort", aborted);
			socket.close();
		};
		const fail = () => {
			if (closed) return;
			close();
			if (settled) callbacks.disconnected();
			else reject(new Error("Realtime provider connection failed"));
		};
		const aborted = () => {
			close();
			if (!settled) reject(new Error("Realtime provider connection aborted"));
		};
		const ready = () => {
			if (settled || closed) return;
			settled = true;
			clearTimeout(timer);
			if (handlers.tick)
				tick = setInterval(() => {
					try {
						handlers.tick?.(socket);
					} catch {
						fail();
					}
				}, 20_000);
			resolve({ close });
		};
		signal.addEventListener("abort", aborted, { once: true });
		socket.addEventListener("open", () => {
			try {
				if (socket.protocol !== protocol)
					throw new Error("Realtime provider selected an unsupported protocol");
				handlers.opened(socket);
			} catch {
				fail();
			}
		});
		socket.addEventListener("message", (event) => {
			if (closed) return;
			try {
				handlers.message(event.data, socket, ready);
			} catch {
				fail();
			}
		});
		socket.addEventListener("close", fail);
		socket.addEventListener("error", fail);
	});
}

function azure(
	transport: Extract<RealtimeTransport, { type: "azure_web_pubsub" }>,
	callbacks: RealtimeProviderCallbacks,
	signal: AbortSignal,
): Promise<RealtimeProviderConnection> {
	validUrl(transport.url, ["wss:", "ws:"]);
	if (!transport.group) throw new Error("Missing realtime group");
	let lastReceived = Date.now();
	return socketConnection(
		transport.url,
		"json.webpubsub.azure.v1",
		{
			opened() {},
			message(data, _socket, ready) {
				if (
					typeof data !== "string" ||
					encoder.encode(data).byteLength > MAX_FRAME_BYTES
				)
					throw new Error("Invalid realtime provider frame");
				const message = JSON.parse(data);
				lastReceived = Date.now();
				if (message.type === "system" && message.event === "connected") ready();
				if (message.type === "system" && message.event === "disconnected")
					throw new Error("Realtime provider disconnected");
				if (
					message.type === "message" &&
					((message.from === "server" && message.group === undefined) ||
						(message.from === "group" && message.group === transport.group))
				)
					wake(callbacks, message.data);
			},
			tick(socket) {
				if (Date.now() - lastReceived >= 60_000)
					throw new Error("Realtime provider heartbeat timed out");
				socket.send(JSON.stringify({ type: "ping" }));
			},
		},
		callbacks,
		signal,
	);
}

/** IoT signs the session token after the signature, matching the AWS IoT device SDK. */
export async function awsRealtimeWebsocketUrl(
	transport: Extract<RealtimeTransport, { type: "aws_mqtt" }>,
	date = new Date(),
): Promise<string> {
	const endpoint = transport.endpoint.includes("://")
		? transport.endpoint
		: `https://${transport.endpoint}`;
	const url = validUrl(endpoint, ["https:", "wss:"]);
	if ((url.pathname !== "/" && url.pathname !== "/mqtt") || url.search)
		throw new Error("Invalid AWS IoT endpoint");
	if (transport.credentials.expiration * 1000 <= date.getTime())
		throw new Error("AWS realtime credentials expired");
	url.protocol = "wss:";
	url.pathname = "/mqtt";
	const timestamp = amzDate(date);
	const dateStamp = timestamp.slice(0, 8);
	const scope = `${dateStamp}/${transport.region}/iotdevicegateway/aws4_request`;
	url.searchParams.set("X-Amz-Algorithm", "AWS4-HMAC-SHA256");
	url.searchParams.set(
		"X-Amz-Credential",
		`${transport.credentials.access_key_id}/${scope}`,
	);
	url.searchParams.set("X-Amz-Date", timestamp);
	url.searchParams.set(
		"X-Amz-Expires",
		String(
			Math.min(
				300,
				Math.floor(transport.credentials.expiration - date.getTime() / 1000),
			),
		),
	);
	url.searchParams.set("X-Amz-SignedHeaders", "host");
	const canonical = [
		"GET",
		"/mqtt",
		canonicalQuery(url.searchParams),
		`host:${url.host}\n`,
		"host",
		await sha256Hex(""),
	].join("\n");
	const toSign = [
		"AWS4-HMAC-SHA256",
		timestamp,
		scope,
		await sha256Hex(canonical),
	].join("\n");
	const signature = await hmacSha256(
		await signingKey(
			transport.credentials.secret_access_key,
			dateStamp,
			transport.region,
			"iotdevicegateway",
		),
		toSign,
	);
	url.searchParams.set(
		"X-Amz-Signature",
		[...signature].map((byte) => byte.toString(16).padStart(2, "0")).join(""),
	);
	url.searchParams.set(
		"X-Amz-Security-Token",
		transport.credentials.session_token,
	);
	// URLSearchParams encodes spaces as '+', while SigV4 requires RFC 3986 encoding.
	url.search = canonicalQuery(url.searchParams);
	return url.toString();
}

function mqttString(value: string): number[] {
	const bytes = encoder.encode(value);
	if (!bytes.length || bytes.length > 65_535)
		throw new Error("Invalid MQTT string");
	return [bytes.length >> 8, bytes.length & 255, ...bytes];
}

function mqttPacket(type: number, payload: number[]): Uint8Array {
	let length = payload.length;
	const header = [type];
	do {
		let byte = length % 128;
		length = Math.floor(length / 128);
		if (length) byte |= 128;
		header.push(byte);
	} while (length);
	return Uint8Array.from([...header, ...payload]);
}

async function aws(
	transport: Extract<RealtimeTransport, { type: "aws_mqtt" }>,
	callbacks: RealtimeProviderCallbacks,
	signal: AbortSignal,
	clockOffsetMs = 0,
): Promise<RealtimeProviderConnection> {
	if (!transport.topic || /[+#]/.test(transport.topic))
		throw new Error("MQTT subscription must name one topic");
	const url = await awsRealtimeWebsocketUrl(
		transport,
		new Date(Date.now() + clockOffsetMs),
	);
	let buffer = new Uint8Array(0);
	let pingOutstanding = false;
	return socketConnection(
		url,
		"mqtt",
		{
			opened(socket) {
				socket.send(
					mqttPacket(0x10, [
						...mqttString("MQTT"),
						4,
						2,
						0,
						30,
						...mqttString(transport.client_id),
					]),
				);
			},
			message(data, socket, ready) {
				if (
					!(data instanceof ArrayBuffer) ||
					data.byteLength + buffer.byteLength > MAX_FRAME_BYTES
				)
					throw new Error("Invalid MQTT frame");
				const joined = new Uint8Array(buffer.length + data.byteLength);
				joined.set(buffer);
				joined.set(new Uint8Array(data), buffer.length);
				buffer = joined;
				while (buffer.length >= 2) {
					let length = 0;
					let multiplier = 1;
					let offset = 1;
					for (;;) {
						if (offset >= buffer.length) return;
						const byte = buffer[offset++];
						length += (byte & 127) * multiplier;
						if (length > MAX_FRAME_BYTES || offset > 5)
							throw new Error("MQTT packet too large");
						if (!(byte & 128)) break;
						multiplier *= 128;
					}
					if (buffer.length < offset + length) return;
					const type = buffer[0];
					const payload = buffer.slice(offset, offset + length);
					buffer = buffer.slice(offset + length);
					if (type === 0x20) {
						if (payload.length !== 2 || payload[1] !== 0)
							throw new Error("MQTT connection denied");
						socket.send(
							mqttPacket(0x82, [0, 1, ...mqttString(transport.topic), 0]),
						);
					} else if (type === 0x90) {
						if (
							payload.length !== 3 ||
							payload[0] !== 0 ||
							payload[1] !== 1 ||
							payload[2] !== 0
						)
							throw new Error("MQTT subscription denied");
						ready();
					} else if (type >> 4 === 3) {
						if (payload.length < 2 || (type & 6) !== 0)
							throw new Error("Invalid MQTT publication");
						const topicLength = (payload[0] << 8) | payload[1];
						if (topicLength + 2 > payload.length)
							throw new Error("Invalid MQTT topic");
						if (
							decoder.decode(payload.slice(2, topicLength + 2)) ===
							transport.topic
						)
							wake(callbacks, decoder.decode(payload.slice(topicLength + 2)));
					} else if (type === 0xd0) pingOutstanding = false;
				}
			},
			tick(socket) {
				if (pingOutstanding) throw new Error("MQTT heartbeat timed out");
				pingOutstanding = true;
				socket.send(Uint8Array.of(0xc0, 0));
			},
		},
		callbacks,
		signal,
	);
}

/** Settles by the deadline or on abort; a result that arrives later is released, not leaked. */
function bounded<T>(
	operation: Promise<T>,
	timeoutMs: number,
	signal: AbortSignal,
	release: (late: T) => void,
): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		let settled = false;
		const settle = () => {
			settled = true;
			clearTimeout(timer);
			signal.removeEventListener("abort", aborted);
		};
		const aborted = () => {
			if (settled) return;
			settle();
			reject(new Error("Realtime provider connection aborted"));
		};
		const timer = setTimeout(() => {
			if (settled) return;
			settle();
			reject(new Error("Realtime provider connection timed out"));
		}, timeoutMs);
		signal.addEventListener("abort", aborted, { once: true });
		if (signal.aborted) aborted();
		operation.then(
			(value) => {
				if (settled) release(value);
				else {
					settle();
					resolve(value);
				}
			},
			(error) => {
				if (settled) return;
				settle();
				reject(error);
			},
		);
	});
}

async function nats(
	transport: Extract<RealtimeTransport, { type: "nats" }>,
	callbacks: RealtimeProviderCallbacks,
	signal: AbortSignal,
): Promise<RealtimeProviderConnection> {
	const url = validUrl(transport.url, ["wss:", "ws:"]);
	if (url.search || !transport.subject || /[*>\s]/.test(transport.subject))
		throw new Error("Invalid NATS subscription");
	const { wsconnect, jwtAuthenticator } = await import("@nats-io/nats-core");
	if (signal.aborted) throw new Error("Realtime provider connection aborted");
	const client = await bounded(
		wsconnect({
			servers: url.toString(),
			authenticator: jwtAuthenticator(transport.token),
			timeout: NATS_CONNECT_TIMEOUT_MS,
			reconnect: false,
			ignoreClusterUpdates: true,
			name: "flow-like-realtime",
		}),
		NATS_CONNECT_TIMEOUT_MS,
		signal,
		(late) => void late.close().catch(() => {}),
	);
	let closed = false;
	const close = () => {
		closed = true;
		signal.removeEventListener("abort", close);
		void client.close();
	};
	signal.addEventListener("abort", close, { once: true });
	if (signal.aborted) {
		close();
		throw new Error("Realtime provider connection aborted");
	}
	const subscription = client.subscribe(transport.subject, {
		callback(error, message) {
			if (closed) return;
			if (error || message.data.byteLength > MAX_FRAME_BYTES) {
				close();
				callbacks.disconnected();
				return;
			}
			wake(callbacks, decoder.decode(message.data));
		},
	});
	try {
		await bounded(client.flush(), NATS_FLUSH_TIMEOUT_MS, signal, () => {});
	} catch (error) {
		subscription.unsubscribe();
		close();
		throw error;
	}
	void client.closed().then(() => {
		if (!closed) {
			close();
			callbacks.disconnected();
		}
	});
	return { close };
}

async function firebase(
	transport: Extract<RealtimeTransport, { type: "gcp_firebase_rtdb" }>,
	callbacks: RealtimeProviderCallbacks,
	signal: AbortSignal,
): Promise<RealtimeProviderConnection> {
	const url = validUrl(transport.database_url, ["https:"]);
	if (
		url.search ||
		(url.pathname !== "/" && url.pathname !== "") ||
		!transport.path ||
		transport.path
			.split("/")
			.some((segment) => segment === ".." || segment === ".")
	)
		throw new Error("Invalid Firebase realtime path");
	const controller = new AbortController();
	let closed = false;
	let idle: ReturnType<typeof setTimeout> | undefined;
	const close = () => {
		closed = true;
		clearTimeout(idle);
		signal.removeEventListener("abort", close);
		controller.abort();
	};
	signal.addEventListener("abort", close, { once: true });
	if (signal.aborted) close();
	const resetIdle = () => {
		clearTimeout(idle);
		idle = setTimeout(() => {
			close();
			callbacks.disconnected();
		}, 90_000);
	};
	try {
		resetIdle();
		const exchange = await fetch(
			`https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key=${encodeURIComponent(transport.api_key)}`,
			{
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					token: transport.custom_token,
					returnSecureToken: true,
				}),
				signal: controller.signal,
				credentials: "omit",
				redirect: "error",
			},
		);
		if (!exchange.ok) {
			await exchange.body?.cancel();
			throw new Error("Firebase realtime authorization failed");
		}
		const auth = await boundedJson(exchange);
		if (typeof auth.idToken !== "string" || !auth.idToken)
			throw new Error("Firebase returned no identity token");
		url.pathname = `${transport.path.replace(/^\/+|\/+$/g, "")}.json`;
		url.searchParams.set("auth", auth.idToken);
		const response = await fetch(url, {
			headers: { Accept: "text/event-stream" },
			signal: controller.signal,
			credentials: "omit",
		});
		if (!response.ok || !response.body) {
			await response.body?.cancel();
			throw new Error("Firebase realtime subscription failed");
		}
		const reader = response.body.getReader();
		void (async () => {
			const streamDecoder = new TextDecoder();
			let buffer = "";
			try {
				for (;;) {
					const { done, value } = await reader.read();
					if (done) throw new Error("Firebase realtime stream ended");
					resetIdle();
					if (value.byteLength > MAX_FRAME_BYTES)
						throw new Error("Firebase realtime frame too large");
					buffer += streamDecoder.decode(value, { stream: true });
					if (encoder.encode(buffer).byteLength > MAX_FRAME_BYTES)
						throw new Error("Firebase realtime frame too large");
					for (;;) {
						const separator = /\r?\n\r?\n/.exec(buffer);
						if (!separator) break;
						const frame = buffer.slice(0, separator.index);
						buffer = buffer.slice(separator.index + separator[0].length);
						const lines = frame.split(/\r?\n/);
						const name = lines
							.find((line) => line.startsWith("event:"))
							?.slice(6)
							.trim();
						if (name === "cancel" || name === "auth_revoked")
							throw new Error("Firebase realtime access ended");
						if (name !== "put" && name !== "patch") continue;
						const raw = lines
							.filter((line) => line.startsWith("data:"))
							.map((line) => line.slice(5).trim())
							.join("\n");
						const event = JSON.parse(raw);
						wake(callbacks, event.data);
					}
				}
			} catch {
				if (!closed) {
					close();
					callbacks.disconnected();
				}
			} finally {
				await reader.cancel().catch(() => {});
				reader.releaseLock();
			}
		})();
		return { close };
	} catch (error) {
		close();
		throw error;
	}
}

export async function boundedJson(
	response: Response,
): Promise<Record<string, unknown>> {
	if (!response.body)
		throw new RealtimeProviderProtocolError("Empty realtime provider response");
	const reader = response.body.getReader();
	const chunks: Uint8Array[] = [];
	let size = 0;
	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (done) break;
			size += value.byteLength;
			if (size > MAX_FRAME_BYTES) {
				void reader.cancel().catch(() => {});
				throw new RealtimeProviderProtocolError(
					"Realtime provider response too large",
				);
			}
			chunks.push(value);
		}
		const bytes = new Uint8Array(size);
		let offset = 0;
		for (const chunk of chunks) {
			bytes.set(chunk, offset);
			offset += chunk.byteLength;
		}
		let value: unknown;
		try {
			value = JSON.parse(decoder.decode(bytes));
		} catch {
			throw new RealtimeProviderProtocolError(
				"Invalid realtime provider response",
			);
		}
		if (!value || typeof value !== "object" || Array.isArray(value))
			throw new RealtimeProviderProtocolError(
				"Invalid realtime provider response",
			);
		return value as Record<string, unknown>;
	} finally {
		reader.releaseLock();
	}
}

export const openRealtimeProvider: RealtimeProviderOpener = (
	transport,
	callbacks,
	signal,
	options,
) => {
	switch (transport.type) {
		case "aws_mqtt":
			return aws(transport, callbacks, signal, options?.clockOffsetMs);
		case "azure_web_pubsub":
			return azure(transport, callbacks, signal);
		case "gcp_firebase_rtdb":
			return firebase(transport, callbacks, signal);
		case "nats":
			return nats(transport, callbacks, signal);
	}
};
