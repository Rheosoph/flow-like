import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";
import {
	MAX_DOCX_BYTES,
	WORDCRAFT_CHANNEL,
	readWordcraftMessage,
} from "./wordcraft-protocol";

const frame = {} as Window;
const origin = "https://app.example.test";
const session = "document-session";

function read(
	data: Record<string, unknown>,
	overrides: Partial<MessageEvent> = {},
) {
	return readWordcraftMessage(
		{
			source: frame,
			origin,
			data: { channel: WORDCRAFT_CHANNEL, session, ...data },
			...overrides,
		} as MessageEvent,
		frame,
		origin,
		session,
	);
}

describe("WordCraft message boundary", () => {
	it.each([
		{ source: {} as Window },
		{ source: null },
		{ origin: "https://other.example.test" },
		{ origin: "null" },
		{ data: { channel: WORDCRAFT_CHANNEL, session: "old", type: "ready" } },
		{ data: { channel: "other-channel", session, type: "ready" } },
		{ data: null },
		{ data: "ready" },
	])("rejects a message outside the active frame session: %j", (overrides) => {
		expect(read({ type: "ready" }, overrides)).toBeNull();
	});

	it("rejects a message when there is no mounted frame", () => {
		expect(
			readWordcraftMessage(
				{ source: frame, origin, data: {} } as MessageEvent,
				null,
				origin,
				session,
			),
		).toBeNull();
	});

	it.each(["ready", "opened"])("accepts the %s lifecycle message", (type) => {
		expect(read({ type, ignored: true })).toEqual({ type });
	});

	it("requires a boolean dirty state", () => {
		expect(read({ type: "dirty", dirty: true })).toEqual({
			type: "dirty",
			dirty: true,
		});
		expect(read({ type: "dirty", dirty: "false" })).toBeNull();
	});

	it("accepts bounded loading progress with an interactive editor", () => {
		for (const [completed, total] of [
			[0, 0],
			[2, 10],
			[10, 10],
		]) {
			const message = { type: "loading", interactive: true, completed, total };
			expect(read(message)).toEqual(message);
		}
	});

	it.each([
		{ interactive: "true" },
		{ completed: -1 },
		{ completed: 11 },
		{ completed: 0.5 },
		{ total: Number.POSITIVE_INFINITY },
		{ total: Number.MAX_SAFE_INTEGER + 1 },
		{ total: -1 },
	])("rejects malformed loading progress: %j", (overrides) => {
		expect(
			read({
				type: "loading",
				interactive: true,
				completed: 0,
				total: 10,
				...overrides,
			}),
		).toBeNull();
	});

	it("keeps the binary export and revision together", () => {
		const bytes = new Uint8Array([80, 75, 3, 4]).buffer;
		expect(
			read({ type: "exported", requestId: "save-1", bytes, revision: 8 }),
		).toEqual({ type: "exported", requestId: "save-1", bytes, revision: 8 });
	});
	it("accepts a real export buffer from another window realm", () => {
		const bytes = runInNewContext("new Uint8Array([80, 75, 3, 4]).buffer");
		expect(bytes).not.toBeInstanceOf(ArrayBuffer);
		expect(
			read({ type: "exported", requestId: "save-portal", bytes, revision: 8 }),
		).toEqual({
			type: "exported",
			requestId: "save-portal",
			bytes,
			revision: 8,
		});
	});

	it("accepts measured export progress and stages without a percentage", () => {
		for (const progress of [0, 42.5, 100, null]) {
			const message = {
				type: "export-progress",
				requestId: "save-1",
				stage: "rendering",
				progress,
			};
			expect(read(message)).toEqual(message);
		}
	});

	it.each([
		{ requestId: 12 },
		{ stage: "unknown" },
		{ progress: -1 },
		{ progress: 101 },
		{ progress: Number.NaN },
		{ progress: Number.POSITIVE_INFINITY },
		{ progress: "50" },
		{ progress: undefined },
	])("rejects invalid export progress: %j", (overrides) => {
		expect(
			read({
				type: "export-progress",
				requestId: "save-1",
				stage: "rendering",
				progress: 50,
				...overrides,
			}),
		).toBeNull();
	});

	it.each([
		{ requestId: 12 },
		{ bytes: new Uint8Array([80, 75]) },
		{ bytes: { byteLength: 4, [Symbol.toStringTag]: "ArrayBuffer" } },
		{ bytes: new SharedArrayBuffer(4) },
		{ bytes: new ArrayBuffer(0) },
		{ bytes: new ArrayBuffer(MAX_DOCX_BYTES + 1) },
		{ revision: -1 },
		{ revision: 0.5 },
		{ revision: Number.MAX_SAFE_INTEGER + 1 },
		{ revision: Number.NaN },
	])("rejects malformed or oversized exports: %j", (overrides) => {
		expect(
			read({
				type: "exported",
				requestId: "save-1",
				bytes: new ArrayBuffer(4),
				revision: 1,
				...overrides,
			}),
		).toBeNull();
	});

	it("bounds error text and ignores an invalid request identifier", () => {
		expect(
			read({ type: "error", error: "x".repeat(2000), requestId: 42 }),
		).toEqual({
			type: "error",
			error: "x".repeat(1000),
			requestId: undefined,
		});
		expect(read({ type: "error", error: {} })).toBeNull();
	});

	it("rejects unknown message types", () => {
		expect(read({ type: "save", bytes: new ArrayBuffer(4) })).toBeNull();
	});
});
