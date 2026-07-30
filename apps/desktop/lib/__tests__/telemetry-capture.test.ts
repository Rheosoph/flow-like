import {
	type ITelemetryCapturedEvent,
	captureTelemetryEvent,
	setTelemetryEventSink,
} from "@flow-like/flow-like-ui/lib/telemetry/capture";
import {
	capturePageView,
	sanitizeTelemetryPath,
} from "@flow-like/flow-like-ui/lib/telemetry/page-view";
import { afterEach, describe, expect, test } from "vitest";

let removeSink: (() => void) | undefined;

function drainPending() {
	const drained: ITelemetryCapturedEvent[] = [];
	const remove = setTelemetryEventSink((event) => drained.push(event));
	remove();
	return drained;
}

afterEach(() => {
	removeSink?.();
	removeSink = undefined;
	drainPending();
});

describe("telemetry capture", () => {
	test("buffers events without a sink and flushes them on attach in order", () => {
		captureTelemetryEvent("first", { step: 1 });
		captureTelemetryEvent("second");

		const received: ITelemetryCapturedEvent[] = [];
		removeSink = setTelemetryEventSink((event) => received.push(event));

		expect(received.map((event) => event.name)).toEqual(["first", "second"]);
		expect(received[0]?.props).toEqual({ step: 1 });
		expect(received[1]?.props).toBeUndefined();
		expect(Date.parse(received[0]?.client_ts ?? "")).not.toBeNaN();
	});

	test("drops the oldest pending events beyond the buffer cap", () => {
		for (let index = 0; index < 130; index++) {
			captureTelemetryEvent(`event_${index}`);
		}

		const received: ITelemetryCapturedEvent[] = [];
		removeSink = setTelemetryEventSink((event) => received.push(event));

		expect(received).toHaveLength(128);
		expect(received[0]?.name).toBe("event_2");
		expect(received[127]?.name).toBe("event_129");
	});

	test("unregister detaches the sink and later events buffer again", () => {
		const received: ITelemetryCapturedEvent[] = [];
		const remove = setTelemetryEventSink((event) => received.push(event));
		remove();

		captureTelemetryEvent("after_unregister");
		expect(received).toHaveLength(0);

		const late: ITelemetryCapturedEvent[] = [];
		removeSink = setTelemetryEventSink((event) => late.push(event));
		expect(late.map((event) => event.name)).toEqual(["after_unregister"]);
	});

	test("stale unregister does not detach a newer sink", () => {
		const removeOld = setTelemetryEventSink(() => undefined);
		const received: ITelemetryCapturedEvent[] = [];
		removeSink = setTelemetryEventSink((event) => received.push(event));

		removeOld();
		captureTelemetryEvent("still_delivered");
		expect(received.map((event) => event.name)).toEqual(["still_delivered"]);
	});

	test("swallows sink errors", () => {
		removeSink = setTelemetryEventSink(() => {
			throw new Error("sink failure");
		});
		expect(() => captureTelemetryEvent("explodes")).not.toThrow();
	});
});

describe("sanitizeTelemetryPath", () => {
	test("replaces id-like segments", () => {
		expect(sanitizeTelemetryPath("/apps/0a1b2c3d4e5f6a7b8c/settings")).toBe(
			"/apps/:id/settings",
		);
		expect(
			sanitizeTelemetryPath("/runs/6f9619ff-8b86-4d01-b42d-00cf4fc964ff"),
		).toBe("/runs/:id");
		expect(sanitizeTelemetryPath("/store/page/42")).toBe("/store/page/:id");
	});

	test("keeps short human-readable segments", () => {
		expect(sanitizeTelemetryPath("/library/settings")).toBe(
			"/library/settings",
		);
	});

	test("strips query strings and hashes", () => {
		expect(sanitizeTelemetryPath("/store?tab=apps&id=secret")).toBe("/store");
		expect(sanitizeTelemetryPath("/store#section")).toBe("/store");
	});

	test("maps empty paths to root and caps length", () => {
		expect(sanitizeTelemetryPath("")).toBe("/");
		expect(sanitizeTelemetryPath("?only=query")).toBe("/");
		const long = "/segment".repeat(40);
		expect(sanitizeTelemetryPath(long)).toHaveLength(256);
	});

	test("capturePageView emits a page_view with the sanitized path", () => {
		const received: ITelemetryCapturedEvent[] = [];
		removeSink = setTelemetryEventSink((event) => received.push(event));

		capturePageView("/apps/0a1b2c3d4e5f6a7b8c?token=nope");

		expect(received).toHaveLength(1);
		expect(received[0]?.name).toBe("page_view");
		expect(received[0]?.props).toEqual({ path: "/apps/:id" });
	});
});
