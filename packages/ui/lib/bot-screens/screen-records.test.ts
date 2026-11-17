import { describe, expect, test } from "bun:test";
import {
	MAX_RECORD_PLAINTEXT,
	RecordCipher,
	RecordStream,
	deriveStreamKeys,
} from "./screen-records";
import type { ScreenStream } from "./screen-stream";

const hex = (bytes: Uint8Array) =>
	Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
const bytes = (value: string) =>
	Uint8Array.from(value.match(/../g) ?? [], (pair) =>
		Number.parseInt(pair, 16),
	);
const SESSION = "session0123456789";
const keys = () =>
	deriveStreamKeys(
		new Uint8Array(32).fill(0x11),
		new Uint8Array(32).fill(0x22),
		SESSION,
	);
const raw = async (key: CryptoKey) =>
	hex(new Uint8Array(await crypto.subtle.exportKey("raw", key)));

class Pipe implements ScreenStream {
	written: Uint8Array[] = [];
	inbox: Uint8Array[] = [];
	resets = 0;
	async read() {
		return this.inbox.shift() ?? null;
	}
	async write(chunk: Uint8Array) {
		this.written.push(chunk);
	}
	reset() {
		this.resets += 1;
	}
}

describe("record layer", () => {
	test("derives the planned direction keys", async () => {
		const derived = await keys();
		expect(await raw(derived.viewerToHost)).toBe(
			"d6d73f83549a18a32cc11027fa390b1e6e717684c11181e523a918d55b56f6c0",
		);
		expect(await raw(derived.hostToViewer)).toBe(
			"cf7d536a0d49bd7f289ceffa6d820f3956a6b25079cefdbd88c9e0cda6dacc22",
		);
	});

	test("seals the planned records", async () => {
		const derived = await keys();
		const viewer = new RecordCipher(derived.viewerToHost);
		expect(hex(await viewer.seal(new Uint8Array(0)))).toBe(
			"582154017df1dbe93e30aecc32f9812a",
		);
		expect(
			hex(
				await viewer.seal(
					bytes("10000000147b2274797065223a2274616b655f6f766572227d"),
				),
			),
		).toBe(
			"7d499b12975d3bb1e70fcbc3d8396c85e087c6a208869c76ef636fd9067aa5001ccf672c80712f7e73",
		);
		const host = new RecordCipher(derived.hostToViewer);
		expect(hex(await host.seal(bytes("010203")))).toBe(
			"eb42a74f10aabc216519434cc7eeab8fda527a",
		);
	});

	test("rejects oversize plaintexts and a gap", async () => {
		const derived = await keys();
		const sender = new RecordCipher(derived.hostToViewer);
		await expect(
			sender.seal(new Uint8Array(MAX_RECORD_PLAINTEXT + 1)),
		).rejects.toThrow();
		await sender.seal(new Uint8Array([1]));
		const second = await sender.seal(new Uint8Array([2]));
		await expect(
			new RecordCipher(derived.hostToViewer).open(second),
		).rejects.toThrow();
	});
});

describe("RecordStream", () => {
	test("confirms the key first, then encrypts writes in order", async () => {
		const derived = await keys();
		const pipe = new Pipe();
		const stream = new RecordStream(pipe, derived);
		const payload = new Uint8Array(MAX_RECORD_PLAINTEXT + 10).fill(7);
		await stream.write(payload);
		expect(pipe.written.length).toBe(3);
		const host = new RecordCipher(derived.viewerToHost);
		expect((await host.open(pipe.written[0])).length).toBe(0);
		expect((await host.open(pipe.written[1])).length).toBe(
			MAX_RECORD_PLAINTEXT,
		);
		expect((await host.open(pipe.written[2])).length).toBe(10);
		expect(pipe.written.every((record) => record.length <= 16 * 1024)).toBe(
			true,
		);
	});

	test("reads plaintext, skips empty records, closes on a bad record", async () => {
		const derived = await keys();
		const pipe = new Pipe();
		const stream = new RecordStream(pipe, derived);
		const host = new RecordCipher(derived.hostToViewer);
		pipe.inbox.push(await host.seal(new Uint8Array(0)));
		pipe.inbox.push(await host.seal(new Uint8Array([1, 2, 3])));
		expect(Array.from((await stream.read()) ?? [])).toEqual([1, 2, 3]);
		const bad = await host.seal(new Uint8Array([4]));
		bad[0] ^= 1;
		pipe.inbox.push(bad);
		expect(await stream.read()).toBeNull();
		expect(pipe.resets).toBe(1);
	});

	test("a record out of order closes the stream", async () => {
		const derived = await keys();
		const pipe = new Pipe();
		const stream = new RecordStream(pipe, derived);
		const host = new RecordCipher(derived.hostToViewer);
		await host.seal(new Uint8Array([1]));
		pipe.inbox.push(await host.seal(new Uint8Array([2])));
		expect(await stream.read()).toBeNull();
		expect(pipe.resets).toBe(1);
	});
});
