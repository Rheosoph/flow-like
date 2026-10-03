import { beforeEach, describe, expect, test } from "bun:test";
import {
	DEVICE_EVENT_CREATION_UNSUPPORTED,
	ensureDeviceEventCreation,
	resetDeviceEventCreationCache,
} from "./event-source-capability";

function harness() {
	let time = 1_000;
	let calls = 0;
	const clock = () => time;
	return {
		clock,
		advance: (ms: number) => {
			time += ms;
		},
		calls: () => calls,
		probe:
			(answer: () => Promise<{ device_event_creation?: boolean }>) => () => {
				calls += 1;
				return answer();
			},
	};
}

const supported = () => Promise.resolve({ device_event_creation: true });

describe("device event creation probe cache", () => {
	beforeEach(resetDeviceEventCreationCache);

	test("shares one probe between concurrent callers", async () => {
		const h = harness();
		await Promise.all([
			ensureDeviceEventCreation("a", h.probe(supported), h.clock),
			ensureDeviceEventCreation("a", h.probe(supported), h.clock),
		]);
		expect(h.calls()).toBe(1);
	});

	test("reuses a confirmed answer for a minute, then probes again", async () => {
		const h = harness();
		await ensureDeviceEventCreation("a", h.probe(supported), h.clock);
		h.advance(59_000);
		await ensureDeviceEventCreation("a", h.probe(supported), h.clock);
		expect(h.calls()).toBe(1);
		h.advance(2_000);
		await ensureDeviceEventCreation("a", h.probe(supported), h.clock);
		expect(h.calls()).toBe(2);
	});

	test("keeps apps apart", async () => {
		const h = harness();
		await ensureDeviceEventCreation("a", h.probe(supported), h.clock);
		await ensureDeviceEventCreation("b", h.probe(supported), h.clock);
		expect(h.calls()).toBe(2);
	});

	test("a hub without the flag is refused, and re-asked soon after an upgrade", async () => {
		const h = harness();
		await expect(
			ensureDeviceEventCreation(
				"a",
				h.probe(() => Promise.resolve({})),
				h.clock,
			),
		).rejects.toThrow(DEVICE_EVENT_CREATION_UNSUPPORTED);
		h.advance(5_000);
		await expect(
			ensureDeviceEventCreation("a", h.probe(supported), h.clock),
		).rejects.toThrow(DEVICE_EVENT_CREATION_UNSUPPORTED);
		expect(h.calls()).toBe(1);
		h.advance(6_000);
		await ensureDeviceEventCreation("a", h.probe(supported), h.clock);
		expect(h.calls()).toBe(2);
	});

	test("a missing response body counts as unsupported", async () => {
		const h = harness();
		await expect(
			ensureDeviceEventCreation(
				"a",
				h.probe(() => Promise.resolve(undefined as never)),
				h.clock,
			),
		).rejects.toThrow(DEVICE_EVENT_CREATION_UNSUPPORTED);
	});

	test.each([
		[403, "cannot manage devices"],
		[503, "does not manage devices"],
	])("explains a %i answer", async (status, text) => {
		const h = harness();
		await expect(
			ensureDeviceEventCreation(
				"a",
				h.probe(() =>
					Promise.reject(Object.assign(new Error("x"), { status })),
				),
				h.clock,
			),
		).rejects.toThrow(text);
	});

	test("passes other failures through and retries after a few seconds", async () => {
		const h = harness();
		const failure = new Error("network down");
		await expect(
			ensureDeviceEventCreation(
				"a",
				h.probe(() => Promise.reject(failure)),
				h.clock,
			),
		).rejects.toBe(failure);
		h.advance(11_000);
		await ensureDeviceEventCreation("a", h.probe(supported), h.clock);
		expect(h.calls()).toBe(2);
	});
});
