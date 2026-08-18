#!/usr/bin/env bun
import { randomUUID } from "crypto";
import { serve } from "bun";
import {
	REALTIME_PROTOCOL,
	createRealtimeAuthenticator,
	parseRealtimeAuthConfig,
} from "./auth";
import { ConnectionRateLimiter } from "./limits";
import {
	type SignalRedisClient,
	attachRedisLifecycleLogging,
	closeSignalRedisClient,
	createSignalRedisClient,
	parseFanoutConfig,
} from "./redis";

// -------------------- Config --------------------
const PORT = Number(process.env.PORT || 4444);
const FANOUT = parseFanoutConfig();
const REDIS_CONFIG = FANOUT.mode === "redis" ? FANOUT.redis : null;
const CHANNEL = process.env.SIGNAL_CHANNEL || "signal:publish";
const PRESENCE_PREFIX = "topic:presence:";
const NODE_ID = process.env.NODE_ID || randomUUID();
const AUTH_CONFIG = parseRealtimeAuthConfig();
const authorizeUpgrade = await createRealtimeAuthenticator(AUTH_CONFIG);

const MAX_MSG_BYTES = 64 * 1024;
const MAX_TOPICS_AUTHENTICATED = 1;
const MAX_TOPICS_INSECURE_LOCAL_DEV = 8;
const LOCAL_TOPIC_PATTERN = /^[A-Za-z0-9:_-]{1,257}$/;

function parseConnectionsPerSubject(raw: string | undefined): number {
	if (raw === undefined || raw.trim() === "") return 16;
	const value = Number(raw.trim());
	if (!Number.isSafeInteger(value) || value < 1 || value > 10_000) {
		throw new Error(
			"REALTIME_MAX_CONNECTIONS_PER_SUB must be an integer between 1 and 10000",
		);
	}
	return value;
}

const MAX_CONNECTIONS_PER_SUB = parseConnectionsPerSubject(
	process.env.REALTIME_MAX_CONNECTIONS_PER_SUB,
);
const liveConnectionsPerSubject = new Map<string, number>();

function releaseSubjectConnection(subject: string) {
	const live = liveConnectionsPerSubject.get(subject);
	if (live === undefined) return;
	if (live <= 1) liveConnectionsPerSubject.delete(subject);
	else liveConnectionsPerSubject.set(subject, live - 1);
}

// -------------------- Redis ---------------------
// In `local` fan-out mode no client is constructed, so nothing is dialled.
const pub = REDIS_CONFIG
	? createSignalRedisClient(REDIS_CONFIG, "publisher")
	: null;
const sync = REDIS_CONFIG
	? createSignalRedisClient(REDIS_CONFIG, "presence")
	: null;
if (pub) attachRedisLifecycleLogging(pub, "publisher");
if (sync) attachRedisLifecycleLogging(sync, "presence");
if (!REDIS_CONFIG && process.env.REDIS_URL?.trim()) {
	console.warn(
		"[Fanout] REALTIME_FANOUT_MODE=local ignores REDIS_URL; messages reach this replica's sockets only",
	);
}

// Per-node heartbeat channel: pub sends a ping every HB_INTERVAL_MS;
// sub listens and updates lastHbAck. If the ack goes stale the sub connection
// has silently dropped and we recreate it.
const HB_CHANNEL = `signal:hb:${NODE_ID}`;
const HB_INTERVAL_MS = 10_000; // publish heartbeat every 10 s
const HB_TIMEOUT_MS = 30_000; // reconnect if ack is older than 30 s

let subClient: SignalRedisClient | null = null;
let subStopped = false;
let subConnecting = false;
let subGeneration = 0;
let lastHbAck = 0;

const SUB_ATTEMPT_TIMEOUT_MS = 30_000;

function withDeadline<T>(
	promise: Promise<T>,
	ms: number,
	label: string,
): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(
			() => reject(new Error(`${label} timed out after ${ms}ms`)),
			ms,
		);
		promise.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(error) => {
				clearTimeout(timer);
				reject(error);
			},
		);
	});
}

async function discardSubClient(client: SignalRedisClient | null) {
	try {
		await closeSignalRedisClient(client);
	} catch {}
}

function onSubMessage(raw: string, ch: string) {
	if (ch === HB_CHANNEL) {
		lastHbAck = Date.now();
		return;
	}
	try {
		const message = JSON.parse(raw);
		if (
			message?.type === "publish" &&
			message.topic &&
			message._origin !== NODE_ID
		) {
			server.publish(message.topic, JSON.stringify(message));
		}
	} catch (err) {
		console.error("[Redis] Failed to process message:", (err as Error).message);
	}
}

async function connectSub(attempt = 0): Promise<void> {
	if (!REDIS_CONFIG || subStopped || subConnecting) return;
	subConnecting = true;
	const generation = ++subGeneration;
	const previous = subClient;
	subClient = null;
	if (previous) await discardSubClient(previous);

	const candidate = createSignalRedisClient(REDIS_CONFIG, "subscriber");
	attachRedisLifecycleLogging(candidate, "subscriber");
	// node-redis retries connect() internally without bound, so an attempt can
	// settle minutes late during a slow Redis recovery. The deadline bounds the
	// attempt; a candidate that settles after it is discarded, never assigned.
	const attemptSettled = (async () => {
		await candidate.connect();
		await candidate.subscribe([CHANNEL, HB_CHANNEL], onSubMessage);
	})();
	try {
		await withDeadline(
			attemptSettled,
			SUB_ATTEMPT_TIMEOUT_MS,
			"[Redis] Subscriber connect",
		);
	} catch (err) {
		console.error(
			`[Redis] Subscriber connect failed (attempt ${attempt}):`,
			(err as Error).message,
		);
		void discardSubClient(candidate);
		attemptSettled.then(
			() => void discardSubClient(candidate),
			() => void discardSubClient(candidate),
		);
		if (subStopped) {
			subConnecting = false;
			return;
		}
		const delay = Math.min(200 * 2 ** attempt, 30_000);
		setTimeout(() => {
			subConnecting = false;
			connectSub(attempt + 1);
		}, delay);
		return;
	}
	if (generation !== subGeneration || subStopped) {
		await discardSubClient(candidate);
		subConnecting = false;
		return;
	}
	if (subClient) await discardSubClient(subClient);
	subClient = candidate;
	lastHbAck = Date.now(); // treat fresh connect as a received ack
	if (attempt > 0)
		console.log(`[Redis] Subscriber reconnected (attempt ${attempt})`);
	subConnecting = false;
}

// Publish heartbeat to our private channel so the sub can prove it's alive.
// Also refresh presence TTLs so quiet rooms don't expire while subscribers are connected.
if (pub) {
	setInterval(async () => {
		if (subStopped) return;
		try {
			await pub.publish(HB_CHANNEL, "1");
		} catch (err) {
			console.warn("[Redis] Heartbeat publish failed:", (err as Error).message);
		}
		// Refresh every active topic's presence TTL before it expires (TTL is PRESENCE_TTL_S).
		for (const topic of topicsLocal.keys()) {
			updateTopicPresence(topic);
		}
	}, HB_INTERVAL_MS);

	// Watcher: if the ack is stale the sub TCP connection has silently dropped.
	// connectSub owns its own deadline and retry loop, so the watcher never
	// resets subConnecting — it only starts a new attempt when none is running.
	setInterval(() => {
		if (subStopped || subConnecting) return;
		if (
			!subClient ||
			(lastHbAck > 0 && Date.now() - lastHbAck > HB_TIMEOUT_MS)
		) {
			console.warn("[Redis] Subscriber heartbeat timeout — reconnecting…");
			connectSub();
		}
	}, HB_INTERVAL_MS);
}

async function initializeFanout() {
	if (!pub || !sync) return;
	await Promise.all([pub.connect(), sync.connect()]);
	await Promise.all([pub.ping(), sync.ping()]);
	await connectSub(); // initial connect — errors are retried automatically
}

function fanoutIsReady(): boolean {
	if (!REDIS_CONFIG) return true;
	return (
		pub?.isReady === true &&
		sync?.isReady === true &&
		subClient?.isReady === true
	);
}

// -------------------- Presence helpers ----------
const topicsLocal = new Map<string, number>(); // local counts per topic

async function getGlobalSubscriberCount(topic: string): Promise<number> {
	if (!sync) return server.subscriberCount(topic);
	try {
		const key = PRESENCE_PREFIX + topic;
		const counts = await sync.hVals(key);
		return (counts || []).reduce((sum, c) => sum + Number(c || 0), 0);
	} catch {
		return 0;
	}
}

const PRESENCE_TTL_S = 90; // seconds; heartbeat refreshes every HB_INTERVAL_MS (~10s)

async function updateTopicPresence(topic: string) {
	if (!sync) return;
	try {
		const key = PRESENCE_PREFIX + topic;
		const localCount = topicsLocal.get(topic) || 0;
		if (localCount > 0) {
			await sync.hSet(key, NODE_ID, String(localCount));
			await sync.expire(key, PRESENCE_TTL_S);
		} else {
			await sync.hDel(key, NODE_ID);
		}
	} catch (err) {
		console.error("[Presence] Update failed:", (err as Error).message);
	}
}

function inc(topic: string, delta: 1 | -1) {
	const next = Math.max(0, (topicsLocal.get(topic) || 0) + delta);
	if (next === 0) topicsLocal.delete(topic);
	else topicsLocal.set(topic, next);
	// fire & forget
	updateTopicPresence(topic);
}

// -------------------- WebSocket server ----------
type WSData = {
	subscribed: Set<string>;
	allowedTopic: string | null;
	subject: string | null;
	insecureLocalDev: boolean;
	expiresAtMs: number | null;
	expiryTimer: ReturnType<typeof setTimeout> | null;
	rateLimiter: ConnectionRateLimiter;
};

function closeForPolicy(ws: { close(code?: number, reason?: string): void }) {
	ws.close(1008, "Policy violation");
}

function topicIsAuthorized(data: WSData, topic: unknown): topic is string {
	if (typeof topic !== "string") return false;
	if (data.allowedTopic !== null) return topic === data.allowedTopic;
	return data.insecureLocalDev && LOCAL_TOPIC_PATTERN.test(topic);
}

function noStoreResponse(body: string, status: number, extraHeaders = {}) {
	return new Response(body, {
		status,
		headers: {
			"Cache-Control": "no-store",
			"Content-Type": "text/plain; charset=utf-8",
			...extraHeaders,
		},
	});
}

const server = serve<WSData>({
	port: PORT,
	development: false,
	reusePort: true,

	async fetch(req, s) {
		let pathname: string;
		try {
			pathname = new URL(req.url).pathname;
		} catch {
			return new Response("Bad Request", { status: 400 });
		}
		// Liveness only: a Redis outage must degrade fan-out, not restart the
		// process and kill live sockets. Readiness gating lives on /ready.
		if (pathname === "/health") {
			return noStoreResponse("okay", 200);
		}
		if (pathname === "/ready") {
			const ready = fanoutIsReady();
			return noStoreResponse(
				ready ? "okay" : "redis unavailable",
				ready ? 200 : 503,
			);
		}
		if (
			pathname === "/" ||
			pathname === "/ws" ||
			pathname === "/ws/" ||
			/^\/ws\/session\/[A-Za-z0-9_-]{32}$/.test(pathname)
		) {
			if (!fanoutIsReady()) {
				return noStoreResponse("Service Unavailable", 503, {
					"Retry-After": "5",
				});
			}
			let authorization;
			try {
				authorization = await authorizeUpgrade(
					req.headers.get("origin"),
					req.headers.get("sec-websocket-protocol"),
				);
			} catch {
				return noStoreResponse("Unauthorized", 401);
			}

			const subject = authorization.subject;
			if (subject !== null) {
				const live = liveConnectionsPerSubject.get(subject) ?? 0;
				if (live >= MAX_CONNECTIONS_PER_SUB) {
					return noStoreResponse("Too Many Requests", 429, {
						"Retry-After": "5",
					});
				}
				liveConnectionsPerSubject.set(subject, live + 1);
			}

			const ok = s.upgrade(req, {
				data: {
					subscribed: new Set<string>(),
					allowedTopic: authorization.allowedTopic,
					subject,
					insecureLocalDev: authorization.insecureLocalDev,
					expiresAtMs: authorization.expiresAtMs,
					expiryTimer: null,
					rateLimiter: new ConnectionRateLimiter(),
				},
				headers: authorization.insecureLocalDev
					? undefined
					: { "Sec-WebSocket-Protocol": REALTIME_PROTOCOL },
			});
			if (ok) return undefined;
			if (subject !== null) releaseSubjectConnection(subject);
			return noStoreResponse("Upgrade failed", 426, { Upgrade: "websocket" });
		}
		return noStoreResponse("Not Found", 404);
	},

	websocket: {
		perMessageDeflate: false,
		idleTimeout: 60,
		maxPayloadLength: MAX_MSG_BYTES,
		backpressureLimit: 1024 * 1024,
		closeOnBackpressureLimit: true,

		open(ws) {
			if (ws.data.expiresAtMs !== null) {
				const remaining = ws.data.expiresAtMs - Date.now();
				if (remaining <= 0) {
					closeForPolicy(ws);
					return;
				}
				ws.data.expiryTimer = setTimeout(() => {
					closeForPolicy(ws);
				}, remaining);
			}
		},

		async message(ws, data) {
			if (
				(typeof data === "string" ? data.length : (data as Buffer).byteLength) >
				MAX_MSG_BYTES
			) {
				ws.close(1009, "Message too large");
				return;
			}
			let msg: any;
			try {
				msg = JSON.parse(
					typeof data === "string" ? data : Buffer.from(data).toString("utf8"),
				);
			} catch {
				ws.close(1007, "Invalid JSON");
				return;
			}
			if (!msg || typeof msg !== "object" || typeof msg.type !== "string") {
				closeForPolicy(ws);
				return;
			}
			if (ws.data.expiresAtMs !== null && Date.now() >= ws.data.expiresAtMs) {
				closeForPolicy(ws);
				return;
			}
			if (!ws.data.rateLimiter.consume(msg.type === "publish")) {
				ws.close(1013, "Rate limit exceeded");
				return;
			}

			switch (msg.type) {
				case "subscribe": {
					if (!Array.isArray(msg.topics)) {
						closeForPolicy(ws);
						return;
					}
					const topics: unknown[] = msg.topics;
					const maximumTopics = ws.data.insecureLocalDev
						? MAX_TOPICS_INSECURE_LOCAL_DEV
						: MAX_TOPICS_AUTHENTICATED;
					if (
						topics.length > maximumTopics ||
						new Set(topics).size !== topics.length ||
						topics.some((topic) => !topicIsAuthorized(ws.data, topic)) ||
						new Set([...ws.data.subscribed, ...topics]).size > maximumTopics
					) {
						closeForPolicy(ws);
						return;
					}
					for (const t of topics) {
						if (typeof t !== "string") continue;
						if (!ws.data.subscribed.has(t)) {
							ws.subscribe(t);
							ws.data.subscribed.add(t);
							inc(t, 1);
						}
					}
					break;
				}
				case "unsubscribe": {
					if (
						!Array.isArray(msg.topics) ||
						msg.topics.some(
							(topic: unknown) => !topicIsAuthorized(ws.data, topic),
						)
					) {
						closeForPolicy(ws);
						return;
					}
					const topics: string[] = msg.topics;
					for (const t of topics) {
						if (ws.data.subscribed.delete(t)) {
							ws.unsubscribe?.(t); // present in recent Bun builds
							inc(t, -1);
						}
					}
					break;
				}
				case "publish": {
					const topic: unknown = msg.topic;
					if (
						!topicIsAuthorized(ws.data, topic) ||
						!ws.data.subscribed.has(topic)
					) {
						closeForPolicy(ws);
						return;
					}
					try {
						const globalCount = await getGlobalSubscriberCount(topic);

						// deliver to local subscribers of `topic` (excluding sender)
						const outbound = JSON.stringify({
							type: "publish",
							topic,
							data: msg.data,
							clients: globalCount,
							_origin: NODE_ID,
						});
						ws.publish(topic, outbound);

						// fan out to other nodes via Redis
						if (pub) await pub.publish(CHANNEL, outbound);
					} catch (err) {
						console.error("[Publish] Error:", (err as Error).message);
					}
					break;
				}
				case "ping": {
					ws.send(JSON.stringify({ type: "pong" }));
					break;
				}
				default:
					closeForPolicy(ws);
					return;
			}
		},

		close(ws) {
			if (ws.data.expiryTimer !== null) clearTimeout(ws.data.expiryTimer);
			if (ws.data.subject !== null) releaseSubjectConnection(ws.data.subject);
			// remove from all topics
			for (const t of ws.data.subscribed) {
				inc(t, -1);
			}
			ws.data.subscribed.clear();
		},
	},
});

// The process is live (and /health green) as soon as it listens; fan-out
// readiness is reported separately once Redis is up.
console.log(
	`[${NODE_ID}] Bun signaling server on :${server.port} (fanout=${FANOUT.mode})`,
);

// Initialize Redis after server is created
initializeFanout()
	.then(() => {
		if (REDIS_CONFIG) console.log("[Fanout] Redis fan-out ready");
	})
	.catch((err) => {
		console.error("[Fanout] Initialization failed:", err);
		process.exit(1);
	});

// Graceful shutdown
async function shutdown() {
	console.log("[Shutdown] Closing server…");
	server.stop?.();
	// drop presence for all topics owned by this node
	if (sync) {
		for (const t of topicsLocal.keys()) {
			try {
				await sync.hDel(PRESENCE_PREFIX + t, NODE_ID);
			} catch {}
		}
	}
	// close Redis connections
	subStopped = true;
	try {
		await closeSignalRedisClient(pub);
	} catch {}
	try {
		await closeSignalRedisClient(subClient);
	} catch {}
	try {
		await closeSignalRedisClient(sync);
	} catch {}
	process.exit(0);
}
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
