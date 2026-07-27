import {
	type ITelemetryClient,
	createTelemetryClient,
} from "@flow-like/flow-like-ui/lib/telemetry/client";
import {
	type ITelemetryCapturedSpan,
	clearActiveTelemetrySpans,
	formatTraceparent,
	getActiveTraceContext,
	getTelemetryTraceSampleRate,
	getTelemetryTraceparent,
	parseTraceparent,
	setTelemetrySpanSink,
	setTelemetryTraceSampleRate,
	startTelemetrySpan,
} from "@flow-like/flow-like-ui/lib/telemetry/tracing";
import type { IApiState } from "@flow-like/flow-like-ui/state/backend-state/api-state";
import type { IProfile } from "@flow-like/flow-like-ui/types";
import { afterEach, describe, expect, test, vi } from "vitest";

const TRACE_ID = /^[0-9a-f]{32}$/;
const SPAN_ID = /^[0-9a-f]{16}$/;
const TRACEPARENT = /^00-[0-9a-f]{32}-[0-9a-f]{16}-(?:00|01)$/;

let removeSink: (() => void) | undefined;

function collectSpans() {
	const received: ITelemetryCapturedSpan[] = [];
	removeSink = setTelemetrySpanSink((span) => received.push(span));
	return received;
}

function drainPendingSpans() {
	const remove = setTelemetrySpanSink(() => undefined);
	remove();
}

afterEach(() => {
	clearActiveTelemetrySpans();
	removeSink?.();
	removeSink = undefined;
	drainPendingSpans();
	setTelemetryTraceSampleRate(0.05);
	vi.restoreAllMocks();
});

describe("telemetry span ids", () => {
	test("mints lowercase hex trace and span ids of the W3C lengths", () => {
		setTelemetryTraceSampleRate(1);
		const span = startTelemetrySpan("board.render");

		expect(span.traceId).toMatch(TRACE_ID);
		expect(span.spanId).toMatch(SPAN_ID);
		expect(span.traceId).not.toMatch(/^0+$/);
		expect(span.spanId).not.toMatch(/^0+$/);
		span.end();
	});

	test("ids are unique across spans", () => {
		setTelemetryTraceSampleRate(1);
		const ids = new Set<string>();
		for (let index = 0; index < 50; index++) {
			const span = startTelemetrySpan("loop");
			ids.add(span.spanId);
			ids.add(span.traceId);
			span.end();
		}

		expect(ids.size).toBe(100);
	});
});

describe("telemetry span emission", () => {
	test("emits a snake_case span body once the span ends", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		const span = startTelemetrySpan("api.post", {
			kind: "client",
			attributes: { path: "/api/v1/telemetry/spans" },
		});
		expect(received).toHaveLength(0);
		span.end();

		expect(received).toHaveLength(1);
		expect(received[0]).toMatchObject({
			trace_id: span.traceId,
			span_id: span.spanId,
			name: "api.post",
			kind: "client",
			status: "ok",
			attributes: { path: "/api/v1/telemetry/spans" },
		});
		expect(received[0]?.parent_span_id).toBeUndefined();
		expect(received[0]?.duration_ms).toBeGreaterThanOrEqual(0);
		expect(Number.isInteger(received[0]?.duration_ms)).toBe(true);
		expect(Date.parse(received[0]?.started_at ?? "")).not.toBeNaN();
	});

	test("defaults to the client kind and records the error status", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		startTelemetrySpan("weird", {
			kind: "teleport" as unknown as ITelemetryCapturedSpan["kind"],
		}).end("error");

		expect(received[0]?.kind).toBe("client");
		expect(received[0]?.status).toBe("error");
	});

	test("caps the span name at 256 characters", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		startTelemetrySpan("n".repeat(400)).end();

		expect(received[0]?.name).toHaveLength(256);
	});

	test("end is idempotent", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		const span = startTelemetrySpan("once");
		span.end();
		span.end("error");

		expect(received).toHaveLength(1);
		expect(received[0]?.status).toBe("ok");
	});

	test("buffers spans without a sink and replays them on attach", () => {
		setTelemetryTraceSampleRate(1);
		startTelemetrySpan("buffered").end();

		const received = collectSpans();
		expect(received).toHaveLength(1);
		expect(received[0]?.name).toBe("buffered");
	});

	test("never throws when the sink throws", () => {
		setTelemetryTraceSampleRate(1);
		removeSink = setTelemetrySpanSink(() => {
			throw new Error("sink exploded");
		});

		expect(() => startTelemetrySpan("safe").end()).not.toThrow();
	});
});

describe("telemetry span attributes", () => {
	test("drops secret-looking keys and merges end attributes", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		startTelemetrySpan("api.post", {
			attributes: { token: "super-secret", status: 200 },
		}).end("ok", { retries: 1 });

		expect(received[0]?.attributes).toEqual({ status: 200, retries: 1 });
	});

	test("drops attributes larger than the 8KiB ingest cap", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		const chunk = Array.from({ length: 20 }, () => "x".repeat(300));
		startTelemetrySpan("huge", {
			attributes: { a: chunk, b: chunk, c: chunk },
		}).end();

		expect(received[0]?.attributes).toBeUndefined();
	});

	test("omits attributes when none survive sanitization", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		startTelemetrySpan("plain", { attributes: { api_key: "nope" } }).end();

		expect(received[0]?.attributes).toBeUndefined();
	});
});

describe("telemetry trace sampling", () => {
	test("rate 0 keeps the context but emits nothing", () => {
		setTelemetryTraceSampleRate(0);
		const received = collectSpans();

		const span = startTelemetrySpan("unsampled");
		expect(span.sampled).toBe(false);
		expect(span.traceId).toMatch(TRACE_ID);
		expect(getTelemetryTraceparent(span)).toMatch(/-00$/);
		span.end();

		expect(received).toHaveLength(0);
	});

	test("the head decision is deterministic for a given random draw", () => {
		setTelemetryTraceSampleRate(0.5);
		vi.spyOn(Math, "random").mockReturnValue(0.4);
		expect(startTelemetrySpan("under").sampled).toBe(true);
		clearActiveTelemetrySpans();

		vi.spyOn(Math, "random").mockReturnValue(0.6);
		expect(startTelemetrySpan("over").sampled).toBe(false);
	});

	test("the sample rate is clamped and invalid values are ignored", () => {
		setTelemetryTraceSampleRate(5);
		expect(getTelemetryTraceSampleRate()).toBe(1);
		setTelemetryTraceSampleRate(-2);
		expect(getTelemetryTraceSampleRate()).toBe(0);
		setTelemetryTraceSampleRate(Number.NaN);
		expect(getTelemetryTraceSampleRate()).toBe(0);
	});

	test("children inherit the trace and the root decision, not the live rate", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		const root = startTelemetrySpan("root", { kind: "internal" });
		setTelemetryTraceSampleRate(0);
		const child = startTelemetrySpan("child");

		expect(child.sampled).toBe(true);
		expect(child.traceId).toBe(root.traceId);

		child.end();
		root.end();

		expect(received.map((span) => span.name)).toEqual(["child", "root"]);
		expect(received[0]?.parent_span_id).toBe(root.spanId);
		expect(received[1]?.parent_span_id).toBeUndefined();
	});

	test("an explicit sampled flag overrides the head decision", () => {
		setTelemetryTraceSampleRate(0);
		const received = collectSpans();

		startTelemetrySpan("forced", { sampled: true }).end();

		expect(received).toHaveLength(1);
	});
});

describe("telemetry trace context", () => {
	test("tracks the active span and restores the parent on end", () => {
		setTelemetryTraceSampleRate(1);
		expect(getActiveTraceContext()).toBeUndefined();

		const root = startTelemetrySpan("root");
		expect(getActiveTraceContext()).toMatchObject({
			traceId: root.traceId,
			spanId: root.spanId,
			sampled: true,
		});

		const child = startTelemetrySpan("child");
		expect(getActiveTraceContext()?.spanId).toBe(child.spanId);

		child.end();
		expect(getActiveTraceContext()?.spanId).toBe(root.spanId);

		root.end();
		expect(getActiveTraceContext()).toBeUndefined();
	});

	test("out-of-order ends remove only their own span", () => {
		setTelemetryTraceSampleRate(1);
		const root = startTelemetrySpan("root");
		const child = startTelemetrySpan("child");

		root.end();
		expect(getActiveTraceContext()?.spanId).toBe(child.spanId);

		child.end();
		expect(getActiveTraceContext()).toBeUndefined();
	});

	test("clearActiveTelemetrySpans drops the stack", () => {
		setTelemetryTraceSampleRate(1);
		startTelemetrySpan("root");
		startTelemetrySpan("child");

		clearActiveTelemetrySpans();
		expect(getActiveTraceContext()).toBeUndefined();
	});
});

describe("traceparent propagation", () => {
	test("formats the W3C header with the sampling flag", () => {
		const traceId = "4bf92f3577b34da6a3ce929d0e0e4736";
		const spanId = "00f067aa0ba902b7";

		expect(formatTraceparent(traceId, spanId, true)).toBe(
			`00-${traceId}-${spanId}-01`,
		);
		expect(formatTraceparent(traceId, spanId, false)).toBe(
			`00-${traceId}-${spanId}-00`,
		);
	});

	test("the active span produces a valid header", () => {
		setTelemetryTraceSampleRate(1);
		expect(getTelemetryTraceparent()).toBeUndefined();

		const span = startTelemetrySpan("api.get");
		const header = getTelemetryTraceparent();

		expect(header).toMatch(TRACEPARENT);
		expect(header).toBe(`00-${span.traceId}-${span.spanId}-01`);
		span.end();
		expect(getTelemetryTraceparent()).toBeUndefined();
	});

	test("parses a valid header and rejects malformed ones", () => {
		expect(
			parseTraceparent(
				"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
			),
		).toEqual({
			traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
			spanId: "00f067aa0ba902b7",
			sampled: true,
		});
		expect(
			parseTraceparent(
				"00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-00",
			)?.sampled,
		).toBe(false);
		expect(parseTraceparent(undefined)).toBeUndefined();
		expect(parseTraceparent("garbage")).toBeUndefined();
		expect(
			parseTraceparent(
				"01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
			),
		).toBeUndefined();
		expect(
			parseTraceparent(
				"00-00000000000000000000000000000000-00f067aa0ba902b7-01",
			),
		).toBeUndefined();
		expect(
			parseTraceparent(
				"00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
			),
		).toBeUndefined();
		expect(
			parseTraceparent(
				"00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01",
			),
		).toBeUndefined();
	});

	test("continues an inbound trace given as a traceparent", () => {
		setTelemetryTraceSampleRate(0);
		const received = collectSpans();

		const span = startTelemetrySpan("continued", {
			traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
		});
		span.end();

		expect(span.traceId).toBe("4bf92f3577b34da6a3ce929d0e0e4736");
		expect(span.sampled).toBe(true);
		expect(received[0]?.parent_span_id).toBe("00f067aa0ba902b7");
	});

	test("an explicit trace id still consults head sampling", () => {
		setTelemetryTraceSampleRate(1);
		const received = collectSpans();

		startTelemetrySpan("adopted", {
			traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
		}).end();

		expect(received[0]?.trace_id).toBe("4bf92f3577b34da6a3ce929d0e0e4736");
	});
});

describe("telemetry client span transport", () => {
	const profile = {
		bits: [],
		created: "",
		updated: "",
		name: "test",
	} as IProfile;
	let clients: ITelemetryClient[] = [];

	function capturedSpan(name: string): ITelemetryCapturedSpan {
		return {
			trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
			span_id: "00f067aa0ba902b7",
			name,
			kind: "client",
			started_at: "2026-07-26T00:00:00.000Z",
			duration_ms: 12,
			status: "ok",
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
			source: "desktop",
			appVersion: "1.2.3",
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

	test("posts span batches to telemetry/spans with the locked body", async () => {
		const { client, post } = createHarness({ release: "1.2.3-beta.1" });
		client.spanSink(capturedSpan("api.post"));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(1);
		expect(post).toHaveBeenCalledWith(profile, "telemetry/spans", {
			anon_id: "anon-1234",
			source: "desktop",
			release: "1.2.3-beta.1",
			platform: "linux",
			spans: [capturedSpan("api.post")],
		});
	});

	test("splits span batches at the 200 per request ingest cap", async () => {
		const { client, post } = createHarness();
		client.enqueueSpans(
			Array.from({ length: 260 }, (_, index) => capturedSpan(`span-${index}`)),
		);
		await client.flush();

		expect(post.mock.calls.map((call) => call[2].spans.length)).toEqual([
			200, 60,
		]);
	});

	test("drops spans while usage telemetry is disabled and on clear", async () => {
		const disabled = createHarness({ isEnabled: () => false });
		disabled.client.spanSink(capturedSpan("dropped"));
		await disabled.client.flush();
		expect(disabled.post).not.toHaveBeenCalled();

		const enabled = createHarness();
		enabled.client.spanSink(capturedSpan("cleared"));
		enabled.client.clear();
		await enabled.client.flush();
		expect(enabled.post).not.toHaveBeenCalled();
	});

	test("carries an ended span from the sink to the ingest body", async () => {
		setTelemetryTraceSampleRate(1);
		const { client, post } = createHarness();
		removeSink = setTelemetrySpanSink(client.spanSink);

		startTelemetrySpan("board.save", { kind: "internal" }).end();
		await client.flush();

		expect(post.mock.calls[0]?.[2].spans[0]).toMatchObject({
			name: "board.save",
			kind: "internal",
			status: "ok",
		});
	});
});
