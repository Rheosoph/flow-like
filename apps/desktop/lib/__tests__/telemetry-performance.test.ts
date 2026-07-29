import {
	type ITelemetryClient,
	createTelemetryClient,
} from "@flow-like/flow-like-ui/lib/telemetry/client";
import {
	type ITelemetryCapturedPerfMetric,
	capturePerfMetric,
	foldInteractionLatency,
	foldLayoutShiftValue,
	initWebVitals,
	pickLargestContentfulPaint,
	readFirstContentfulPaint,
	readTimeToFirstByte,
	setTelemetryPerfSink,
} from "@flow-like/flow-like-ui/lib/telemetry/performance";
import type { IApiState } from "@flow-like/flow-like-ui/state/backend-state/api-state";
import type { IProfile } from "@flow-like/flow-like-ui/types";
import { afterEach, describe, expect, test, vi } from "vitest";

type EntryList = { getEntries: () => unknown[] };
type ObserverCallback = (list: EntryList) => void;

function createObserverStub(config: {
	supported?: string[];
	failing?: string[];
}) {
	const emitters = new Map<string, ObserverCallback>();
	class ObserverStub {
		static supportedEntryTypes = config.supported;
		readonly callback: ObserverCallback;
		type: string | undefined;
		constructor(callback: ObserverCallback) {
			this.callback = callback;
		}
		observe(init: { type: string }) {
			if (config.failing?.includes(init.type))
				throw new TypeError(`unsupported entry type: ${init.type}`);
			this.type = init.type;
			emitters.set(init.type, this.callback);
		}
		disconnect() {
			if (this.type) emitters.delete(this.type);
		}
	}
	vi.stubGlobal("PerformanceObserver", ObserverStub);
	return {
		emit: (type: string, entries: unknown[]) =>
			emitters.get(type)?.({ getEntries: () => entries }),
		observedTypes: () => [...emitters.keys()],
	};
}

function stubDom(pathname = "/boards/abcdef0123456789") {
	const windowListeners = new Map<string, Set<() => void>>();
	const documentListeners = new Map<string, Set<() => void>>();
	const visibility = { value: "visible" };
	const add =
		(listeners: Map<string, Set<() => void>>) =>
		(name: string, handler: () => void) => {
			const existing = listeners.get(name) ?? new Set<() => void>();
			existing.add(handler);
			listeners.set(name, existing);
		};
	const remove =
		(listeners: Map<string, Set<() => void>>) =>
		(name: string, handler: () => void) => {
			listeners.get(name)?.delete(handler);
		};

	vi.stubGlobal("window", {
		location: { pathname },
		addEventListener: add(windowListeners),
		removeEventListener: remove(windowListeners),
	});
	vi.stubGlobal("document", {
		get visibilityState() {
			return visibility.value;
		},
		addEventListener: add(documentListeners),
		removeEventListener: remove(documentListeners),
	});

	const fire = (listeners: Map<string, Set<() => void>>, name: string) => {
		for (const handler of [...(listeners.get(name) ?? [])]) handler();
	};
	return {
		fireWindow: (name: string) => fire(windowListeners, name),
		fireDocument: (name: string) => fire(documentListeners, name),
		hide: () => {
			visibility.value = "hidden";
		},
		windowListeners: (name: string) => windowListeners.get(name)?.size ?? 0,
	};
}

function stubNavigationTiming(navigation: Record<string, number> | undefined) {
	vi.spyOn(performance, "getEntriesByType").mockImplementation((type) =>
		type === "navigation" && navigation
			? ([navigation] as unknown as PerformanceEntry[])
			: [],
	);
}

let removeSink: (() => void) | undefined;
let disposeVitals: (() => void) | undefined;

function collectMetrics() {
	const received: ITelemetryCapturedPerfMetric[] = [];
	removeSink = setTelemetryPerfSink((metric) => received.push(metric));
	return received;
}

function drainPendingMetrics() {
	const remove = setTelemetryPerfSink(() => undefined);
	remove();
}

function metricValue(
	received: ITelemetryCapturedPerfMetric[],
	metric: string,
): number | undefined {
	return received.find((entry) => entry.metric === metric)?.value;
}

afterEach(() => {
	disposeVitals?.();
	disposeVitals = undefined;
	removeSink?.();
	removeSink = undefined;
	drainPendingMetrics();
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe("capturePerfMetric", () => {
	test("captures a rounded metric with the sanitized current path", () => {
		stubDom();
		const received = collectMetrics();

		capturePerfMetric("app_start", 1234.6);

		expect(received).toHaveLength(1);
		expect(received[0]?.metric).toBe("app_start");
		expect(received[0]?.value).toBe(1235);
		expect(received[0]?.path).toBe("/boards/:id");
		expect(Date.parse(received[0]?.client_ts ?? "")).not.toBeNaN();
	});

	test("an explicit path wins and never carries a query or hash", () => {
		stubDom();
		const received = collectMetrics();

		capturePerfMetric("screen_load", 42, "/apps/1234/settings?tab=x#frag");

		expect(received[0]?.path).toBe("/apps/:id/settings");
		expect(received[0]?.path).not.toContain("?");
		expect(received[0]?.path).not.toContain("#");
	});

	test("omits the path when there is no DOM", () => {
		const received = collectMetrics();

		capturePerfMetric("app_start", 100);

		expect(received[0]?.path).toBeUndefined();
	});

	test("keeps cls unitless at four decimals", () => {
		const received = collectMetrics();

		capturePerfMetric("cls", 0.123456);

		expect(received[0]?.value).toBe(0.1235);
	});

	test("drops unknown metrics and impossible values", () => {
		const received = collectMetrics();

		capturePerfMetric(
			"bogus" as unknown as ITelemetryCapturedPerfMetric["metric"],
			10,
		);
		capturePerfMetric("lcp", Number.NaN);
		capturePerfMetric("lcp", Number.POSITIVE_INFINITY);
		capturePerfMetric("lcp", -1);
		capturePerfMetric("lcp", 3_600_001);
		capturePerfMetric("cls", 1_001);

		expect(received).toHaveLength(0);
	});

	test("buffers metrics without a sink and replays them on attach", () => {
		capturePerfMetric("ttfb", 120);

		const received = collectMetrics();
		expect(received).toHaveLength(1);
		expect(received[0]?.metric).toBe("ttfb");
	});

	test("never throws when the sink throws", () => {
		removeSink = setTelemetryPerfSink(() => {
			throw new Error("sink exploded");
		});

		expect(() => capturePerfMetric("fcp", 900)).not.toThrow();
	});
});

describe("web vitals folding", () => {
	test("cls accumulates a session window and reports the worst one", () => {
		const value = foldLayoutShiftValue([
			{ value: 0.05, startTime: 0 },
			{ value: 0.04, startTime: 500 },
			{ value: 0.02, startTime: 900 },
			{ value: 0.03, startTime: 4_000 },
		]);

		expect(value).toBeCloseTo(0.11, 5);
	});

	test("cls starts a new session after a 1s gap or a 5s window", () => {
		expect(
			foldLayoutShiftValue([
				{ value: 0.2, startTime: 0 },
				{ value: 0.3, startTime: 2_000 },
			]),
		).toBeCloseTo(0.3, 5);
		expect(
			foldLayoutShiftValue([
				{ value: 0.1, startTime: 0 },
				{ value: 0.1, startTime: 900 },
				{ value: 0.1, startTime: 1_800 },
				{ value: 0.1, startTime: 2_700 },
				{ value: 0.1, startTime: 3_600 },
				{ value: 0.1, startTime: 4_500 },
				{ value: 0.9, startTime: 5_100 },
			]),
		).toBeCloseTo(0.9, 5);
	});

	test("cls ignores shifts that follow recent input and junk values", () => {
		expect(
			foldLayoutShiftValue([
				{ value: 0.5, startTime: 0, hadRecentInput: true },
				{ value: Number.NaN, startTime: 10 },
				{ value: 0, startTime: 20 },
				{ value: 0.1, startTime: 30 },
			]),
		).toBeCloseTo(0.1, 5);
	});

	test("inp takes the worst qualifying interaction", () => {
		const value = foldInteractionLatency([
			{ duration: 64, interactionId: 1 },
			{ duration: 210, interactionId: 2 },
			{ duration: 900, interactionId: 0 },
			{ duration: 500 },
			{ duration: 12, interactionId: 3 },
			{ duration: 80, entryType: "first-input" },
		]);

		expect(value).toBe(210);
	});

	test("inp folds incrementally across observer callbacks", () => {
		const first = foldInteractionLatency([{ duration: 90, interactionId: 1 }]);
		expect(
			foldInteractionLatency([{ duration: 50, interactionId: 2 }], first),
		).toBe(90);
	});

	test("lcp takes the latest candidate and prefers renderTime", () => {
		expect(
			pickLargestContentfulPaint([
				{ startTime: 800, renderTime: 800 },
				{ startTime: 2_400, renderTime: 2_450, loadTime: 2_300 },
			]),
		).toBe(2_450);
		expect(
			pickLargestContentfulPaint([{ startTime: 900, loadTime: 950 }]),
		).toBe(950);
		expect(pickLargestContentfulPaint([])).toBeUndefined();
	});

	test("ttfb subtracts the prerender activation and rejects empty timing", () => {
		expect(readTimeToFirstByte({ responseStart: 800 })).toBe(800);
		expect(
			readTimeToFirstByte({ responseStart: 800, activationStart: 300 }),
		).toBe(500);
		expect(readTimeToFirstByte({ responseStart: 0 })).toBeUndefined();
		expect(readTimeToFirstByte(undefined)).toBeUndefined();
	});

	test("fcp picks the first-contentful-paint entry only", () => {
		expect(
			readFirstContentfulPaint([
				{ name: "first-paint", startTime: 700 },
				{ name: "first-contentful-paint", startTime: 900 },
			]),
		).toBe(900);
		expect(
			readFirstContentfulPaint([{ name: "first-paint", startTime: 700 }]),
		).toBeUndefined();
	});
});

describe("initWebVitals", () => {
	test("reports ttfb and fcp eagerly and the rest once on pagehide", () => {
		const dom = stubDom();
		const observers = createObserverStub({
			supported: [
				"largest-contentful-paint",
				"layout-shift",
				"event",
				"first-input",
				"paint",
				"navigation",
			],
		});
		stubNavigationTiming({ responseStart: 812, activationStart: 0 });
		const received = collectMetrics();

		disposeVitals = initWebVitals();

		expect(metricValue(received, "ttfb")).toBe(812);

		observers.emit("paint", [
			{ name: "first-paint", startTime: 700 },
			{ name: "first-contentful-paint", startTime: 912.4 },
		]);
		observers.emit("largest-contentful-paint", [
			{ startTime: 2_400, renderTime: 2_400.7 },
		]);
		observers.emit("layout-shift", [
			{ value: 0.05, startTime: 100 },
			{ value: 0.06, startTime: 400 },
		]);
		observers.emit("event", [{ duration: 176, interactionId: 4 }]);

		expect(metricValue(received, "fcp")).toBe(912);
		expect(metricValue(received, "lcp")).toBeUndefined();
		expect(metricValue(received, "cls")).toBeUndefined();

		dom.fireWindow("pagehide");

		expect(metricValue(received, "lcp")).toBe(2_401);
		expect(metricValue(received, "cls")).toBe(0.11);
		expect(metricValue(received, "inp")).toBe(176);
		expect(received.every((metric) => metric.path === "/boards/:id")).toBe(
			true,
		);

		dom.fireWindow("pagehide");
		expect(received.filter((metric) => metric.metric === "lcp")).toHaveLength(
			1,
		);
	});

	test("finalizes when the page becomes hidden", () => {
		const dom = stubDom();
		const observers = createObserverStub({});
		stubNavigationTiming(undefined);
		const received = collectMetrics();

		disposeVitals = initWebVitals();
		observers.emit("largest-contentful-paint", [{ startTime: 1_500 }]);

		dom.fireDocument("visibilitychange");
		expect(metricValue(received, "lcp")).toBeUndefined();

		dom.hide();
		dom.fireDocument("visibilitychange");
		expect(metricValue(received, "lcp")).toBe(1_500);
	});

	test("stops updating lcp after the first interaction", () => {
		const dom = stubDom();
		const observers = createObserverStub({});
		stubNavigationTiming(undefined);
		const received = collectMetrics();

		disposeVitals = initWebVitals();
		observers.emit("largest-contentful-paint", [{ startTime: 1_200 }]);
		dom.fireWindow("pointerdown");
		observers.emit("largest-contentful-paint", [{ startTime: 4_000 }]);
		dom.fireWindow("pagehide");

		expect(metricValue(received, "lcp")).toBe(1_200);
	});

	test("degrades when entry types are unsupported or observe throws", () => {
		const dom = stubDom();
		const observers = createObserverStub({
			supported: ["paint", "navigation", "largest-contentful-paint"],
			failing: ["largest-contentful-paint"],
		});
		stubNavigationTiming({ responseStart: 400 });
		const received = collectMetrics();

		expect(() => {
			disposeVitals = initWebVitals();
		}).not.toThrow();
		expect(observers.observedTypes()).toEqual(["paint", "navigation"]);

		observers.emit("paint", [
			{ name: "first-contentful-paint", startTime: 600 },
		]);
		dom.fireWindow("pagehide");

		expect(metricValue(received, "ttfb")).toBe(400);
		expect(metricValue(received, "fcp")).toBe(600);
		expect(metricValue(received, "lcp")).toBeUndefined();
	});

	test("still reports navigation timing without PerformanceObserver", () => {
		stubDom();
		vi.stubGlobal("PerformanceObserver", undefined);
		stubNavigationTiming({ responseStart: 250 });
		const received = collectMetrics();

		disposeVitals = initWebVitals();

		expect(metricValue(received, "ttfb")).toBe(250);
	});

	test("is a no-op without a DOM and reports nothing", () => {
		createObserverStub({});
		stubNavigationTiming({ responseStart: 250 });
		const received = collectMetrics();

		const dispose = initWebVitals();
		dispose();

		expect(received).toHaveLength(0);
	});

	test("a second call reuses the first registration", () => {
		const dom = stubDom();
		createObserverStub({});
		stubNavigationTiming(undefined);
		collectMetrics();

		disposeVitals = initWebVitals();
		const second = initWebVitals();

		expect(second).toBe(disposeVitals);
		expect(dom.windowListeners("pagehide")).toBe(1);
	});

	test("detaches the observers and listeners on dispose", () => {
		const dom = stubDom();
		const observers = createObserverStub({});
		stubNavigationTiming(undefined);
		const received = collectMetrics();

		const dispose = initWebVitals();
		observers.emit("largest-contentful-paint", [{ startTime: 1_000 }]);
		dispose();

		expect(observers.observedTypes()).toHaveLength(0);
		expect(dom.windowListeners("pagehide")).toBe(0);
		dom.fireWindow("pagehide");
		expect(received).toHaveLength(0);
	});
});

describe("telemetry client performance transport", () => {
	const profile = {
		bits: [],
		created: "",
		updated: "",
		name: "test",
	} as IProfile;
	let clients: ITelemetryClient[] = [];

	function capturedMetric(
		metric: ITelemetryCapturedPerfMetric["metric"],
	): ITelemetryCapturedPerfMetric {
		return {
			metric,
			value: 1_200,
			path: "/boards/:id",
			client_ts: "2026-07-26T00:00:00.000Z",
		};
	}

	function createHarness(
		overrides: Partial<Parameters<typeof createTelemetryClient>[0]> = {},
	) {
		const post = vi.fn().mockResolvedValue({ accepted: 0 });
		const client = createTelemetryClient({
			apiState: { post } as unknown as IApiState,
			getProfile: () => profile,
			isEnabled: () => true,
			getAnonId: () => "anon-1234",
			source: "web",
			appVersion: "2.0.0",
			platform: "linux",
			...overrides,
		});
		clients.push(client);
		return { client, post };
	}

	afterEach(() => {
		for (const client of clients) client.dispose();
		clients = [];
	});

	test("posts metric batches to telemetry/performance with the locked body", async () => {
		const { client, post } = createHarness({ release: "2.0.0-beta.1" });
		client.perfSink(capturedMetric("lcp"));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(1);
		expect(post).toHaveBeenCalledWith(profile, "telemetry/performance", {
			anon_id: "anon-1234",
			source: "web",
			release: "2.0.0-beta.1",
			platform: "linux",
			metrics: [capturedMetric("lcp")],
		});
	});

	test("splits metric batches at the 50 per request ingest cap", async () => {
		const { client, post } = createHarness();
		client.enqueuePerfMetrics(
			Array.from({ length: 60 }, () => capturedMetric("inp")),
		);
		await client.flush();

		expect(post.mock.calls.map((call) => call[2].metrics.length)).toEqual([
			50, 10,
		]);
	});

	test("drops metrics while usage telemetry is disabled and on clear", async () => {
		const disabled = createHarness({ isEnabled: () => false });
		disabled.client.perfSink(capturedMetric("cls"));
		await disabled.client.flush();
		expect(disabled.post).not.toHaveBeenCalled();

		const enabled = createHarness();
		enabled.client.perfSink(capturedMetric("ttfb"));
		enabled.client.clear();
		await enabled.client.flush();
		expect(enabled.post).not.toHaveBeenCalled();
	});

	test("carries a captured metric from the sink to the ingest body", async () => {
		const { client, post } = createHarness();
		removeSink = setTelemetryPerfSink(client.perfSink);

		capturePerfMetric("app_start", 842.2);
		await client.flush();

		expect(post.mock.calls[0]?.[2].metrics[0]).toMatchObject({
			metric: "app_start",
			value: 842,
		});
	});
});
