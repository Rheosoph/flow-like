import { describe, expect, test } from "bun:test";
import type { Subprocess } from "bun";
import { SignJWT, exportSPKI, generateKeyPair } from "jose";
import { REALTIME_PROTOCOL, REALTIME_TOKEN_PROTOCOL_PREFIX } from "../auth";
import { parseFanoutConfig } from "../redis";

const SERVER_PATH = new URL("../server.ts", import.meta.url).pathname;
const SERVER_CWD = new URL("..", import.meta.url).pathname;

type SignalMessage = { type: string; [key: string]: unknown };

type RedisSpy = { port: number; connections: number; stop(): void };

// Accepts TCP connections and never answers, so any constructed Redis client
// shows up as a connection while the signaling server stays unblocked.
function startRedisSpy(): RedisSpy {
	const spy = { port: 0, connections: 0, stop: () => {} };
	const listener = Bun.listen({
		hostname: "127.0.0.1",
		port: 0,
		socket: {
			open() {
				spy.connections += 1;
			},
			data() {},
		},
	});
	spy.port = listener.port;
	spy.stop = () => listener.stop(true);
	return spy;
}

function spawnSignaling(env: Record<string, string>) {
	return Bun.spawn(["bun", SERVER_PATH], {
		cwd: SERVER_CWD,
		env: {
			PATH: process.env.PATH,
			HOME: process.env.HOME,
			PORT: "0",
			NODE_ENV: "development",
			REALTIME_ALLOW_INSECURE_LOCAL_DEV: "true",
			...env,
		},
		stdout: "pipe",
		stderr: "inherit",
	});
}

async function readListeningPort(
	stream: ReadableStream<Uint8Array>,
): Promise<number> {
	const decoder = new TextDecoder();
	let buffered = "";
	for await (const chunk of stream) {
		buffered += decoder.decode(chunk, { stream: true });
		const match = buffered.match(/on :(\d+)/);
		if (match) return Number(match[1]);
	}
	throw new Error(`signaling server never reported a port: ${buffered}`);
}

// SIGKILL: a Redis client stuck mid-connect can stall the graceful shutdown.
async function stopSignaling(child: Subprocess): Promise<void> {
	child.kill("SIGKILL");
	await child.exited;
}

async function waitFor<T>(
	probe: () => T | undefined,
	label: string,
): Promise<T> {
	for (let attempt = 0; attempt < 200; attempt++) {
		const value = probe();
		if (value !== undefined) return value;
		await Bun.sleep(25);
	}
	throw new Error(`timed out waiting for ${label}`);
}

function connect(port: number): Promise<WebSocket> {
	return new Promise((resolve, reject) => {
		const socket = new WebSocket(`ws://127.0.0.1:${port}/`);
		socket.onopen = () => resolve(socket);
		socket.onerror = () => reject(new Error("websocket upgrade rejected"));
	});
}

function collect(socket: WebSocket): SignalMessage[] {
	const messages: SignalMessage[] = [];
	socket.onmessage = (event) => {
		messages.push(JSON.parse(String(event.data)) as SignalMessage);
	};
	return messages;
}

function received(messages: SignalMessage[], type: string) {
	return messages.find((message) => message.type === type);
}

async function subscribe(
	socket: WebSocket,
	messages: SignalMessage[],
	topic: string,
): Promise<void> {
	socket.send(JSON.stringify({ type: "subscribe", topics: [topic] }));
	socket.send(JSON.stringify({ type: "ping" }));
	await waitFor(() => received(messages, "pong"), `${topic} subscription`);
}

describe("fan-out configuration", () => {
	test("defaults to redis fan-out and demands an explicit endpoint", () => {
		expect(() => parseFanoutConfig({})).toThrow(/requires REDIS_URL/);
		expect(parseFanoutConfig({ REDIS_URL: "redis://127.0.0.1:6379" })).toEqual({
			mode: "redis",
			redis: {
				authMode: "local",
				url: "redis://127.0.0.1:6379",
				hostname: "127.0.0.1",
				clusterMode: false,
			},
		});
	});

	test("carries no Redis configuration in local mode", () => {
		expect(parseFanoutConfig({ REALTIME_FANOUT_MODE: "local" })).toEqual({
			mode: "local",
		});
	});

	test("rejects an unknown fan-out mode", () => {
		expect(() =>
			parseFanoutConfig({ REALTIME_FANOUT_MODE: "cluster" }),
		).toThrow(/REALTIME_FANOUT_MODE must be local or redis/);
	});
});

describe("local fan-out mode", () => {
	test("serves without Redis and relays between local sockets", async () => {
		const spy = startRedisSpy();
		const server = spawnSignaling({
			REALTIME_FANOUT_MODE: "local",
			REDIS_URL: `redis://127.0.0.1:${spy.port}`,
		});
		try {
			const port = await readListeningPort(server.stdout);

			const health = await fetch(`http://127.0.0.1:${port}/health`);
			expect(health.status).toBe(200);
			expect(await health.text()).toBe("okay");
			const ready = await fetch(`http://127.0.0.1:${port}/ready`);
			expect(ready.status).toBe(200);

			const sender = await connect(port);
			const peer = await connect(port);
			const senderMessages = collect(sender);
			const peerMessages = collect(peer);
			await subscribe(peer, peerMessages, "room-1");
			await subscribe(sender, senderMessages, "room-1");

			sender.send(
				JSON.stringify({
					type: "publish",
					topic: "room-1",
					data: { hello: "world" },
				}),
			);
			const relayed = await waitFor(
				() => received(peerMessages, "publish"),
				"relayed publish",
			);
			expect(relayed.data).toEqual({ hello: "world" });
			expect(relayed.clients).toBe(2);
			expect(received(senderMessages, "publish")).toBeUndefined();
			expect(spy.connections).toBe(0);

			sender.close();
			peer.close();
		} finally {
			await stopSignaling(server);
			spy.stop();
		}
	}, 20_000);
});

describe("authenticated connection cap", () => {
	test("rejects upgrades beyond REALTIME_MAX_CONNECTIONS_PER_SUB with 429 and frees slots on close", async () => {
		const { privateKey, publicKey } = await generateKeyPair("ES256");
		const publicKeyPem = await exportSPKI(publicKey);
		const now = Math.floor(Date.now() / 1000);
		const token = await new SignJWT({
			typ: "realtime",
			scope: "realtime.read",
			app_id: "app_123",
			board_id: "board_456",
		})
			.setProtectedHeader({ alg: "ES256", kid: "backend-es256-v1" })
			.setIssuer("flow-like")
			.setAudience("y-webrtc")
			.setSubject("user_789")
			.setJti("token_123")
			.setIssuedAt(now)
			.setNotBefore(now - 30)
			.setExpirationTime(now + 3600)
			.sign(privateKey);

		const server = spawnSignaling({
			REALTIME_FANOUT_MODE: "local",
			REALTIME_ALLOW_INSECURE_LOCAL_DEV: "false",
			BACKEND_PUB: Buffer.from(publicKeyPem).toString("base64"),
			BACKEND_KID: "backend-es256-v1",
			REALTIME_ALLOWED_ORIGINS: "https://app.example.com,tauri://localhost",
			REALTIME_MAX_CONNECTIONS_PER_SUB: "2",
		});
		try {
			const port = await readListeningPort(server.stdout);
			const upgradeHeaders = {
				Origin: "tauri://localhost",
				"Sec-WebSocket-Protocol": `${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			};
			const openAuthenticated = (): Promise<WebSocket> =>
				new Promise((resolve, reject) => {
					const socket = new WebSocket(`ws://127.0.0.1:${port}/`, {
						protocols: [
							REALTIME_PROTOCOL,
							`${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
						],
						headers: { Origin: "tauri://localhost" },
					});
					socket.onopen = () => resolve(socket);
					socket.onerror = () =>
						reject(new Error("websocket upgrade rejected"));
				});

			const first = await openAuthenticated();
			const second = await openAuthenticated();

			// Authorization succeeds but the subject is at its cap.
			const overCap = await fetch(`http://127.0.0.1:${port}/`, {
				headers: upgradeHeaders,
			});
			expect(overCap.status).toBe(429);

			second.close();
			// Once the closed socket releases its slot, the same request passes the
			// cap and fails only as a non-WebSocket request (426).
			let released: number | undefined;
			for (let attempt = 0; attempt < 100; attempt++) {
				const probe = await fetch(`http://127.0.0.1:${port}/`, {
					headers: upgradeHeaders,
				});
				released = probe.status;
				if (released !== 429) break;
				await Bun.sleep(25);
			}
			expect(released).toBe(426);
			first.close();
		} finally {
			await stopSignaling(server);
		}
	}, 20_000);
});

describe("redis fan-out mode", () => {
	test("connects to the configured endpoint", async () => {
		const spy = startRedisSpy();
		const server = spawnSignaling({
			REALTIME_FANOUT_MODE: "redis",
			REDIS_URL: `redis://127.0.0.1:${spy.port}`,
		});
		try {
			await waitFor(
				() => (spy.connections > 0 ? spy.connections : undefined),
				"a Redis connection attempt",
			);
		} finally {
			await stopSignaling(server);
			spy.stop();
		}
	}, 20_000);

	test("stays live before fan-out is ready: /health 200, /ready 503", async () => {
		// The spy never answers, so the fan-out clients never become ready.
		const spy = startRedisSpy();
		const server = spawnSignaling({
			REALTIME_FANOUT_MODE: "redis",
			REDIS_URL: `redis://127.0.0.1:${spy.port}`,
		});
		try {
			const port = await readListeningPort(server.stdout);

			const health = await fetch(`http://127.0.0.1:${port}/health`);
			expect(health.status).toBe(200);
			expect(await health.text()).toBe("okay");

			const ready = await fetch(`http://127.0.0.1:${port}/ready`);
			expect(ready.status).toBe(503);
			expect(await ready.text()).toBe("redis unavailable");

			// New upgrades stay refused while fan-out is unready.
			const upgrade = await fetch(`http://127.0.0.1:${port}/`);
			expect(upgrade.status).toBe(503);
		} finally {
			await stopSignaling(server);
			spy.stop();
		}
	}, 20_000);
});
