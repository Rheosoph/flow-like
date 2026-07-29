/**
 * Anonymous performance telemetry. Every metric routed through this module is a
 * timing number keyed to the random anonymous install id: no user identity, no
 * content and no URLs beyond a sanitized, id-free path.
 */

import { sanitizeTelemetryPath } from "./page-view";

export type TelemetryPerfMetricName =
	| "lcp"
	| "inp"
	| "cls"
	| "ttfb"
	| "fcp"
	| "app_start"
	| "screen_load";

export interface ITelemetryCapturedPerfMetric {
	metric: TelemetryPerfMetricName;
	value: number;
	path?: string;
	client_ts: string;
}

export type TelemetryPerfSink = (metric: ITelemetryCapturedPerfMetric) => void;

/** Structural view of the entries the observers hand us; browsers vary. */
export interface IPerfEntryLike {
	name?: string;
	entryType?: string;
	startTime?: number;
	duration?: number;
}

export interface ILayoutShiftEntryLike extends IPerfEntryLike {
	value?: number;
	hadRecentInput?: boolean;
}

export interface IEventTimingEntryLike extends IPerfEntryLike {
	interactionId?: number;
}

export interface ILargestContentfulPaintEntryLike extends IPerfEntryLike {
	renderTime?: number;
	loadTime?: number;
}

export interface INavigationTimingLike extends IPerfEntryLike {
	responseStart?: number;
	activationStart?: number;
}

const PERF_METRIC_NAMES: readonly TelemetryPerfMetricName[] = [
	"lcp",
	"inp",
	"cls",
	"ttfb",
	"fcp",
	"app_start",
	"screen_load",
];

const MAX_PENDING_TELEMETRY_PERF_METRICS = 32;
const MAX_PERF_VALUE_MS = 3_600_000;
const MAX_CLS_VALUE = 1_000;
/** Layout shifts join the current session while both bounds hold. */
const CLS_SESSION_GAP_MS = 1_000;
const CLS_SESSION_WINDOW_MS = 5_000;
/** Interactions below this are indistinguishable from frame noise. */
const INP_DURATION_THRESHOLD_MS = 40;

const PENDING_TELEMETRY_PERF_METRICS: ITelemetryCapturedPerfMetric[] = [];

let telemetryPerfSink: TelemetryPerfSink | undefined;
let webVitalsDispose: (() => void) | undefined;

const NOOP = () => undefined;

function deliverPerfMetric(metric: ITelemetryCapturedPerfMetric) {
	const sink = telemetryPerfSink;
	if (!sink) {
		PENDING_TELEMETRY_PERF_METRICS.push(metric);
		if (
			PENDING_TELEMETRY_PERF_METRICS.length > MAX_PENDING_TELEMETRY_PERF_METRICS
		) {
			PENDING_TELEMETRY_PERF_METRICS.shift();
		}
		return;
	}
	try {
		sink(metric);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

function finiteNumber(value: unknown): number | undefined {
	return typeof value === "number" && Number.isFinite(value)
		? value
		: undefined;
}

/** Milliseconds are whole numbers; CLS is a unitless ratio kept at 4 decimals. */
function normalizePerfValue(
	metric: TelemetryPerfMetricName,
	value: number,
): number | undefined {
	const finite = finiteNumber(value);
	if (finite === undefined || finite < 0) return undefined;
	if (metric === "cls") {
		if (finite > MAX_CLS_VALUE) return undefined;
		return Math.round(finite * 10_000) / 10_000;
	}
	if (finite > MAX_PERF_VALUE_MS) return undefined;
	return Math.round(finite);
}

function currentTelemetryPath(): string | undefined {
	try {
		if (typeof window === "undefined") return undefined;
		const pathname = window.location?.pathname;
		return typeof pathname === "string"
			? sanitizeTelemetryPath(pathname)
			: undefined;
	} catch {
		return undefined;
	}
}

function resolvePerfPath(path: string | undefined): string | undefined {
	if (typeof path === "string" && path.length > 0)
		return sanitizeTelemetryPath(path);
	return currentTelemetryPath();
}

/**
 * Reports a single performance metric. Values are milliseconds except `cls`,
 * which is unitless. Never throws into the application path.
 */
export function capturePerfMetric(
	metric: TelemetryPerfMetricName,
	value: number,
	path?: string,
) {
	try {
		if (!PERF_METRIC_NAMES.includes(metric)) return;
		const normalized = normalizePerfValue(metric, value);
		if (normalized === undefined) return;
		const captured: ITelemetryCapturedPerfMetric = {
			metric,
			value: normalized,
			client_ts: new Date().toISOString(),
		};
		const resolved = resolvePerfPath(path);
		if (resolved) captured.path = resolved;
		deliverPerfMetric(captured);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

/** Register the performance sink. Pending metrics are flushed on attach. */
export function setTelemetryPerfSink(sink: TelemetryPerfSink | undefined) {
	telemetryPerfSink = sink;
	if (sink) {
		for (const metric of PENDING_TELEMETRY_PERF_METRICS.splice(0)) {
			deliverPerfMetric(metric);
		}
	}
	return () => {
		if (telemetryPerfSink === sink) telemetryPerfSink = undefined;
	};
}

interface IClsSession {
	value: number;
	start: number;
	last: number;
	max: number;
}

function createClsSession(): IClsSession {
	return { value: 0, start: 0, last: 0, max: 0 };
}

function stepClsSession(session: IClsSession, entry: ILayoutShiftEntryLike) {
	if (entry.hadRecentInput === true) return;
	const value = finiteNumber(entry.value);
	if (value === undefined || value <= 0) return;
	const start = finiteNumber(entry.startTime) ?? 0;
	const continues =
		session.value > 0 &&
		start - session.last < CLS_SESSION_GAP_MS &&
		start - session.start < CLS_SESSION_WINDOW_MS;
	if (continues) {
		session.value += value;
		session.last = start;
	} else {
		session.value = value;
		session.start = start;
		session.last = start;
	}
	if (session.value > session.max) session.max = session.value;
}

/** CLS is the worst 5s session window, with shifts grouped by a 1s gap. */
export function foldLayoutShiftValue(
	entries: readonly ILayoutShiftEntryLike[],
): number {
	const session = createClsSession();
	for (const entry of entries) stepClsSession(session, entry);
	return session.max;
}

function isInteraction(entry: IEventTimingEntryLike): boolean {
	if (entry.entryType === "first-input") return true;
	const interactionId = finiteNumber(entry.interactionId);
	return interactionId !== undefined && interactionId > 0;
}

/** INP is the worst interaction latency observed during the page load. */
export function foldInteractionLatency(
	entries: readonly IEventTimingEntryLike[],
	current = 0,
): number {
	let worst = current;
	for (const entry of entries) {
		if (!isInteraction(entry)) continue;
		const duration = finiteNumber(entry.duration);
		if (duration === undefined || duration < INP_DURATION_THRESHOLD_MS)
			continue;
		if (duration > worst) worst = duration;
	}
	return worst;
}

/** The final LCP candidate is the latest one the browser reported. */
export function pickLargestContentfulPaint(
	entries: readonly ILargestContentfulPaintEntryLike[],
): number | undefined {
	let best: number | undefined;
	let bestStart = Number.NEGATIVE_INFINITY;
	for (const entry of entries) {
		const value =
			finiteNumber(entry.renderTime) ??
			finiteNumber(entry.loadTime) ??
			finiteNumber(entry.startTime);
		if (value === undefined) continue;
		const start = finiteNumber(entry.startTime) ?? value;
		if (start >= bestStart) {
			bestStart = start;
			best = value;
		}
	}
	return best;
}

export function readTimeToFirstByte(
	navigation: INavigationTimingLike | undefined,
): number | undefined {
	const responseStart = finiteNumber(navigation?.responseStart);
	if (responseStart === undefined || responseStart <= 0) return undefined;
	const activationStart = finiteNumber(navigation?.activationStart) ?? 0;
	return Math.max(0, responseStart - activationStart);
}

export function readFirstContentfulPaint(
	entries: readonly IPerfEntryLike[],
): number | undefined {
	for (const entry of entries) {
		if (entry.name !== "first-contentful-paint") continue;
		const startTime = finiteNumber(entry.startTime);
		if (startTime !== undefined) return startTime;
	}
	return undefined;
}

interface IObserveOptions {
	durationThreshold?: number;
}

function isEntryTypeSupported(type: string): boolean {
	const supported = (
		PerformanceObserver as unknown as { supportedEntryTypes?: unknown }
	).supportedEntryTypes;
	return Array.isArray(supported) ? supported.includes(type) : true;
}

/** Attaches one observer; unsupported entry types degrade to `undefined`. */
function observeEntries(
	type: string,
	handler: (entries: IPerfEntryLike[]) => void,
	options?: IObserveOptions,
): (() => void) | undefined {
	try {
		if (typeof PerformanceObserver === "undefined") return undefined;
		if (!isEntryTypeSupported(type)) return undefined;
		const observer = new PerformanceObserver((list) => {
			try {
				handler(list.getEntries() as unknown as IPerfEntryLike[]);
			} catch {
				// Telemetry is best-effort and must never affect the application path.
			}
		});
		observer.observe({
			type,
			buffered: true,
			...(options ?? {}),
		} as PerformanceObserverInit);
		return () => {
			try {
				observer.disconnect();
			} catch {
				// Telemetry is best-effort and must never affect the application path.
			}
		};
	} catch {
		return undefined;
	}
}

function readNavigationTiming(): INavigationTimingLike | undefined {
	try {
		if (typeof performance === "undefined") return undefined;
		const entries = performance.getEntriesByType?.("navigation");
		return entries?.[0] as INavigationTimingLike | undefined;
	} catch {
		return undefined;
	}
}

/**
 * Starts Core Web Vitals collection for the current page load using native
 * `PerformanceObserver` only. Each metric is reported at most once per load;
 * LCP, CLS and INP are finalized when the page is hidden. Safe on the server,
 * on unsupported browsers and when called more than once.
 */
export function initWebVitals(): () => void {
	if (typeof window === "undefined" || typeof document === "undefined")
		return NOOP;
	if (webVitalsDispose) return webVitalsDispose;
	try {
		const path = currentTelemetryPath();
		const reported = new Set<TelemetryPerfMetricName>();
		const report = (metric: TelemetryPerfMetricName, value: number) => {
			if (reported.has(metric)) return;
			reported.add(metric);
			capturePerfMetric(metric, value, path);
		};

		const session = createClsSession();
		let largestContentfulPaint: number | undefined;
		let largestContentfulPaintFrozen = false;
		let interactionLatency = 0;

		const reportTimeToFirstByte = (
			navigation: INavigationTimingLike | undefined,
		) => {
			const ttfb = readTimeToFirstByte(navigation);
			if (ttfb !== undefined) report("ttfb", ttfb);
		};

		const disposers: (() => void)[] = [];
		const push = (disposer: (() => void) | undefined) => {
			if (disposer) disposers.push(disposer);
		};

		push(
			observeEntries("largest-contentful-paint", (entries) => {
				if (largestContentfulPaintFrozen) return;
				const candidate = pickLargestContentfulPaint(entries);
				if (candidate !== undefined) largestContentfulPaint = candidate;
			}),
		);
		push(
			observeEntries("layout-shift", (entries) => {
				for (const entry of entries) stepClsSession(session, entry);
			}),
		);
		push(
			observeEntries(
				"event",
				(entries) => {
					interactionLatency = foldInteractionLatency(
						entries,
						interactionLatency,
					);
				},
				{ durationThreshold: INP_DURATION_THRESHOLD_MS },
			),
		);
		push(
			observeEntries("first-input", (entries) => {
				interactionLatency = foldInteractionLatency(
					entries.map((entry) => ({ ...entry, entryType: "first-input" })),
					interactionLatency,
				);
			}),
		);
		push(
			observeEntries("paint", (entries) => {
				const fcp = readFirstContentfulPaint(entries);
				if (fcp !== undefined) report("fcp", fcp);
			}),
		);
		push(
			observeEntries("navigation", (entries) => {
				reportTimeToFirstByte(entries[0] as INavigationTimingLike | undefined);
			}),
		);
		reportTimeToFirstByte(readNavigationTiming());

		/** LCP stops changing at the first interaction, mirroring the spec. */
		const freezeLcp = () => {
			largestContentfulPaintFrozen = true;
		};
		const finalize = () => {
			if (largestContentfulPaint !== undefined)
				report("lcp", largestContentfulPaint);
			if (session.max > 0) report("cls", session.max);
			if (interactionLatency > 0) report("inp", interactionLatency);
		};
		const onVisibilityChange = () => {
			if (document.visibilityState === "hidden") finalize();
		};

		window.addEventListener("keydown", freezeLcp, { once: true });
		window.addEventListener("pointerdown", freezeLcp, { once: true });
		window.addEventListener("pagehide", finalize);
		document.addEventListener("visibilitychange", onVisibilityChange);
		push(() => {
			window.removeEventListener("keydown", freezeLcp);
			window.removeEventListener("pointerdown", freezeLcp);
			window.removeEventListener("pagehide", finalize);
			document.removeEventListener("visibilitychange", onVisibilityChange);
		});

		webVitalsDispose = () => {
			webVitalsDispose = undefined;
			for (const disposer of disposers.splice(0)) {
				try {
					disposer();
				} catch {
					// Telemetry is best-effort and must never affect the application path.
				}
			}
		};
		return webVitalsDispose;
	} catch {
		return NOOP;
	}
}
