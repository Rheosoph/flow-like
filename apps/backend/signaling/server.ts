#!/usr/bin/env bun
import { randomUUID } from "crypto";
import { serve } from "bun";
import { RedisClient } from "bun";

// -------------------- Config --------------------
const PORT = Number(process.env.PORT || 4444);
const REDIS_URL = process.env.REDIS_URL || "redis://127.0.0.1:6379";
const CHANNEL = process.env.SIGNAL_CHANNEL || "signal:publish";
const PRESENCE_PREFIX = "topic:presence:";
const NODE_ID = process.env.NODE_ID || randomUUID();

// -------------------- Redis ---------------------
const pub = new RedisClient(REDIS_URL);
const sync = new RedisClient(REDIS_URL);

// Per-node heartbeat channel: pub sends a ping every HB_INTERVAL_MS;
// sub listens and updates lastHbAck. If the ack goes stale the sub connection
// has silently dropped and we recreate it.
const HB_CHANNEL = `signal:hb:${NODE_ID}`;
const HB_INTERVAL_MS = 10_000; // publish heartbeat every 10 s
const HB_TIMEOUT_MS = 30_000; // reconnect if ack is older than 30 s

let subClient: RedisClient | null = null;
let subStopped = false;
let subConnecting = false;
let lastHbAck = 0;

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
	if (subStopped || subConnecting) return;
	subConnecting = true;
	try {
		if (subClient) {
			try {
				await (subClient as any).close?.();
			} catch {}
			subClient = null;
		}
		const client = new RedisClient(REDIS_URL);
		// RedisClient.subscribe accepts a string or string[]
		await client.subscribe([CHANNEL, HB_CHANNEL] as any, onSubMessage);
		subClient = client;
		lastHbAck = Date.now(); // treat fresh connect as a received ack
		if (attempt > 0)
			console.log(`[Redis] Subscriber reconnected (attempt ${attempt})`);
	} catch (err) {
		console.error(
			`[Redis] Subscriber connect failed (attempt ${attempt}):`,
			(err as Error).message,
		);
		subClient = null;
		const delay = Math.min(200 * 2 ** attempt, 30_000);
		setTimeout(() => {
			subConnecting = false;
			connectSub(attempt + 1);
		}, delay);
		return;
	}
	subConnecting = false;
}

// Publish heartbeat to our private channel so the sub can prove it's alive.
// Also refresh presence TTLs so quiet rooms don't expire while subscribers are connected.
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
setInterval(() => {
	if (subStopped) return;
	if (!subClient || (lastHbAck > 0 && Date.now() - lastHbAck > HB_TIMEOUT_MS)) {
		console.warn("[Redis] Subscriber heartbeat timeout — reconnecting…");
		subConnecting = false;
		connectSub();
	}
}, HB_INTERVAL_MS);

async function initializeRedis() {
	await sync.duplicate();
	await connectSub(); // initial connect — errors are retried automatically
}

// -------------------- Presence helpers ----------
const topicsLocal = new Map<string, number>(); // local counts per topic

async function getGlobalSubscriberCount(topic: string): Promise<number> {
	try {
		const key = PRESENCE_PREFIX + topic;
		const counts = await sync.hvals(key);
		return (counts || []).reduce((sum, c) => sum + Number(c || 0), 0);
	} catch {
		return 0;
	}
}

const PRESENCE_TTL_S = 90; // seconds; heartbeat refreshes every HB_INTERVAL_MS (~10s)

async function updateTopicPresence(topic: string) {
	try {
		const key = PRESENCE_PREFIX + topic;
		const localCount = topicsLocal.get(topic) || 0;
		if (localCount > 0) {
			await sync.hset(key, NODE_ID, String(localCount));
			await sync.expire(key, PRESENCE_TTL_S);
		} else {
			await sync.hdel(key, NODE_ID);
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
type WSData = { subscribed: Set<string> };

const server = serve<WSData>({
	port: PORT,
	development: false,
	reusePort: true,

	fetch(req, s) {
		let pathname: string;
		try {
			pathname = new URL(req.url).pathname;
		} catch {
			return new Response("Bad Request", { status: 400 });
		}
		if (pathname === "/") {
			const ok = s.upgrade(req, { data: { subscribed: new Set<string>() } });
			return ok
				? undefined
				: new Response("Upgrade failed", {
						status: 426,
						headers: { Upgrade: "websocket" },
					});
		}
		// simple health endpoint
		return new Response("okay", {
			status: 200,
			headers: { "Content-Type": "text/plain" },
		});
	},

	websocket: {
		perMessageDeflate: true,
		idleTimeout: 60,

		open(ws) {
			// nothing fancy; per-connection state is in ws.data
		},

		async message(ws, data) {
			// Reject oversized frames before parsing (DoS protection)
			const MAX_MSG_BYTES = 64 * 1024; // 64 KiB
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
				return;
			}
			if (!msg?.type) return;

			switch (msg.type) {
				case "subscribe": {
					const topics: string[] = Array.isArray(msg.topics) ? msg.topics : [];
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
					const topics: string[] = Array.isArray(msg.topics) ? msg.topics : [];
					for (const t of topics) {
						if (ws.data.subscribed.delete(t)) {
							ws.unsubscribe?.(t); // present in recent Bun builds
							inc(t, -1);
						}
					}
					break;
				}
				case "publish": {
					const topic: string = msg.topic;
					if (!topic) return;
					try {
						const globalCount = await getGlobalSubscriberCount(topic);

						// deliver to local subscribers of `topic` (excluding sender)
						const outbound = { ...msg, clients: globalCount, _origin: NODE_ID };
						ws.publish(topic, JSON.stringify(outbound));

						// fan out to other nodes via Redis
						await pub.publish(CHANNEL, JSON.stringify(outbound));
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
					// ignore unknown types
					break;
			}
		},

		close(ws) {
			// remove from all topics
			for (const t of ws.data.subscribed) {
				inc(t, -1);
			}
			ws.data.subscribed.clear();
		},
	},
});

// Initialize Redis after server is created
initializeRedis()
	.then(() => {
		console.log(`[${NODE_ID}] Bun signaling server on :${server.port}`);
	})
	.catch((err) => {
		console.error("[Redis] Initialization failed:", err);
		process.exit(1);
	});

// Graceful shutdown
async function shutdown() {
	console.log("[Shutdown] Closing server…");
	server.stop?.();
	// drop presence for all topics owned by this node
	for (const t of topicsLocal.keys()) {
		try {
			await sync.hdel(PRESENCE_PREFIX + t, NODE_ID);
		} catch {}
	}
	// close Redis connections
	subStopped = true;
	try {
		await pub.close?.();
	} catch {}
	try {
		await subClient?.close?.();
	} catch {}
	try {
		await sync.close?.();
	} catch {}
	process.exit(0);
}
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
