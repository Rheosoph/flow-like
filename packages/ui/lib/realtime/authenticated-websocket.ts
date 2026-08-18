const REALTIME_PROTOCOL = "flowlike.realtime.v1";
const REALTIME_TOKEN_PROTOCOL_PREFIX = "flowlike.jwt.";
const JWT_PATTERN = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/;
const STATE_SYMBOL = Symbol.for(
	"flow-like.realtime-authenticated-websocket.v1",
);

type TokenRegistration = {
	token: string;
	references: number;
	onAuthFailure?: () => void;
};
type AuthenticatedWebSocketState = {
	original: typeof WebSocket;
	proxy: typeof WebSocket;
	registrations: Map<string, TokenRegistration>;
};

type WebSocketHolder = {
	WebSocket?: typeof WebSocket;
	[STATE_SYMBOL]?: AuthenticatedWebSocketState;
};

function holder(): WebSocketHolder {
	return globalThis as unknown as WebSocketHolder;
}

function validateJwt(token: string): void {
	if (token.length === 0 || token.length > 4096 || !JWT_PATTERN.test(token)) {
		throw new Error("The realtime access token is malformed");
	}
}

function normalizeWebSocketUrl(value: string | URL): string {
	const parsed = new URL(value.toString());
	if (parsed.protocol !== "wss:" && parsed.protocol !== "ws:") {
		throw new Error("Realtime signaling endpoints must use ws:// or wss://");
	}
	return parsed.href;
}

function installAuthenticatedWebSocket(): AuthenticatedWebSocketState {
	const global = holder();
	if (global[STATE_SYMBOL]) {
		if (global.WebSocket !== global[STATE_SYMBOL].proxy) {
			throw new Error("The global WebSocket constructor changed unexpectedly");
		}
		return global[STATE_SYMBOL];
	}
	const NativeWebSocket = global.WebSocket;
	if (!NativeWebSocket) {
		throw new Error("WebSocket is unavailable in this environment");
	}

	const registrations = new Map<string, TokenRegistration>();
	const proxy = new Proxy(NativeWebSocket, {
		construct(target, argumentsList) {
			const url = argumentsList[0] as string | URL;
			const key = normalizeWebSocketUrl(url);
			const registration = registrations.get(key);
			if (!registration) {
				return Reflect.construct(target, argumentsList, target);
			}
			if (argumentsList.length > 1 && argumentsList[1] !== undefined) {
				throw new Error(
					"Authenticated signaling does not accept caller-supplied subprotocols",
				);
			}
			const socket = Reflect.construct(
				target,
				[
					url,
					[
						REALTIME_PROTOCOL,
						`${REALTIME_TOKEN_PROTOCOL_PREFIX}${registration.token}`,
					],
				],
				target,
			) as WebSocket;
			// A close with code 1008 (server-side token expiry/policy) or a close
			// before the socket ever opened (rejected upgrade) signals a stale
			// credential; the registered handler can rotate it before the provider's
			// automatic reconnect.
			let opened = false;
			socket.addEventListener("open", () => {
				opened = true;
			});
			socket.addEventListener("close", (event) => {
				const active = registrations.get(key);
				if (!active?.onAuthFailure) return;
				if (event.code === 1008 || !opened) active.onAuthFailure();
			});
			return socket;
		},
	}) as typeof WebSocket;

	const state = { original: NativeWebSocket, proxy, registrations };
	global.WebSocket = proxy;
	global[STATE_SYMBOL] = state;
	return state;
}

function toBase64Url(bytes: Uint8Array): string {
	let binary = "";
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary)
		.replace(/\+/g, "-")
		.replace(/\//g, "_")
		.replace(/=+$/g, "");
}

/**
 * Give each room a distinct, non-secret signaling URL. y-webrtc shares a
 * signaling socket by URL, while the server deliberately binds one JWT to one
 * app/board topic. The path contains only a one-way room digest, never the JWT.
 */
export async function scopedSignalingUrl(
	endpoint: string,
	room: string,
): Promise<string> {
	if (!room || room.length > 257) throw new Error("Realtime room is invalid");
	const parsed = new URL(normalizeWebSocketUrl(endpoint));
	if (parsed.username || parsed.password || parsed.search || parsed.hash) {
		throw new Error(
			"Realtime signaling endpoints must not contain credentials, query data, or fragments",
		);
	}
	const normalizedPath = parsed.pathname.replace(/\/+$/g, "");
	if (normalizedPath !== "" && normalizedPath !== "/ws") {
		throw new Error("Realtime signaling endpoint path must be / or /ws");
	}
	const digest = await crypto.subtle.digest(
		"SHA-256",
		new TextEncoder().encode(room),
	);
	const roomDigest = toBase64Url(new Uint8Array(digest).slice(0, 24));
	parsed.pathname = `/ws/session/${roomDigest}`;
	return parsed.href;
}

function registerToken(
	url: string,
	token: string,
	onAuthFailure?: () => void,
): () => void {
	validateJwt(token);
	const state = installAuthenticatedWebSocket();
	const key = normalizeWebSocketUrl(url);
	const current = state.registrations.get(key);
	if (current && current.token !== token) {
		throw new Error(
			"A realtime signaling scope already has another credential",
		);
	}
	const registration = current ?? { token, references: 0 };
	if (onAuthFailure) registration.onAuthFailure = onAuthFailure;
	registration.references++;
	state.registrations.set(key, registration);

	let released = false;
	return () => {
		if (released) return;
		released = true;
		if (state.registrations.get(key) !== registration) return;
		registration.references--;
		if (registration.references <= 0) state.registrations.delete(key);
	};
}

function rotateToken(url: string, token: string): void {
	validateJwt(token);
	const state = installAuthenticatedWebSocket();
	const registration = state.registrations.get(normalizeWebSocketUrl(url));
	if (!registration) {
		throw new Error("Cannot rotate an unregistered realtime signaling scope");
	}
	registration.token = token;
}

/** Decodes the JWT `exp` claim into epoch milliseconds without verifying the
 *  signature — verification is the server's job; the client only schedules
 *  proactive rotation. Returns null when no usable expiry is present. */
export function decodeJwtExpiryMs(token: string): number | null {
	const segments = token.split(".");
	if (segments.length !== 3) return null;
	try {
		const payload = JSON.parse(
			atob(segments[1].replace(/-/g, "+").replace(/_/g, "/")),
		) as { exp?: unknown };
		return typeof payload.exp === "number" && Number.isFinite(payload.exp)
			? payload.exp * 1000
			: null;
	} catch {
		return null;
	}
}

export type AuthenticatedSignaling = {
	signaling: string[];
	rotate: (nextToken: string) => void;
	dispose: () => void;
};

export async function prepareAuthenticatedSignaling(
	endpoints: string[],
	room: string,
	token: string,
	onAuthFailure?: () => void,
): Promise<AuthenticatedSignaling> {
	validateJwt(token);
	if (endpoints.length === 0 || endpoints.length > 5) {
		throw new Error("Between one and five signaling endpoints are required");
	}
	const signaling = await Promise.all(
		endpoints.map((endpoint) => scopedSignalingUrl(endpoint, room)),
	);
	if (new Set(signaling).size !== signaling.length) {
		throw new Error("Realtime signaling endpoints must be unique");
	}

	const releases: Array<() => void> = [];
	try {
		for (const url of signaling) {
			releases.push(registerToken(url, token, onAuthFailure));
		}
	} catch (error) {
		for (const release of releases) release();
		throw error;
	}
	return {
		signaling,
		rotate: (nextToken: string) => {
			for (const url of signaling) rotateToken(url, nextToken);
		},
		dispose: () => {
			for (const release of releases) release();
		},
	};
}
