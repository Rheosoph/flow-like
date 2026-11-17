import { describe, expect, test } from "bun:test";
import {
	PlaygroundDecoder,
	type PlaygroundEvent,
	encodeControl,
} from "./playground-codec";

const text = (value: string) => new TextEncoder().encode(value);

function message(kind: number, payload: Uint8Array): Uint8Array {
	const bytes = new Uint8Array(5 + payload.length);
	bytes[0] = kind;
	new DataView(bytes.buffer).setUint32(1, payload.length);
	bytes.set(payload, 5);
	return bytes;
}

const STATE = '{"taken_over":false,"asleep":true,"display":"none"}';
const META =
	'{"width":1280,"height":800,"kind":"computer","caption":"Desktop","age_ms":12}';

function frameBytes(jpeg: number[]): Uint8Array {
	const meta = text(META);
	const payload = new Uint8Array(2 + meta.length + jpeg.length);
	payload[0] = meta.length >> 8;
	payload[1] = meta.length & 0xff;
	payload.set(meta, 2);
	payload.set(jpeg, 2 + meta.length);
	return message(2, payload);
}

function drain(decoder: PlaygroundDecoder): PlaygroundEvent[] {
	const events: PlaygroundEvent[] = [];
	for (let event = decoder.next(); event; event = decoder.next())
		events.push(event);
	return events;
}

describe("playground codec", () => {
	test("encodes controls with the wire header", () => {
		expect(Array.from(encodeControl({ type: "take_over" }))).toEqual([
			16,
			0,
			0,
			0,
			20,
			...text('{"type":"take_over"}'),
		]);
		expect(
			new TextDecoder().decode(encodeControl({ type: "stop" }).subarray(5)),
		).toBe('{"type":"stop"}');
	});

	test("encodes input with only known fields and whole scroll deltas", () => {
		const body = (bytes: Uint8Array) =>
			JSON.parse(new TextDecoder().decode(bytes.subarray(5)));
		expect(
			body(
				encodeControl({
					type: "input",
					event: { type: "scroll", x: 1.5, y: 2, dx: 0.4, dy: 99.6 },
				}),
			),
		).toEqual({
			type: "input",
			event: { type: "scroll", x: 1.5, y: 2, dx: 0, dy: 100 },
		});
	});

	test("rejects input outside the caps", () => {
		const input = (event: object) =>
			encodeControl({ type: "input", event } as never);
		expect(() =>
			input({ type: "pointer_move", x: Number.NaN, y: 0 }),
		).toThrow();
		expect(() => input({ type: "key", chord: "" })).toThrow();
		expect(() => input({ type: "key", chord: "a".repeat(65) })).toThrow();
		expect(() => input({ type: "text", text: "a".repeat(1025) })).toThrow();
		expect(() => input({ type: "text", text: "é".repeat(600) })).toThrow();
		expect(() => input({ type: "text", text: "a".repeat(1024) })).not.toThrow();
	});

	test("decodes a state message", () => {
		const decoder = new PlaygroundDecoder();
		decoder.push(message(1, text(STATE)));
		expect(drain(decoder)).toEqual([
			{
				type: "state",
				state: { taken_over: false, asleep: true, display: "none" },
			},
		]);
	});

	test("reassembles a frame split at every byte boundary", () => {
		const bytes = frameBytes([0xff, 0xd8]);
		for (let split = 1; split < bytes.length; split++) {
			const decoder = new PlaygroundDecoder();
			decoder.push(bytes.subarray(0, split));
			expect(decoder.next()).toBeNull();
			decoder.push(bytes.subarray(split));
			const [event] = drain(decoder);
			expect(event).toMatchObject({
				type: "frame",
				meta: { width: 1280, height: 800, kind: "computer", age_ms: 12 },
			});
			expect(Array.from((event as { jpeg: Uint8Array }).jpeg)).toEqual([
				0xff, 0xd8,
			]);
		}
	});

	test("decodes several messages from one chunk, errors included", () => {
		const decoder = new PlaygroundDecoder();
		const parts = [
			message(1, text(STATE)),
			message(3, text('{"code":"busy","message":"later"}')),
			frameBytes([1]),
		];
		const all = new Uint8Array(
			parts.reduce((sum, part) => sum + part.length, 0),
		);
		let offset = 0;
		for (const part of parts) {
			all.set(part, offset);
			offset += part.length;
		}
		decoder.push(all);
		expect(drain(decoder).map((event) => event.type)).toEqual([
			"state",
			"error",
			"frame",
		]);
	});

	test("rejects unknown kinds, oversize lengths and bad payloads", () => {
		const unknown = new PlaygroundDecoder();
		unknown.push(Uint8Array.of(9, 0, 0, 0, 0));
		expect(() => unknown.next()).toThrow();

		const control = new PlaygroundDecoder();
		control.push(encodeControl({ type: "stop" }));
		expect(() => control.next()).toThrow();

		const big = new PlaygroundDecoder();
		big.push(Uint8Array.of(1, 0, 0, 0x40, 0x01));
		expect(() => big.next()).toThrow();

		const bigFrame = new PlaygroundDecoder();
		bigFrame.push(Uint8Array.of(2, 0, 0x20, 0x40, 0x03));
		expect(() => bigFrame.next()).toThrow();

		const badState = new PlaygroundDecoder();
		badState.push(message(1, text('{"taken_over":1}')));
		expect(() => badState.next()).toThrow();

		const noJpeg = new PlaygroundDecoder();
		noJpeg.push(frameBytes([]));
		expect(() => noJpeg.next()).toThrow();
	});
});
