"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ITelemetryClient,
	TelemetryConsentPrompt,
	addTelemetryBreadcrumb,
	capturePageView,
	capturePerfMetric,
	captureTelemetryError,
	captureTelemetryEvent,
	clearActiveTelemetrySpans,
	createTelemetryClient,
	createTelemetrySamplingFetcher,
	endTelemetrySession,
	initTelemetrySampling,
	initWebVitals,
	isBenignBrowserError,
	markTelemetrySessionCrashed,
	sanitizeTelemetryPath,
	setTelemetryErrorSink,
	setTelemetryEventSink,
	setTelemetryPerfSink,
	setTelemetrySessionSink,
	setTelemetrySpanSink,
	startTelemetrySession,
	useBackend,
	useFeatures,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { setFlowPilotProductionMetricsSink } from "@flow-like/flow-like-ui/state/global-chat/agent-debug-report";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { usePathname } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	type ITelemetrySettings,
	getTelemetrySettings,
	isCrashReportingEnabled,
	isUsageTelemetryEnabled,
	onTelemetrySettingsChange,
	setTelemetryEnabled,
} from "../lib/telemetry-settings";

interface IQueuedTelemetryRow {
	id: number;
}

interface IQueuedTelemetryEvent extends IQueuedTelemetryRow {
	name: string;
	props?: Record<string, unknown> | null;
	client_ts: string;
}

interface IQueuedTelemetryError extends IQueuedTelemetryRow {
	kind: string;
	value: string;
	level?: string | null;
	culprit?: string | null;
	stacktrace?: unknown;
	breadcrumbs?: unknown;
	context?: Record<string, unknown> | null;
	client_ts: string;
}

interface IBufferDrainSpec<T extends IQueuedTelemetryRow> {
	drainCommand: string;
	ackCommand: string;
	/** Rows pulled from the native buffer per tick. */
	limit: number;
	/** Rows per ingest request; must not exceed the server batch cap. */
	batchSize: number;
	path: string;
	isEnabled: () => boolean;
	buildBody: (anonId: string, items: Omit<T, "id">[]) => unknown;
	failureMessage: string;
}

const DRAIN_INTERVAL_MS = 60_000;
/**
 * Ingest caps the API rejects with a 400 when exceeded — `MAX_EVENTS_PER_BATCH`
 * in `packages/api/src/routes/telemetry.rs` and `MAX_ERRORS_PER_BATCH` in
 * `packages/api/src/routes/telemetry/errors.rs`. A rejected batch is never
 * acked, and the native drain is non-destructive, so an oversized request wedges
 * the buffer permanently: every request size below must stay at or under them.
 */
const MAX_EVENTS_PER_BATCH = 50;
const MAX_ERRORS_PER_BATCH = 20;
/** Drained rows are chunked into cap-sized requests, so a backlog can exceed
 * the cap and still drain several batches per tick. */
const EVENT_DRAIN_LIMIT = 200;
const ERROR_DRAIN_LIMIT = 60;
/** Keeps a webview resumed from sleep from being reported as a startup time. */
const MAX_APP_START_MS = 300_000;
const MAX_SCREEN_LOAD_MS = 60_000;

let globalHandlerRefCount = 0;
let removeGlobalHandlers: (() => void) | undefined;
let appStartElapsed: Promise<number | undefined> | undefined;
let appStartReported = false;

function webviewElapsedMs(): number | undefined {
	return typeof performance === "undefined" ? undefined : performance.now();
}

/**
 * Process start to first render, resolved once per process from the native
 * marker the Tauri entrypoint sets, falling back to the webview's own timeline
 * for browser dev runs. A duration carries no identity, so it is measured
 * unconditionally and only reported once usage consent is granted — which keeps
 * the number honest when consent arrives after startup.
 */
function measureAppStart(): Promise<number | undefined> {
	appStartElapsed ??= invoke<number | null>("app_start_elapsed_ms")
		.then((elapsed) =>
			typeof elapsed === "number" && Number.isFinite(elapsed)
				? elapsed
				: webviewElapsedMs(),
		)
		.catch(() => webviewElapsedMs());
	return appStartElapsed;
}

function useWebVitals(enabled: boolean) {
	useEffect(() => {
		if (!enabled) return;
		return initWebVitals();
	}, [enabled]);
}

/**
 * Reports the commit-to-paint duration of a client-side route change. The first
 * pathname is skipped because the initial screen is already covered by
 * `app_start` and the paint vitals.
 */
function useScreenLoadMetric(pathname: string | null, enabled: boolean) {
	const previous = useRef<string | undefined>(undefined);
	useEffect(() => {
		if (!pathname) return;
		const from = previous.current;
		previous.current = pathname;
		if (!enabled || from === undefined || from === pathname) return;
		if (typeof requestAnimationFrame !== "function") return;
		const start = performance.now();
		let timer: ReturnType<typeof setTimeout> | undefined;
		const frame = requestAnimationFrame(() => {
			timer = setTimeout(() => {
				const duration = performance.now() - start;
				if (duration <= MAX_SCREEN_LOAD_MS)
					capturePerfMetric("screen_load", duration, pathname);
			}, 0);
		});
		return () => {
			cancelAnimationFrame(frame);
			if (timer !== undefined) clearTimeout(timer);
		};
	}, [pathname, enabled]);
}

function useAppStartMetric(enabled: boolean) {
	useEffect(() => {
		void measureAppStart();
	}, []);

	useEffect(() => {
		if (!enabled || appStartReported) return;
		let cancelled = false;
		void measureAppStart().then((elapsed) => {
			if (cancelled || appStartReported) return;
			if (elapsed === undefined || elapsed > MAX_APP_START_MS) return;
			appStartReported = true;
			capturePerfMetric("app_start", elapsed);
		});
		return () => {
			cancelled = true;
		};
	}, [enabled]);
}

function captureUnhandled(error: unknown, culprit: string) {
	try {
		captureTelemetryError(error, { level: "fatal", culprit });
		markTelemetrySessionCrashed();
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

/**
 * Installs the unhandled error hooks once per window, no matter how many
 * providers mount, and removes them once the last one unmounts.
 */
function installGlobalErrorHandlers(): () => void {
	if (typeof window === "undefined") return () => undefined;
	globalHandlerRefCount += 1;
	if (globalHandlerRefCount === 1) {
		const onError = (event: ErrorEvent) => {
			if (event.target && event.target !== window) return;
			const error = event.error ?? event.message;
			if (isBenignBrowserError(error)) return;
			captureUnhandled(error, "window.onerror");
		};
		const onRejection = (event: PromiseRejectionEvent) =>
			captureUnhandled(event.reason, "window.onunhandledrejection");
		window.addEventListener("error", onError);
		window.addEventListener("unhandledrejection", onRejection);
		removeGlobalHandlers = () => {
			window.removeEventListener("error", onError);
			window.removeEventListener("unhandledrejection", onRejection);
		};
	}
	return () => {
		globalHandlerRefCount = Math.max(0, globalHandlerRefCount - 1);
		if (globalHandlerRefCount === 0) {
			removeGlobalHandlers?.();
			removeGlobalHandlers = undefined;
		}
	};
}

function desktopPlatform(): string {
	const ua = navigator.userAgent.toLowerCase();
	if (ua.includes("android")) return "android";
	if (/ipad|iphone|ipod/.test(ua)) return "ios";
	if (ua.includes("mac")) return "macos";
	if (ua.includes("win")) return "windows";
	if (ua.includes("linux")) return "linux";
	return "desktop";
}

export function TelemetryProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const features = useFeatures();
	const pathname = usePathname();
	const [settings, setSettings] = useState<ITelemetrySettings | undefined>();

	const settingsRef = useRef<ITelemetrySettings | undefined>(undefined);
	settingsRef.current = settings;

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
	);
	const profileRef = useRef(profile.data);
	profileRef.current = profile.data;

	const available = features.data?.telemetry === true;
	const availableRef = useRef(available);
	availableRef.current = available;

	const usageEnabled = useCallback(
		() => availableRef.current && isUsageTelemetryEnabled(settingsRef.current),
		[],
	);
	const crashEnabled = useCallback(
		() => availableRef.current && isCrashReportingEnabled(settingsRef.current),
		[],
	);

	useEffect(() => {
		getTelemetrySettings()
			.then(setSettings)
			.catch((error) =>
				console.warn("Failed to load telemetry settings:", error),
			);
		return onTelemetrySettingsChange(setSettings);
	}, []);

	const settingsLoaded = settings !== undefined;
	// This query is explicitly excluded from persistence, so the first value is
	// authoritative for this process. Do not tear down telemetry during later
	// background refetches merely because `isFetching` becomes true again.
	const featuresResolved = features.data !== undefined;

	useEffect(() => {
		// Keep early crash reports in the telemetry module's pending buffer until
		// the backend feature gate is known. Attaching a disabled client earlier
		// would consume and permanently drop those startup reports.
		if (!settingsLoaded || !featuresResolved) return;
		let cancelled = false;
		let client: ITelemetryClient | undefined;
		let removeEventSink: (() => void) | undefined;
		let removeErrorSink: (() => void) | undefined;
		let removeSessionSink: (() => void) | undefined;
		let removePerfSink: (() => void) | undefined;
		let removeSpanSink: (() => void) | undefined;
		let removeMetricsSink: (() => void) | undefined;
		let removeSettingsListener: (() => void) | undefined;
		let drainTimer: ReturnType<typeof setInterval> | undefined;

		(async () => {
			const appVersion = await getVersion().catch(() => undefined);
			if (cancelled) return;
			const platform = desktopPlatform();

			/**
			 * Uploads the native buffer in server-cap-sized batches, acking each
			 * batch only once its request succeeded. A failure leaves the
			 * remaining rows buffered for the next tick instead of replaying an
			 * already delivered batch.
			 */
			const drainBuffer = async <T extends IQueuedTelemetryRow>(
				spec: IBufferDrainSpec<T>,
			) => {
				const anonId = settingsRef.current?.anonId;
				const currentProfile = profileRef.current;
				if (!spec.isEnabled() || !anonId || !currentProfile) return;
				try {
					const rows = await invoke<T[]>(spec.drainCommand, {
						limit: spec.limit,
					});
					for (
						let offset = 0;
						offset < rows.length && !cancelled;
						offset += spec.batchSize
					) {
						const batch = rows.slice(offset, offset + spec.batchSize);
						await backend.apiState.post(
							currentProfile,
							spec.path,
							spec.buildBody(
								anonId,
								batch.map(({ id, ...row }) => row),
							),
						);
						await invoke(spec.ackCommand, {
							ids: batch.map((row) => row.id),
						});
					}
				} catch (error) {
					console.warn(spec.failureMessage, error);
				}
			};

			const drain = async () => {
				await drainBuffer<IQueuedTelemetryEvent>({
					drainCommand: "drain_telemetry_events",
					ackCommand: "ack_telemetry_events",
					limit: EVENT_DRAIN_LIMIT,
					batchSize: MAX_EVENTS_PER_BATCH,
					path: "telemetry/events",
					isEnabled: usageEnabled,
					buildBody: (anonId, events) => ({
						anon_id: anonId,
						source: "desktop_core",
						app_version: appVersion ?? null,
						platform,
						events,
					}),
					failureMessage: t('failedToDeliverBufferedTelemetryEvents', 'Failed to deliver buffered telemetry events:'),
				});
				await drainBuffer<IQueuedTelemetryError>({
					drainCommand: "drain_telemetry_errors",
					ackCommand: "ack_telemetry_errors",
					limit: ERROR_DRAIN_LIMIT,
					batchSize: MAX_ERRORS_PER_BATCH,
					path: "telemetry/errors",
					isEnabled: crashEnabled,
					buildBody: (anonId, errors) => ({
						anon_id: anonId,
						source: "desktop_native",
						app_version: appVersion ?? null,
						release: appVersion ?? null,
						platform,
						errors,
					}),
					failureMessage: t('failedToDeliverBufferedCrashReports', 'Failed to deliver buffered crash reports:'),
				});
			};

			const beacon = (body: unknown, path: string) => {
				if (
					typeof navigator === "undefined" ||
					typeof navigator.sendBeacon !== "function"
				)
					return false;
				return navigator.sendBeacon(
					getApiUrl(profileRef.current, path),
					new Blob([JSON.stringify(body)], { type: "application/json" }),
				);
			};

			client = createTelemetryClient({
				apiState: backend.apiState,
				getProfile: () => profileRef.current,
				isEnabled: usageEnabled,
				isCrashEnabled: crashEnabled,
				getAnonId: () => settingsRef.current?.anonId ?? undefined,
				source: "desktop",
				appVersion,
				release: appVersion,
				platform,
				beacon: (body) => beacon(body, "telemetry/events"),
				crashBeacon: beacon,
				usageBeacon: beacon,
			});
			void initTelemetrySampling(
				createTelemetrySamplingFetcher(
					backend.apiState,
					() => profileRef.current,
				),
			);
			removeEventSink = setTelemetryEventSink(client.sink);
			removeErrorSink = setTelemetryErrorSink(client.errorSink);
			removeSessionSink = setTelemetrySessionSink(client.sessionSink);
			removePerfSink = setTelemetryPerfSink(client.perfSink);
			removeSpanSink = setTelemetrySpanSink(client.spanSink);
			removeMetricsSink = setFlowPilotProductionMetricsSink((metrics) => {
				captureTelemetryEvent("flowpilot_generation_metrics", { ...metrics });
			});
			removeSettingsListener = onTelemetrySettingsChange((next) => {
				if (!isUsageTelemetryEnabled(next)) client?.clear();
				if (!isCrashReportingEnabled(next)) client?.clearCrashReports();
			});
			await drain();
			if (cancelled) return;
			drainTimer = setInterval(() => void drain(), DRAIN_INTERVAL_MS);
		})();

		return () => {
			cancelled = true;
			if (drainTimer) clearInterval(drainTimer);
			removeSettingsListener?.();
			removeEventSink?.();
			removeErrorSink?.();
			removeSessionSink?.();
			removePerfSink?.();
			removeSpanSink?.();
			removeMetricsSink?.();
			client?.dispose();
		};
	}, [backend, settingsLoaded, featuresResolved, usageEnabled, crashEnabled]);

	const crashReportingActive = available && isCrashReportingEnabled(settings);
	const usageActive = available && isUsageTelemetryEnabled(settings);
	const sessionActive =
		available &&
		(isCrashReportingEnabled(settings) || isUsageTelemetryEnabled(settings));

	useWebVitals(usageActive);
	useScreenLoadMetric(pathname, usageActive);
	useAppStartMetric(usageActive);

	useEffect(() => {
		if (!crashReportingActive) return;
		return installGlobalErrorHandlers();
	}, [crashReportingActive]);

	useEffect(() => {
		if (!sessionActive) return;
		startTelemetrySession();
		return () => endTelemetrySession();
	}, [sessionActive]);

	useEffect(() => {
		if (!pathname) return;
		// Spans never span a navigation; anything still open would otherwise
		// parent the next screen's spans into the previous trace.
		clearActiveTelemetrySpans();
		addTelemetryBreadcrumb({
			category: "navigation",
			message: sanitizeTelemetryPath(pathname),
		});
		capturePageView(pathname);
	}, [pathname]);

	const handleDecision = useCallback(async (enabled: boolean) => {
		try {
			await setTelemetryEnabled(enabled);
		} catch (error) {
			console.warn("Failed to update telemetry consent:", error);
		}
	}, []);

	const showConsentPrompt =
		settings != null && settings.enabled == null && available;

	return (
		<>
			{children}
			{showConsentPrompt && (
				<TelemetryConsentPrompt
					onDecision={handleDecision}
					privacyHref="/settings/privacy"
				/>
			)}
		</>
	);
}
