import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../schema/profile/profile";
import type { ITelemetryCapturedEvent, TelemetryEventSink } from "./capture";
import type { ITelemetryCapturedError, TelemetryErrorSink } from "./errors";
import type {
	ITelemetryCapturedPerfMetric,
	TelemetryPerfSink,
} from "./performance";
import {
	type ITelemetryCapturedSession,
	type TelemetrySessionSink,
	flushTelemetrySessionForUnload,
} from "./session";
import type { ITelemetryCapturedSpan, TelemetrySpanSink } from "./tracing";

export interface ITelemetryClientOptions {
	apiState: IApiState;
	getProfile: () => IProfile | undefined;
	isEnabled: () => boolean;
	getAnonId: () => string | undefined;
	source: "desktop" | "web";
	appVersion?: string;
	platform?: string;
	/** Release identifier for crash reports and sessions; defaults to `appVersion`. */
	release?: string;
	/**
	 * Consent gate for crash reports and sessions. Crash reporting is a separate
	 * consent from usage telemetry and defaults ON, so this defaults to enabled;
	 * callers must pass the user's crash-report setting to honour an opt-out.
	 */
	isCrashEnabled?: () => boolean;
	flushIntervalMs?: number;
	maxBatchSize?: number;
	maxQueueSize?: number;
	/** Ingest path for usage events. */
	eventPath?: string;
	/** Ingest path for crash reports. */
	errorPath?: string;
	/** Ingest path for release-health sessions. */
	sessionPath?: string;
	/** Ingest path for client trace spans. */
	spanPath?: string;
	/** Ingest path for performance metrics. */
	perfPath?: string;
	/** Hand-off transport for unload; returns true when the body was accepted. */
	beacon?: (body: unknown) => boolean;
	/** Unload transport for crash reports and sessions; receives the ingest path. */
	crashBeacon?: (body: unknown, path: string) => boolean;
	/** Unload transport for spans and performance metrics; receives the ingest path. */
	usageBeacon?: (body: unknown, path: string) => boolean;
}

export interface ITelemetryClient {
	sink: TelemetryEventSink;
	errorSink: TelemetryErrorSink;
	sessionSink: TelemetrySessionSink;
	spanSink: TelemetrySpanSink;
	perfSink: TelemetryPerfSink;
	enqueue(events: ITelemetryCapturedEvent[]): void;
	enqueueErrors(errors: ITelemetryCapturedError[]): void;
	enqueueSessions(sessions: ITelemetryCapturedSession[]): void;
	enqueueSpans(spans: ITelemetryCapturedSpan[]): void;
	enqueuePerfMetrics(metrics: ITelemetryCapturedPerfMetric[]): void;
	flush(): Promise<void>;
	/** Drops queued usage events, spans and metrics; resets retry state. */
	clear(): void;
	/** Drops queued crash reports and sessions; usage events are untouched. */
	clearCrashReports(): void;
	dispose(): void;
}

const DEFAULT_FLUSH_INTERVAL_MS = 15_000;
const DEFAULT_MAX_BATCH_SIZE = 50;
const DEFAULT_MAX_QUEUE_SIZE = 256;
const MAX_REQUESTS_PER_FLUSH = 5;
const MAX_ERROR_BATCH_SIZE = 20;
const MAX_ERROR_QUEUE_SIZE = 64;
const MAX_SESSION_BATCH_SIZE = 50;
const MAX_SESSION_QUEUE_SIZE = 64;
const MAX_SPAN_BATCH_SIZE = 200;
const MAX_SPAN_QUEUE_SIZE = 600;
const MAX_PERF_BATCH_SIZE = 50;
const MAX_PERF_QUEUE_SIZE = 128;
const EVENT_PATH = "telemetry/events";
const ERROR_PATH = "telemetry/errors";
const SESSION_PATH = "telemetry/sessions";
const SPAN_PATH = "telemetry/spans";
const PERF_PATH = "telemetry/performance";

interface ITelemetryQueueConfig<T> {
	path: string;
	maxBatchSize: number;
	maxQueueSize: number;
	isEnabled: () => boolean;
	buildBody: (anonId: string, items: T[]) => unknown;
	beacon?: (body: unknown, path: string) => boolean;
}

interface ITelemetryQueue<T> {
	enqueue(items: T[]): void;
	flush(): Promise<void>;
	/** Consumes the post-failure tick suppression flag. */
	consumeSkip(): boolean;
	drainToBeacon(): void;
	clear(): void;
}

export function createTelemetryClient(
	options: ITelemetryClientOptions,
): ITelemetryClient {
	const flushIntervalMs = options.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS;
	const maxBatchSize = options.maxBatchSize ?? DEFAULT_MAX_BATCH_SIZE;
	const maxQueueSize = options.maxQueueSize ?? DEFAULT_MAX_QUEUE_SIZE;
	const release = options.release ?? options.appVersion;
	const isCrashEnabled = options.isCrashEnabled ?? (() => true);

	let disposed = false;
	// Tracked as the in-flight promise rather than a boolean: a boolean cleared in
	// `finally` latches forever if a transport promise never settles, silently
	// killing telemetry for the rest of the session.
	let flushing: Promise<void> | null = null;

	const createQueue = <T>(
		config: ITelemetryQueueConfig<T>,
	): ITelemetryQueue<T> => {
		const queue: T[] = [];
		let pendingRetry = false;
		let skipNextTick = false;

		const enqueue = (items: T[]) => {
			if (disposed || items.length === 0) return;
			try {
				if (!config.isEnabled()) return;
			} catch {
				return;
			}
			queue.push(...items);
			if (queue.length > config.maxQueueSize) {
				queue.splice(0, queue.length - config.maxQueueSize);
			}
		};

		const flush = async () => {
			try {
				if (!config.isEnabled()) return;
				const profile = options.getProfile();
				const anonId = options.getAnonId();
				if (!profile || !anonId || queue.length === 0) return;
				for (
					let request = 0;
					request < MAX_REQUESTS_PER_FLUSH && queue.length > 0;
					request++
				) {
					const batch = queue.splice(0, config.maxBatchSize);
					try {
						await options.apiState.post(
							profile,
							config.path,
							config.buildBody(anonId, batch),
						);
						pendingRetry = false;
					} catch {
						if (!pendingRetry) {
							queue.unshift(...batch);
							pendingRetry = true;
						}
						skipNextTick = true;
						return;
					}
				}
			} catch {
				// Telemetry is best-effort and must never affect the application path.
			}
		};

		const drainToBeacon = () => {
			const beacon = config.beacon;
			if (!beacon || queue.length === 0) return;
			try {
				if (!config.isEnabled()) return;
				const profile = options.getProfile();
				const anonId = options.getAnonId();
				if (!profile || !anonId) return;
				const batch = queue.slice(0, config.maxBatchSize);
				if (beacon(config.buildBody(anonId, batch), config.path)) {
					queue.splice(0, batch.length);
				}
			} catch {
				// Telemetry is best-effort and must never affect the application path.
			}
		};

		return {
			enqueue,
			flush,
			consumeSkip: () => {
				const skip = skipNextTick;
				skipNextTick = false;
				return skip;
			},
			drainToBeacon,
			clear: () => {
				queue.length = 0;
				pendingRetry = false;
				skipNextTick = false;
			},
		};
	};

	const eventQueue = createQueue<ITelemetryCapturedEvent>({
		path: options.eventPath ?? EVENT_PATH,
		maxBatchSize,
		maxQueueSize,
		isEnabled: options.isEnabled,
		buildBody: (anonId, events) => ({
			anon_id: anonId,
			source: options.source,
			app_version: options.appVersion ?? null,
			platform: options.platform ?? null,
			events,
		}),
		beacon: options.beacon
			? (body) => options.beacon?.(body) === true
			: undefined,
	});

	const errorQueue = createQueue<ITelemetryCapturedError>({
		path: options.errorPath ?? ERROR_PATH,
		maxBatchSize: MAX_ERROR_BATCH_SIZE,
		maxQueueSize: MAX_ERROR_QUEUE_SIZE,
		isEnabled: isCrashEnabled,
		buildBody: (anonId, errors) => ({
			anon_id: anonId,
			source: options.source,
			app_version: options.appVersion ?? null,
			release: release ?? null,
			platform: options.platform ?? null,
			errors,
		}),
		beacon: options.crashBeacon,
	});

	const sessionQueue = createQueue<ITelemetryCapturedSession>({
		path: options.sessionPath ?? SESSION_PATH,
		maxBatchSize: MAX_SESSION_BATCH_SIZE,
		maxQueueSize: MAX_SESSION_QUEUE_SIZE,
		isEnabled: isCrashEnabled,
		buildBody: (anonId, sessions) => ({
			anon_id: anonId,
			source: options.source,
			release: release ?? null,
			platform: options.platform ?? null,
			sessions,
		}),
		beacon: options.crashBeacon,
	});

	const spanQueue = createQueue<ITelemetryCapturedSpan>({
		path: options.spanPath ?? SPAN_PATH,
		maxBatchSize: MAX_SPAN_BATCH_SIZE,
		maxQueueSize: MAX_SPAN_QUEUE_SIZE,
		isEnabled: options.isEnabled,
		buildBody: (anonId, spans) => ({
			anon_id: anonId,
			source: options.source,
			release: release ?? null,
			platform: options.platform ?? null,
			spans,
		}),
		beacon: options.usageBeacon,
	});

	const perfQueue = createQueue<ITelemetryCapturedPerfMetric>({
		path: options.perfPath ?? PERF_PATH,
		maxBatchSize: MAX_PERF_BATCH_SIZE,
		maxQueueSize: MAX_PERF_QUEUE_SIZE,
		isEnabled: options.isEnabled,
		buildBody: (anonId, metrics) => ({
			anon_id: anonId,
			source: options.source,
			release: release ?? null,
			platform: options.platform ?? null,
			metrics,
		}),
		beacon: options.usageBeacon,
	});

	const queues = [eventQueue, errorQueue, sessionQueue, spanQueue, perfQueue];

	const runFlush = (skippable: boolean): Promise<void> => {
		if (disposed) return Promise.resolve();
		if (flushing) return flushing;
		const pending = (async () => {
			for (const queue of queues) {
				if (skippable && queue.consumeSkip()) continue;
				await queue.flush();
			}
		})().finally(() => {
			if (flushing === pending) flushing = null;
		});
		flushing = pending;
		return pending;
	};

	const flush = () => runFlush(false);

	const flushDue = () => runFlush(true);

	const timer = setInterval(() => {
		void flushDue();
	}, flushIntervalMs);

	const onPageHide = () => {
		// Pulled, not awaited: the session module registers its own unload
		// listener and may run after this one, which would leave the terminal
		// record (final status plus duration) queued with nothing left to drain.
		flushTelemetrySessionForUnload();
		for (const queue of queues) queue.drainToBeacon();
		void flush();
	};
	const onVisibilityChange = () => {
		if (document.visibilityState === "hidden") void flush();
	};
	if (typeof window !== "undefined") {
		window.addEventListener("pagehide", onPageHide);
	}
	if (typeof document !== "undefined") {
		document.addEventListener("visibilitychange", onVisibilityChange);
	}

	return {
		sink: (event) => eventQueue.enqueue([event]),
		errorSink: (error) => errorQueue.enqueue([error]),
		sessionSink: (session) => sessionQueue.enqueue([session]),
		spanSink: (span) => spanQueue.enqueue([span]),
		perfSink: (metric) => perfQueue.enqueue([metric]),
		enqueue: (events) => eventQueue.enqueue(events),
		enqueueErrors: (errors) => errorQueue.enqueue(errors),
		enqueueSessions: (sessions) => sessionQueue.enqueue(sessions),
		enqueueSpans: (spans) => spanQueue.enqueue(spans),
		enqueuePerfMetrics: (metrics) => perfQueue.enqueue(metrics),
		flush,
		clear: () => {
			eventQueue.clear();
			spanQueue.clear();
			perfQueue.clear();
		},
		clearCrashReports: () => {
			errorQueue.clear();
			sessionQueue.clear();
		},
		dispose: () => {
			disposed = true;
			clearInterval(timer);
			if (typeof window !== "undefined") {
				window.removeEventListener("pagehide", onPageHide);
			}
			if (typeof document !== "undefined") {
				document.removeEventListener("visibilitychange", onVisibilityChange);
			}
			for (const queue of queues) queue.clear();
		},
	};
}
