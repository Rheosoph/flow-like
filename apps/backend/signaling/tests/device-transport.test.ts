import { expect, test } from "bun:test";
import { type Socket, createServer } from "node:net";
import type { Subprocess } from "bun";
import { SignJWT, exportSPKI, generateKeyPair } from "jose";
import {
	DEVICE_SIGNALING_AUDIENCE,
	DEVICE_SIGNALING_JOSE_TYPE,
	DEVICE_SIGNALING_PROTOCOL,
	DEVICE_SIGNALING_SCOPE,
} from "../device-frames";

const keys = await generateKeyPair("ES256");
const publicKey = Buffer.from(await exportSPKI(keys.publicKey)).toString(
	"base64",
);
const origin = "https://app.example.com";
const channels: Set<WebSocket> = new Set();
const children: Set<Subprocess> = new Set();

async function eventually<T>(
	probe: () => T | undefined,
	label: string,
): Promise<T> {
	for (let i = 0; i < 200; i++) {
		const value = probe();
		if (value !== undefined) return value;
		await Bun.sleep(20);
	}
	throw new Error(`Timed out: ${label}`);
}
async function credential(
	role: "device" | "controller",
	participant: string,
	epoch = 1,
	lifetime = 300,
) {
	const now = Math.floor(Date.now() / 1000);
	return new SignJWT({
		iss: "flow-like",
		aud: DEVICE_SIGNALING_AUDIENCE,
		sub: role === "device" ? "device" : "owner",
		typ: "device_signaling",
		scope: DEVICE_SIGNALING_SCOPE,
		device_id: "device",
		device_auth_epoch: epoch,
		role,
		participant_id: participant,
		iat: now,
		nbf: now,
		exp: now + lifetime,
		jti: crypto.randomUUID(),
	})
		.setProtectedHeader({
			alg: "ES256",
			kid: "backend-es256-v1",
			typ: DEVICE_SIGNALING_JOSE_TYPE,
		})
		.sign(keys.privateKey);
}
async function replica(redisPort?: number) {
	const child = Bun.spawn(
		["bun", new URL("../server.ts", import.meta.url).pathname],
		{
			cwd: new URL("..", import.meta.url).pathname,
			env: {
				PATH: process.env.PATH,
				PORT: "0",
				SIGNAL_HOST: "127.0.0.1",
				REALTIME_FANOUT_MODE: redisPort ? "redis" : "local",
				REDIS_URL: redisPort ? `redis://127.0.0.1:${redisPort}` : undefined,
				REALTIME_ALLOWED_ORIGINS: origin,
				BACKEND_PUB: publicKey,
				BACKEND_KID: "backend-es256-v1",
			},
			stdout: "pipe",
			stderr: "pipe",
		},
	);
	children.add(child);
	let stdout = "";
	let port: number | undefined;
	void (async () => {
		for await (const chunk of child.stdout) {
			stdout += new TextDecoder().decode(chunk);
			const match = stdout.match(/on :(\d+)/);
			if (match) port = Number(match[1]);
		}
	})();
	const listen = await eventually(() => port, "signaling listener");
	for (let i = 0; i < 200; i++) {
		if ((await fetch(`http://127.0.0.1:${listen}/ready`)).ok)
			return { child, port: listen };
		await Bun.sleep(20);
	}
	throw new Error("Signaling fanout did not become ready");
}
type Message = { type: string; [key: string]: unknown };
async function connection(
	port: number,
	role: "device" | "controller",
	participant: string,
	epoch = 1,
	lifetime = 300,
) {
	const token = await credential(role, participant, epoch, lifetime);
	const socket = new WebSocket(`ws://127.0.0.1:${port}/ws/devices`, {
		protocols: [DEVICE_SIGNALING_PROTOCOL, `flowlike.jwt.${token}`],
		headers: role === "controller" ? { Origin: origin } : {},
	});
	channels.add(socket);
	const messages: Message[] = [];
	let closed: number | undefined;
	socket.onmessage = (event) =>
		messages.push(JSON.parse(String(event.data)) as Message);
	socket.onclose = (event) => {
		closed = event.code;
	};
	await eventually(
		() => messages.find((message) => message.type === "ready"),
		"authenticated inbox readiness",
	);
	expect(socket.protocol).toBe(DEVICE_SIGNALING_PROTOCOL);
	return { socket, messages, closed: () => closed };
}
async function cleanup() {
	for (const socket of channels) socket.close();
	channels.clear();
	for (const child of children) {
		child.kill("SIGKILL");
		await child.exited;
	}
	children.clear();
}
const opaque = Buffer.from([0, 255, 1, 2, 3]).toString("base64url");
const frame = (to: string) =>
	JSON.stringify({ type: "frame", to, channel: "noise", payload: opaque });

test("dedicated management sockets enforce routing, expiry and replacement credentials", async () => {
	try {
		const server = await replica();
		const device = await connection(server.port, "device", "device");
		const controller = await connection(
			server.port,
			"controller",
			"controller",
		);
		const isolated = await connection(server.port, "device", "device", 2);
		controller.socket.send(frame("device"));
		const incoming = await eventually(
			() => device.messages.find((message) => message.type === "frame"),
			"opaque controller frame",
		);
		expect(incoming).toEqual({
			type: "frame",
			to: "device",
			channel: "noise",
			payload: opaque,
			from: "controller",
			from_role: "controller",
		});
		expect(
			isolated.messages.filter((message) => message.type === "frame"),
		).toHaveLength(0);
		device.socket.send(frame("controller"));
		expect(
			(
				await eventually(
					() => controller.messages.find((message) => message.type === "frame"),
					"opaque device reply",
				)
			).from_role,
		).toBe("device");
		controller.socket.send(
			JSON.stringify({ type: "subscribe", topics: ["device-mgmt:v1:other"] }),
		);
		expect(
			await eventually(controller.closed, "disallowed room operation"),
		).toBe(1008);
		const expiring = await connection(
			server.port,
			"controller",
			"renewing",
			1,
			2,
		);
		expect(
			await eventually(expiring.closed, "transport credential expiry"),
		).toBe(1008);
		const renewed = await connection(server.port, "controller", "renewing");
		renewed.socket.send(frame("device"));
		await eventually(
			() => device.messages.find((message) => message.from === "renewing"),
			"replacement credential routing",
		);
		const policy = await connection(server.port, "controller", "policy");
		policy.socket.send(frame("other-controller"));
		expect(
			await eventually(policy.closed, "controller-to-controller denial"),
		).toBe(1008);
	} finally {
		await cleanup();
	}
}, 20_000);

// This in-memory RESP3 fixture implements the Redis commands used by signaling.
// It exercises the real Redis clients and separate server processes without a
// machine-global Redis installation or durable storage.
async function redisFixture() {
	const peers = new Map<Socket, Set<string>>();
	const bulk = (value: string) =>
		`$${Buffer.byteLength(value)}\r\n${value}\r\n`;
	const server = createServer((socket) => {
		peers.set(socket, new Set());
		let buffer = Buffer.alloc(0);
		socket.on("close", () => peers.delete(socket));
		socket.on("error", () => {});
		socket.on("data", (chunk) => {
			buffer = Buffer.concat([buffer, chunk]);
			for (;;) {
				const line = buffer.indexOf("\r\n");
				if (line < 0) return;
				if (buffer[0] !== 42) {
					socket.destroy();
					return;
				}
				const count = Number(buffer.subarray(1, line).toString());
				let offset = line + 2;
				const args: string[] = [];
				for (let i = 0; i < count; i++) {
					const end = buffer.indexOf("\r\n", offset);
					if (end < 0) return;
					if (buffer[offset] !== 36) {
						socket.destroy();
						return;
					}
					const length = Number(buffer.subarray(offset + 1, end).toString());
					if (buffer.length < end + 2 + length + 2) return;
					args.push(buffer.subarray(end + 2, end + 2 + length).toString());
					offset = end + 2 + length + 2;
				}
				buffer = buffer.subarray(offset);
				const command = args[0]?.toUpperCase();
				if (command === "HELLO")
					socket.write(
						`%3\r\n${bulk("server")}${bulk("redis")}${bulk("version")}${bulk("7.4.0")}${bulk("proto")}:3\r\n`,
					);
				else if (command === "PING") socket.write("+PONG\r\n");
				else if (command === "SUBSCRIBE") {
					for (const channel of args.slice(1)) {
						peers.get(socket)?.add(channel);
						socket.write(
							`>3\r\n${bulk("subscribe")}${bulk(channel)}:${peers.get(socket)?.size ?? 0}\r\n`,
						);
					}
				} else if (command === "PUBLISH") {
					const channel = args[1];
					const payload = args[2];
					if (channel === undefined || payload === undefined) {
						socket.destroy();
						return;
					}
					let delivered = 0;
					for (const [peer, subscriptions] of peers) {
						if (subscriptions.has(channel)) {
							peer.write(
								`>3\r\n${bulk("message")}${bulk(channel)}${bulk(payload)}`,
							);
							delivered++;
						}
					}
					socket.write(`:${delivered}\r\n`);
				} else if (command === "HVALS") socket.write("*0\r\n");
				else if (["HSET", "HDEL", "EXPIRE"].includes(command ?? ""))
					socket.write(":1\r\n");
				else socket.write("+OK\r\n");
			}
		});
	});
	await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
	const address = server.address();
	if (!address || typeof address === "string")
		throw new Error("Missing fixture address");
	return {
		port: address.port,
		stop: async () => {
			for (const socket of peers.keys()) socket.destroy();
			await new Promise<void>((resolve) => server.close(() => resolve()));
		},
	};
}

test("opaque frames fan out across replicas and clients reconnect to a surviving replica", async () => {
	const redis = await redisFixture();
	try {
		const one = await replica(redis.port);
		const two = await replica(redis.port);
		const device = await connection(one.port, "device", "device");
		const controller = await connection(two.port, "controller", "controller");
		controller.socket.send(frame("device"));
		await eventually(
			() => device.messages.find((message) => message.type === "frame"),
			"cross-replica frame",
		);
		expect(
			device.messages.filter((message) => message.type === "frame"),
		).toHaveLength(1);
		one.child.kill("SIGKILL");
		await one.child.exited;
		children.delete(one.child);
		const replacement = await connection(two.port, "device", "device");
		controller.socket.send(frame("device"));
		await eventually(
			() => replacement.messages.find((message) => message.type === "frame"),
			"surviving replica delivery",
		);
		replacement.socket.send(frame("controller"));
		await eventually(
			() => controller.messages.find((message) => message.type === "frame"),
			"surviving replica reply",
		);
		await redis.stop();
		expect(
			await eventually(
				controller.closed,
				"fanout outage closes management transport",
			),
		).toBe(1013);
		expect((await fetch(`http://127.0.0.1:${two.port}/health`)).status).toBe(
			200,
		);
		expect((await fetch(`http://127.0.0.1:${two.port}/ready`)).status).toBe(
			503,
		);
	} finally {
		await cleanup();
		await redis.stop();
	}
}, 20_000);
