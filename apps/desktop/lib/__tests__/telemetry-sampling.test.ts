import type { IApiState, IProfile } from "@flow-like/flow-like-ui";
import {
	type ITelemetryCapturedEvent,
	captureTelemetryEvent,
	setTelemetryEventSink,
} from "@flow-like/flow-like-ui/lib/telemetry/capture";
import { capturePageView } from "@flow-like/flow-like-ui/lib/telemetry/page-view";
import {
	type ITelemetrySamplingConfig,
	TELEMETRY_CONFIG_PATH,
	TELEMETRY_PAGE_VIEW_EVENT,
	createTelemetrySamplingFetcher,
	initTelemetrySampling,
	resetTelemetrySampling,
	shouldSampleEvent,
} from "@flow-like/flow-like-ui/lib/telemetry/sampling";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

let removeSink: (() => void) | undefined;

function samplingConfig(
	pageView: number,
	event: number,
	enabled = true,
): ITelemetrySamplingConfig {
	return { sampling: { pageView, event }, enabled };
}

function loadConfig(config: unknown) {
	return initTelemetrySampling(async () => config as ITelemetrySamplingConfig);
}

function collectEvents() {
	const received: ITelemetryCapturedEvent[] = [];
	removeSink = setTelemetryEventSink((event) => received.push(event));
	return received;
}

function drainPending() {
	const remove = setTelemetryEventSink(() => undefined);
	remove();
}

beforeEach(() => {
	resetTelemetrySampling();
});

afterEach(() => {
	removeSink?.();
	removeSink = undefined;
	drainPending();
	resetTelemetrySampling();
	vi.restoreAllMocks();
});

describe("telemetry sampling rates", () => {
	test("keeps every event at rate 1", async () => {
		await loadConfig(samplingConfig(1, 1));

		for (let index = 0; index < 50; index++) {
			expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
			expect(shouldSampleEvent(`event_${index}`)).toBe(true);
		}
	});

	test("drops every event at rate 0", async () => {
		await loadConfig(samplingConfig(0, 0));

		for (let index = 0; index < 50; index++) {
			expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
			expect(shouldSampleEvent(`event_${index}`)).toBe(false);
		}
	});

	test("page views and other events use their own rate", async () => {
		await loadConfig(samplingConfig(0, 1));

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
		expect(shouldSampleEvent("app_started")).toBe(true);
	});

	test("deterministic rates never consult the random source", async () => {
		const random = vi.spyOn(Math, "random");
		await loadConfig(samplingConfig(0, 1));

		shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT);
		shouldSampleEvent("app_started");

		expect(random).not.toHaveBeenCalled();
	});

	test("clamps out-of-range and malformed rates", async () => {
		await loadConfig(samplingConfig(7, -3));
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		expect(shouldSampleEvent("app_started")).toBe(false);

		resetTelemetrySampling();
		await loadConfig({
			sampling: { pageView: "0", event: null },
			enabled: true,
		});
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		expect(shouldSampleEvent("app_started")).toBe(true);
	});

	test("an explicitly disabled platform drops usage events", async () => {
		await loadConfig(samplingConfig(1, 1, false));

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
		expect(shouldSampleEvent("app_started")).toBe(false);
	});
});

describe("telemetry sampling fail-open", () => {
	test("an unreachable config keeps everything", async () => {
		await initTelemetrySampling(async () => {
			throw new Error("offline");
		});

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		expect(shouldSampleEvent("app_started")).toBe(true);
	});

	test("a config that resolves nothing keeps everything", async () => {
		await loadConfig(undefined);

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		expect(shouldSampleEvent("app_started")).toBe(true);
	});

	test("captures before the config lands are kept but not memoized", async () => {
		let resolveConfig: ((config: ITelemetrySamplingConfig) => void) | undefined;
		const pending = new Promise<ITelemetrySamplingConfig>((resolve) => {
			resolveConfig = resolve;
		});
		const loading = initTelemetrySampling(() => pending);

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);

		resolveConfig?.(samplingConfig(0, 1));
		await loading;

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
	});

	test("a failing fetch is retried by a later init call, then gives up", async () => {
		const fetcher = vi.fn(async () => {
			throw new Error("offline");
		});

		for (let attempt = 0; attempt < 6; attempt++) {
			await initTelemetrySampling(fetcher);
			expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		}

		expect(fetcher).toHaveBeenCalledTimes(3);
	});

	test("a retry after a failed fetch applies the config", async () => {
		let offline = true;
		const fetcher = vi.fn(async () => {
			if (offline) throw new Error("offline");
			return samplingConfig(0, 1);
		});

		await initTelemetrySampling(fetcher);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);

		offline = false;
		await initTelemetrySampling(fetcher);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
	});

	test("concurrent init calls fetch the config once per session", async () => {
		const fetcher = vi.fn(async () => samplingConfig(0, 0));

		await Promise.all([
			initTelemetrySampling(fetcher),
			initTelemetrySampling(fetcher),
		]);
		await initTelemetrySampling(fetcher);

		expect(fetcher).toHaveBeenCalledTimes(1);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
	});
});

describe("createTelemetrySamplingFetcher", () => {
	const profile = { id: "profile" } as unknown as IProfile;

	function apiStateWith(get: IApiState["get"]) {
		return { get } as unknown as IApiState;
	}

	test("reads the public config path through the api state", async () => {
		const get = vi.fn(async () => samplingConfig(0, 1));
		const apiState = apiStateWith(get as unknown as IApiState["get"]);

		await initTelemetrySampling(
			createTelemetrySamplingFetcher(apiState, () => profile),
		);

		expect(get).toHaveBeenCalledWith(profile, TELEMETRY_CONFIG_PATH);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
	});

	test("stays retryable while no profile is available", async () => {
		const session: { current?: IProfile } = {};
		const get = vi.fn(async () => samplingConfig(0, 1));
		const fetcher = createTelemetrySamplingFetcher(
			apiStateWith(get as unknown as IApiState["get"]),
			() => session.current,
		);

		await initTelemetrySampling(fetcher);
		expect(get).not.toHaveBeenCalled();
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);

		session.current = profile;
		await initTelemetrySampling(fetcher);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
	});
});

describe("telemetry sampling stability", () => {
	test("a sampled-out name stays out for the rest of the session", async () => {
		const random = vi.spyOn(Math, "random").mockReturnValue(0.9);
		await loadConfig(samplingConfig(0.5, 1));

		// The first page view is always kept so the install stays visible.
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		for (let index = 0; index < 20; index++) {
			expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);
		}
		expect(random).toHaveBeenCalledTimes(1);
	});

	test("a sampled-in name stays in for the rest of the session", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.1);
		await loadConfig(samplingConfig(0.5, 1));

		for (let index = 0; index < 20; index++) {
			expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		}
	});

	test("decisions are per event name", async () => {
		vi.spyOn(Math, "random")
			.mockReturnValueOnce(0.9)
			.mockReturnValueOnce(0.1)
			.mockReturnValue(0.9);
		await loadConfig(samplingConfig(1, 0.5));

		expect(shouldSampleEvent("first")).toBe(false);
		expect(shouldSampleEvent("second")).toBe(true);
		expect(shouldSampleEvent("first")).toBe(false);
		expect(shouldSampleEvent("second")).toBe(true);
	});

	test("a new session redraws the decision", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.9);
		await loadConfig(samplingConfig(0.5, 1));
		shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);

		resetTelemetrySampling();
		vi.spyOn(Math, "random").mockReturnValue(0.1);
		await loadConfig(samplingConfig(0.5, 1));
		shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
	});

	test("the decision cache is capped so unbounded names cannot grow it", async () => {
		const random = vi.spyOn(Math, "random").mockReturnValue(0.9);
		await loadConfig(samplingConfig(1, 0.5));

		for (let index = 0; index < 400; index++) {
			expect(shouldSampleEvent(`event_${index}`)).toBe(false);
		}

		random.mockReturnValue(0.1);
		expect(shouldSampleEvent("event_0")).toBe(false);
		expect(shouldSampleEvent("event_399")).toBe(true);
	});
});

describe("install visibility", () => {
	test("the first page view of a session lands even when sampled out", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.99);
		await loadConfig(samplingConfig(0.25, 1));
		const received = collectEvents();

		capturePageView("/library");
		capturePageView("/store");
		capturePageView("/settings");

		expect(received.map((event) => event.props)).toEqual([
			{ path: "/library" },
		]);
	});

	test("an explicit rate of 0 keeps nothing, not even the first page view", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0);
		await loadConfig(samplingConfig(0, 1));
		const received = collectEvents();

		capturePageView("/library");
		capturePageView("/store");

		expect(received).toHaveLength(0);
	});

	test("a disabled platform keeps nothing, not even the first page view", async () => {
		await loadConfig(samplingConfig(0.25, 1, false));
		const received = collectEvents();

		capturePageView("/library");

		expect(received).toHaveLength(0);
	});

	test("the exemption applies once per session, not once per config", async () => {
		vi.spyOn(Math, "random").mockReturnValue(0.99);
		await loadConfig(samplingConfig(0.25, 1));

		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(false);

		resetTelemetrySampling();
		await loadConfig(samplingConfig(0.25, 1));
		expect(shouldSampleEvent(TELEMETRY_PAGE_VIEW_EVENT)).toBe(true);
	});
});

describe("sampling is applied at capture time", () => {
	test("a sampled-out page view never reaches the sink", async () => {
		await loadConfig(samplingConfig(0, 1));
		const received = collectEvents();

		capturePageView("/library");
		captureTelemetryEvent("page_view", { path: "/library" });

		expect(received).toHaveLength(0);
	});

	test("a sampled-out event never reaches the sink", async () => {
		await loadConfig(samplingConfig(1, 0));
		const received = collectEvents();

		captureTelemetryEvent("app_started");

		expect(received).toHaveLength(0);
	});

	test("a sampled-out event is not buffered while no sink is attached", async () => {
		await loadConfig(samplingConfig(0, 0));

		capturePageView("/library");
		captureTelemetryEvent("app_started");

		const received = collectEvents();
		expect(received).toHaveLength(0);
	});

	test("sampled-in captures still reach the sink", async () => {
		await loadConfig(samplingConfig(1, 1));
		const received = collectEvents();

		capturePageView("/apps/0a1b2c3d4e5f6a7b8c?token=nope");
		captureTelemetryEvent("app_started", { cold: true });

		expect(received.map((event) => event.name)).toEqual([
			"page_view",
			"app_started",
		]);
		expect(received[0]?.props).toEqual({ path: "/apps/:id" });
	});

	test("captures are kept while the config has not been initialized", () => {
		const received = collectEvents();

		capturePageView("/library");
		captureTelemetryEvent("app_started");

		expect(received.map((event) => event.name)).toEqual([
			"page_view",
			"app_started",
		]);
	});
});
