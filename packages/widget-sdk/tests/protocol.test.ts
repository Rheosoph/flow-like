import { describe, expect, test } from "bun:test";
import { FLW_PROTOCOL, createEnvelope, isFlwEnvelope } from "../src/protocol";

describe("createEnvelope", () => {
	test("produces the frozen flw/1 shape", () => {
		const envelope = createEnvelope(
			"ready",
			{ contractVersion: 1 },
			"nonce-1",
			"instance-1",
		);
		expect(envelope).toEqual({
			protocol: "flw/1",
			nonce: "nonce-1",
			instanceId: "instance-1",
			type: "ready",
			payload: { contractVersion: 1 },
		});
	});

	test("hello uses empty nonce and instanceId", () => {
		const envelope = createEnvelope("hello", {}, "", "");
		expect(envelope.protocol).toBe(FLW_PROTOCOL);
		expect(envelope.nonce).toBe("");
		expect(envelope.instanceId).toBe("");
		expect(envelope.payload).toEqual({});
	});
});

describe("isFlwEnvelope", () => {
	const valid = createEnvelope("resize", { height: 100 }, "n", "i");

	test("accepts a valid envelope", () => {
		expect(isFlwEnvelope(valid)).toBe(true);
	});

	test("accepts every known message type", () => {
		const types = [
			"init",
			"props:update",
			"theme:change",
			"query",
			"hello",
			"ready",
			"event",
			"query:result",
			"resize",
			"value:changed",
		];
		for (const type of types) {
			expect(isFlwEnvelope({ ...valid, type })).toBe(true);
		}
	});

	test("rejects non-objects", () => {
		expect(isFlwEnvelope(null)).toBe(false);
		expect(isFlwEnvelope(undefined)).toBe(false);
		expect(isFlwEnvelope("flw/1")).toBe(false);
		expect(isFlwEnvelope(42)).toBe(false);
	});

	test("rejects a wrong protocol", () => {
		expect(isFlwEnvelope({ ...valid, protocol: "flw/2" })).toBe(false);
		expect(isFlwEnvelope({ ...valid, protocol: undefined })).toBe(false);
	});

	test("rejects missing or non-string nonce / instanceId", () => {
		expect(isFlwEnvelope({ ...valid, nonce: undefined })).toBe(false);
		expect(isFlwEnvelope({ ...valid, nonce: 5 })).toBe(false);
		expect(isFlwEnvelope({ ...valid, instanceId: undefined })).toBe(false);
		expect(isFlwEnvelope({ ...valid, instanceId: {} })).toBe(false);
	});

	test("rejects unknown message types", () => {
		expect(isFlwEnvelope({ ...valid, type: "self-destruct" })).toBe(false);
		expect(isFlwEnvelope({ ...valid, type: 3 })).toBe(false);
	});

	test("rejects an envelope without a payload field", () => {
		const { payload: _payload, ...withoutPayload } = valid;
		expect(isFlwEnvelope(withoutPayload)).toBe(false);
	});

	test("accepts a null payload", () => {
		expect(isFlwEnvelope({ ...valid, payload: null })).toBe(true);
	});
});
