import { describe, expect, test } from "bun:test";
import {
	eventTriggerConfig,
	isDeviceEventSource,
	mergeEventTriggerConfig,
	resolveEventSourceIntent,
	withDefaultEventSource,
	withDeviceEventSource,
} from "./event-source";

const bytes = (value: unknown) =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));
const read = (value: number[]) =>
	JSON.parse(new TextDecoder().decode(new Uint8Array(value)));

describe("device event source metadata", () => {
	test("retains trigger settings and activation when marking a definition for devices", () => {
		const event = {
			active: true,
			config: bytes({ expression: "0 9 * * *", timezone: "UTC" }),
		};
		const marked = withDeviceEventSource(event);
		expect(marked.active).toBe(true);
		expect(isDeviceEventSource(marked)).toBe(true);
		expect(read(eventTriggerConfig(marked.config))).toEqual(read(event.config));
		expect(isDeviceEventSource(event)).toBe(false);
	});

	test("preserves the source when a trigger editor replaces its full configuration", () => {
		const original = withDeviceEventSource({
			config: bytes({ expression: "0 9 * * *" }),
		});
		const next = mergeEventTriggerConfig(
			original.config,
			bytes({ expression: "0 10 * * *" }),
		);
		expect(isDeviceEventSource({ config: next })).toBe(true);
		expect(read(eventTriggerConfig(next))).toEqual({
			expression: "0 10 * * *",
		});
	});

	test("rejects malformed settings instead of losing them when creating a device event", () => {
		expect(() => withDeviceEventSource({ config: [255] })).toThrow(
			"JSON object",
		);
		expect(() => withDeviceEventSource({ config: bytes(["invalid"]) })).toThrow(
			"JSON object",
		);
	});

	test("clears the marker explicitly without losing trigger settings", () => {
		const device = withDeviceEventSource({
			config: bytes({ expression: "0 9 * * *" }),
		});
		const cleared = withDefaultEventSource(device);
		expect(isDeviceEventSource(cleared)).toBe(false);
		expect(read(cleared.config)).toMatchObject({
			__flow_like_source: "default",
			expression: "0 9 * * *",
		});
		expect(read(eventTriggerConfig(cleared.config))).toEqual({
			expression: "0 9 * * *",
		});
		expect(() => withDefaultEventSource({ config: bytes([1]) })).toThrow(
			"JSON object",
		);
	});

	test("an explicit source wins over the marker already in the config", () => {
		const device = withDeviceEventSource({ config: bytes({ a: 1 }) });
		expect(resolveEventSourceIntent(device, "default").intent).toBe("clear");
		expect(
			isDeviceEventSource(resolveEventSourceIntent(device, "default").event),
		).toBe(false);
		const plain = { config: bytes({ a: 1 }) };
		expect(resolveEventSourceIntent(plain, "device").intent).toBe("device");
		expect(
			isDeviceEventSource(resolveEventSourceIntent(plain, "device").event),
		).toBe(true);
	});

	test("without an explicit source the config marker decides, otherwise the event is untouched", () => {
		const device = withDeviceEventSource({ config: bytes({}) });
		expect(resolveEventSourceIntent(device).intent).toBe("device");
		const plain = { config: bytes({ a: 1 }) };
		const resolved = resolveEventSourceIntent(plain);
		expect(resolved.intent).toBe("keep");
		expect(resolved.event).toBe(plain);
	});

	test("treats whitespace-only settings as empty", () => {
		const blank = Array.from(new TextEncoder().encode("  \n"));
		expect(isDeviceEventSource({ config: blank })).toBe(false);
		expect(isDeviceEventSource(withDeviceEventSource({ config: blank }))).toBe(
			true,
		);
	});

	test("leaves non-object settings untouched for trigger editors", () => {
		const list = bytes(["invalid"]);
		expect(eventTriggerConfig(list)).toBe(list);
		expect(isDeviceEventSource({ config: list })).toBe(false);
	});

	test("a non-device event drops a stray marker when an editor replaces its settings", () => {
		const next = mergeEventTriggerConfig(
			bytes({ a: 1 }),
			bytes({ a: 2, __flow_like_source: "device" }),
		);
		expect(isDeviceEventSource({ config: next })).toBe(false);
		expect(read(next)).toEqual({ a: 2 });
	});

	test("merging into a device event rejects settings that are not an object", () => {
		const device = withDeviceEventSource({ config: bytes({}) });
		expect(() => mergeEventTriggerConfig(device.config, bytes([1]))).toThrow(
			"JSON object",
		);
	});
});
