import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { FrameQueue } from "./transport";
import { DeviceServiceTunnel } from "./tunnel";
import {
	TUNNEL_MAX_DATA,
	TUNNEL_WINDOW,
	type TunnelFrame,
	TunnelKind,
	decodeTunnelEnvelope,
	decodeTunnelFrame,
	encodeTunnelEnvelope,
	encodeTunnelFrame,
	tunnelJson,
} from "./tunnel-protocol";
import type { TunnelTransport } from "./tunnel-transport";
import type { NoiseHandshake, NoiseSession } from "./types";

const empty = new Uint8Array();
const now = () => Math.floor(Date.now() / 1000);
const active: DeviceServiceTunnel[] = [];
afterEach(() => {
	for (const tunnel of active.splice(0)) tunnel.close();
});
const settle = async () => {
	for (let index = 0; index < 20; index++) await Promise.resolve();
};

class Peer implements TunnelTransport {
	kind = "websocket" as const;
	expiresAt = now() + 300;
	input = new FrameQueue<Uint8Array>(1024);
	sent: (TunnelFrame & { freshKey: boolean })[] = [];
	sequence = 1n;
	closed = false;
	installed = false;
	async send(bytes: Uint8Array) {
		const frame = decodeTunnelFrame(bytes);
		expect(frame.sequence).toBe(BigInt(this.sent.length));
		this.sent.push({ ...frame, freshKey: this.installed });
		if (frame.kind === TunnelKind.Open)
			this.emit(TunnelKind.Opened, frame.stream);
	}
	next() {
		return this.input.next();
	}
	emit(
		kind: TunnelKind,
		stream = 0,
		body: Uint8Array = empty,
		sequence?: bigint,
	) {
		this.input.push(
			encodeTunnelFrame({
				kind,
				stream,
				body,
				sequence: sequence ?? this.sequence++,
			}),
		);
	}
	beginRenewal(): NoiseHandshake {
		let messages = 0;
		return {
			certificate: () => "fresh-certificate",
			sessionId: () => "new-session",
			write: () => new Uint8Array([++messages]),
			read: (bytes) => {
				expect(bytes).toEqual(new Uint8Array([2]));
			},
			finish: () => ({
				encrypt: (b) => b,
				decrypt: (b) => b,
				close() {},
				free() {},
			}),
			close() {},
			free() {},
		};
	}
	installSession(_session: NoiseSession) {
		this.installed = true;
	}
	confirmRenewal(expiry: number) {
		this.expiresAt = expiry;
	}
	close() {
		this.closed = true;
		this.input.close();
	}
	connect() {
		const tunnel = DeviceServiceTunnel.fromTransport(this);
		active.push(tunnel);
		return tunnel;
	}
}

function credit(value: number): Uint8Array {
	const bytes = new Uint8Array(4);
	new DataView(bytes.buffer).setUint32(0, value);
	return bytes;
}
function renew(tunnel: DeviceServiceTunnel) {
	(tunnel as unknown as { renew(): void }).renew();
}

describe("device tunnel wire contract", () => {
	test("matches the Rust fixture including a sequence above the JS safe integer range", () => {
		const fixture = JSON.parse(
			readFileSync(
				new URL(
					"../../../device-protocol/fixtures/tunnel-v1.json",
					import.meta.url,
				),
				"utf8",
			),
		);
		const bytes = Uint8Array.from(Buffer.from(fixture.data_frame_hex, "hex"));
		const frame = decodeTunnelFrame(bytes);
		expect(frame.kind).toBe(TunnelKind.Data);
		expect(frame.sequence).toBe(BigInt(fixture.sequence));
		expect(frame.stream).toBe(fixture.stream_id);
		expect(Buffer.from(frame.body).toString("hex")).toBe(fixture.data_hex);
		expect(encodeTunnelFrame(frame)).toEqual(bytes);
	});
	test("rejects oversized chunks, invalid credit and unknown open metadata", () => {
		expect(() =>
			encodeTunnelFrame({
				kind: TunnelKind.Data,
				stream: 1,
				sequence: 0n,
				body: new Uint8Array(TUNNEL_MAX_DATA + 1),
			}),
		).toThrow();
		expect(() =>
			encodeTunnelFrame({
				kind: TunnelKind.Window,
				stream: 1,
				sequence: 0n,
				body: credit(0),
			}),
		).toThrow();
		expect(() =>
			encodeTunnelFrame({
				kind: TunnelKind.Open,
				stream: 1,
				sequence: 0n,
				body: tunnelJson({
					placement_id: "placement",
					service_id: "hosting",
					host: "127.0.0.1",
				}),
			}),
		).toThrow();
		const envelope = encodeTunnelEnvelope(
			"message",
			"session-1",
			new Uint8Array(16),
		);
		expect(decodeTunnelEnvelope(envelope).sessionId).toBe("session-1");
		envelope[5] = 129;
		expect(() => decodeTunnelEnvelope(envelope)).toThrow();
	});
});

describe("bounded multiplexed service streams", () => {
	test("a model gateway open names its target and no placement or service", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const stream = await tunnel.openModelGateway();
		const open = peer.sent.find((frame) => frame.kind === TunnelKind.Open);
		expect(open?.stream).toBe(stream.id);
		expect(JSON.parse(new TextDecoder().decode(open?.body))).toEqual({
			target: "model_gateway",
			mode: "http",
		});
	});
	test("a blocked writer does not block another stream, and reads replenish only consumed bytes", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const first = await tunnel.open("app-a");
		const second = await tunnel.open("app-b");
		let progress = 0;
		await first.write(new Uint8Array(TUNNEL_WINDOW), () => {
			progress++;
		});
		expect(progress).toBe(Math.ceil(TUNNEL_WINDOW / TUNNEL_MAX_DATA));
		let done = false;
		const blocked = first.write(new Uint8Array([5])).then(() => {
			done = true;
		});
		await settle();
		expect(done).toBe(false);
		await second.write(new Uint8Array([7]));
		expect(peer.sent.at(-1)?.stream).toBe(second.id);
		peer.emit(TunnelKind.Window, first.id, credit(1));
		await blocked;
		peer.emit(TunnelKind.Data, second.id, new Uint8Array([9, 8]));
		await settle();
		expect(peer.sent.filter((f) => f.kind === TunnelKind.Window)).toHaveLength(
			0,
		);
		expect(await second.read()).toEqual(new Uint8Array([9, 8]));
		await settle();
		expect(peer.sent.at(-1)?.body).toEqual(credit(2));
	});
	test("caps streams and preserves half-close until buffered input is consumed", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const streams = await Promise.all(
			Array.from({ length: 16 }, (_, index) => tunnel.open(`app-${index}`)),
		);
		await expect(tunnel.open("overflow")).rejects.toThrow("no free");
		const stream = streams[0];
		peer.emit(TunnelKind.Data, stream.id, new Uint8Array([1]));
		peer.emit(TunnelKind.Fin, stream.id);
		await settle();
		await stream.write(new Uint8Array([2]));
		await stream.end();
		expect(await stream.read()).toEqual(new Uint8Array([1]));
		expect(await stream.read()).toBeNull();
		peer.emit(TunnelKind.Window, stream.id, credit(1));
		await settle();
		expect(peer.closed).toBe(false);
		await tunnel.open("replacement");
	});
	test("resets all streams on transport loss without replaying pending writes", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		await stream.write(new Uint8Array(TUNNEL_WINDOW));
		const write = stream.write(new Uint8Array([1])).catch((error) => error);
		const read = stream.read().catch((error) => error);
		peer.close();
		expect(await write).toBeInstanceOf(Error);
		expect(await read).toBeInstanceOf(Error);
		await expect(tunnel.open("app")).rejects.toThrow();
		expect(
			peer.sent
				.filter((f) => f.kind === TunnelKind.Data)
				.reduce((total, f) => total + f.body.length, 0),
		).toBe(TUNNEL_WINDOW);
	});
	test("closing rejects a write whose transport is still waiting for buffer capacity", async () => {
		const peer = new Peer();
		const original = peer.send.bind(peer);
		let release: () => void = () => {};
		peer.send = async (bytes) => {
			if (decodeTunnelFrame(bytes).kind === TunnelKind.Data)
				await new Promise<void>((resolve) => {
					release = resolve;
				});
			await original(bytes);
		};
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		const writing = stream.write(new Uint8Array([1])).catch((error) => error);
		await settle();
		tunnel.close();
		expect(await writing).toBeInstanceOf(Error);
		release();
		await settle();
	});
	test("resetting a service rejects its stalled write and leaves other streams available", async () => {
		const peer = new Peer();
		const original = peer.send.bind(peer);
		let release: () => void = () => {};
		peer.send = async (bytes) => {
			if (decodeTunnelFrame(bytes).kind === TunnelKind.Data)
				await new Promise<void>((resolve) => {
					release = resolve;
				});
			await original(bytes);
		};
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		const writing = stream.write(new Uint8Array([1])).catch((error) => error);
		await settle();
		stream.reset();
		expect(await writing).toBeInstanceOf(Error);
		expect(peer.closed).toBe(false);
		release();
		await settle();
		await tunnel.open("another-app");
	});
	test("fails closed on excess credit, sequence gaps and a peer overrunning the receive window", async () => {
		for (const violation of ["credit", "sequence", "window"] as const) {
			const peer = new Peer();
			const tunnel = peer.connect();
			const stream = await tunnel.open("app");
			if (violation === "credit")
				peer.emit(TunnelKind.Window, stream.id, credit(1));
			else if (violation === "sequence")
				peer.emit(TunnelKind.Data, stream.id, new Uint8Array([1]), 100n);
			else {
				let left = TUNNEL_WINDOW;
				while (left > 0) {
					const size = Math.min(left, TUNNEL_MAX_DATA);
					peer.emit(TunnelKind.Data, stream.id, new Uint8Array(size));
					left -= size;
				}
				peer.emit(TunnelKind.Data, stream.id, new Uint8Array([1]));
			}
			await settle();
			expect(peer.closed).toBe(true);
			await expect(stream.read()).rejects.toThrow();
		}
	});
	test("coalesces tiny unread messages into bounded chunks", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		for (let index = 0; index < 513; index++)
			peer.emit(TunnelKind.Data, stream.id, new Uint8Array([1]));
		for (let index = 0; index < 30; index++) await settle();
		expect(peer.closed).toBe(false);
		expect(await stream.read()).toEqual(new Uint8Array(513).fill(1));
	});
});

describe("tunnel authorization renewal", () => {
	test("keeps service streams and sequences while switching keys and defers a crossing heartbeat", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		renew(tunnel);
		await settle();
		expect(peer.sent.at(-1)?.kind).toBe(TunnelKind.RenewStart);
		let written = false;
		const write = stream.write(new Uint8Array([7])).then(() => {
			written = true;
		});
		peer.emit(TunnelKind.Data, stream.id, new Uint8Array([5]));
		peer.emit(TunnelKind.Ping, 0, new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]));
		peer.emit(TunnelKind.RenewReply, 0, new Uint8Array([2]));
		await settle();
		expect(written).toBe(false);
		expect(peer.installed).toBe(true);
		expect(peer.sent.at(-1)?.kind).toBe(TunnelKind.RenewFinish);
		expect(peer.sent.at(-1)?.freshKey).toBe(false);
		peer.emit(TunnelKind.Renewed, 0, tunnelJson({ expires_at: now() + 600 }));
		await write;
		expect(await stream.read()).toEqual(new Uint8Array([5]));
		await settle();
		const resumed = peer.sent.filter((f) => f.freshKey);
		expect(
			resumed.some((f) => f.kind === TunnelKind.Data && f.stream === stream.id),
		).toBe(true);
		expect(resumed.some((f) => f.kind === TunnelKind.Pong)).toBe(true);
		expect(peer.closed).toBe(false);
	});
	test("requires the new-key confirmation before accepting further service data", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		renew(tunnel);
		await settle();
		peer.emit(TunnelKind.RenewReply, 0, new Uint8Array([2]));
		await settle();
		peer.emit(TunnelKind.Data, stream.id, new Uint8Array([1]));
		await settle();
		expect(peer.closed).toBe(true);
		await expect(stream.read()).rejects.toThrow();
	});
	test("closes pending streams when renewal does not finish before its deadline", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		const stream = await tunnel.open("app");
		const original = globalThis.setTimeout;
		let deadline: (() => void) | undefined;
		globalThis.setTimeout = ((callback: () => void, delay: number) => {
			if (delay === 15_000) deadline = callback;
			return original(callback, delay);
		}) as typeof setTimeout;
		try {
			renew(tunnel);
			await settle();
			expect(deadline).toBeFunction();
			const reading = stream.read().catch((error) => error);
			deadline?.();
			expect((await reading).message).toContain("renewal timed out");
			expect(peer.closed).toBe(true);
		} finally {
			globalThis.setTimeout = original;
		}
	});
});
