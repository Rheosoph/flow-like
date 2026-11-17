import { describe, expect, test } from "bun:test";
import type { IBotScreenAccess } from "../../state/backend-state/bot-state";
import { openEnvelope, sealEnvelope } from "./screen-crypto";
import { discoverHosts } from "./screen-discovery";
import { ScreenSignaling, type SocketLike } from "./screen-signaling";

const KEY = {
	id: "2026-11-17",
	key: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
};
const ROOM = "0123456789abcdef0123456789abcdef";
const NOW = 1_763_337_600_000;
const ACCESS: IBotScreenAccess = {
	jwt: "a.b.c",
	room: ROOM,
	topic: `bot-screens:${ROOM}:v1`,
	keys: [KEY],
	signaling_servers: ["wss://signaling.example"],
	expires_at: NOW / 1000 + 3600,
};

class FakeSocket implements SocketLike {
	onopen: SocketLike["onopen"] = null;
	onmessage: SocketLike["onmessage"] = null;
	onclose: SocketLike["onclose"] = null;
	onerror: SocketLike["onerror"] = null;
	sent: string[] = [];
	constructor(
		readonly url: string,
		readonly protocols: string[],
	) {}
	send(data: string) {
		this.sent.push(data);
	}
	close() {}
}

function harness(viewerId = "viewer0123456789") {
	const sockets: FakeSocket[] = [];
	const signaling = new ScreenSignaling({
		access: async () => ACCESS,
		viewerId,
		now: () => NOW,
		createSocket: (url, protocols) => {
			const socket = new FakeSocket(url, protocols);
			sockets.push(socket);
			return socket;
		},
	});
	const open = async () => {
		await new Promise((resolve) => setTimeout(resolve, 5));
		sockets[0].onopen?.({});
		await signaling.ready();
		return sockets[0];
	};
	const deliver = async (socket: FakeSocket, message: object, key = KEY) => {
		const data = await sealEnvelope(key, ROOM, JSON.stringify(message));
		socket.onmessage?.({
			data: JSON.stringify({
				type: "publish",
				topic: ACCESS.topic,
				data,
				clients: 2,
			}),
		});
	};
	return { signaling, open, deliver };
}

const here = (from: string, active: boolean, bot = "bot-1") => ({
	type: "here",
	from,
	to: "viewer0123456789",
	bot_id: bot,
	active,
	at: NOW,
});

describe("ScreenSignaling", () => {
	test("connects with the realtime subprotocols, subscribes and seals publishes", async () => {
		const { signaling, open } = harness();
		const socket = await open();
		expect(socket.url).toMatch(/^wss:\/\/signaling\.example\/ws\/session\//);
		expect(socket.protocols).toEqual([
			"flowlike.realtime.v1",
			"flowlike.jwt.a.b.c",
		]);
		expect(JSON.parse(socket.sent[0])).toEqual({
			type: "subscribe",
			topics: [ACCESS.topic],
		});
		await signaling.send({
			type: "hello",
			from: signaling.viewerId,
			bot_id: "bot-1",
		});
		const frame = JSON.parse(socket.sent[1]);
		expect(frame).toMatchObject({ type: "publish", topic: ACCESS.topic });
		const opened = await openEnvelope(frame.data, ROOM, [KEY]);
		expect(opened.status === "ok" && JSON.parse(opened.text)).toEqual({
			type: "hello",
			from: "viewer0123456789",
			bot_id: "bot-1",
			at: NOW,
		});
		signaling.close();
	});

	test("delivers valid messages and drops undecryptable ones", async () => {
		const { signaling, open, deliver } = harness();
		const seen: unknown[] = [];
		signaling.subscribe((signal) => seen.push(signal));
		const socket = await open();
		await deliver(socket, here("hostpeer0123456789", true));
		await deliver(socket, here("hostpeer0123456789", true), {
			...KEY,
			key: "AgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4fICE",
		});
		await deliver(socket, { type: "here", from: "x" });
		socket.onmessage?.({ data: "garbage" });
		await new Promise((resolve) => setTimeout(resolve, 10));
		expect(seen).toHaveLength(1);
		signaling.close();
	});

	test("rejects ready when the first access fails", async () => {
		const signaling = new ScreenSignaling({
			access: async () => {
				throw new Error("no access");
			},
		});
		await expect(signaling.ready()).rejects.toThrow("no access");
	});
});

describe("discoverHosts", () => {
	test("stops at the first active host", async () => {
		const { signaling, open, deliver } = harness();
		const socket = await open();
		const found = discoverHosts(signaling, "bot-1", { windowMs: 500 });
		await new Promise((resolve) => setTimeout(resolve, 10));
		await deliver(socket, here("hostpeer0000000001", false));
		await deliver(socket, here("hostpeer0000000002", true));
		const result = await found;
		expect(result.active).toBe("hostpeer0000000002");
		expect([...result.hosts.keys()].sort()).toEqual([
			"hostpeer0000000001",
			"hostpeer0000000002",
		]);
		signaling.close();
	});

	test("answered but none active means asleep hosts, none answered means no hosts", async () => {
		const { signaling, open, deliver } = harness();
		const socket = await open();
		const asleep = discoverHosts(signaling, "bot-1", { windowMs: 60 });
		await new Promise((resolve) => setTimeout(resolve, 10));
		await deliver(socket, here("hostpeer0000000001", false));
		const first = await asleep;
		expect(first.active).toBeNull();
		expect(first.hosts.size).toBe(1);
		const nobody = await discoverHosts(signaling, "bot-1", { windowMs: 20 });
		expect(nobody.hosts.size).toBe(0);
		signaling.close();
	});

	test("ignores the excluded host, other bots and other viewers", async () => {
		const { signaling, open, deliver } = harness();
		const socket = await open();
		const found = discoverHosts(signaling, "bot-1", {
			windowMs: 60,
			excludeHost: "hostpeer0000000009",
		});
		await new Promise((resolve) => setTimeout(resolve, 10));
		await deliver(socket, here("hostpeer0000000009", true));
		await deliver(socket, here("hostpeer0000000001", true, "bot-2"));
		await deliver(socket, {
			...here("hostpeer0000000001", true),
			to: "someoneelse1234",
		});
		const result = await found;
		expect(result.hosts.size).toBe(0);
		signaling.close();
	});
});
