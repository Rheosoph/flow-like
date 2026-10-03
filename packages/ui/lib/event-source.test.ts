import { describe, expect, test } from "vitest";
import {
	eventTriggerConfig,
	isDeviceEventSource,
	mergeEventTriggerConfig,
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
});
