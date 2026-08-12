"use client";

import {
	type TelemetryConsent,
	getCrashReportsEnabled,
	getTelemetryAnonId,
	getTelemetryConsent,
	onTelemetryConsentChange,
	setTelemetryConsent,
} from "@/lib/telemetry-consent";
import {
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
import { usePathname } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";

const MAX_SCREEN_LOAD_MS = 60_000;

let globalHandlerRefCount = 0;
let removeGlobalHandlers: (() => void) | undefined;

function captureUnhandled(error: unknown, culprit: string) {
	try {
		captureTelemetryError(error, { level: "fatal", culprit });
		markTelemetrySessionCrashed();
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

/**
 * Installs the unhandled error hooks once per document, no matter how many
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

function useWebVitals(enabled: boolean) {
	useEffect(() => {
		if (!enabled) return;
		return initWebVitals();
	}, [enabled]);
}

/**
 * Reports the commit-to-paint duration of a client-side route change. The first
 * pathname is skipped because the initial page load is already covered by the
 * paint and navigation vitals.
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

export function TelemetryProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const backend = useBackend();
	const features = useFeatures();
	const pathname = usePathname();
	const [consent, setConsent] = useState<TelemetryConsent>(undefined);
	const [crashReports, setCrashReports] = useState(false);
	const [hydrated, setHydrated] = useState(false);

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
		() => availableRef.current && getTelemetryConsent() === "granted",
		[],
	);
	const crashEnabled = useCallback(
		() => availableRef.current && getCrashReportsEnabled(),
		[],
	);

	useEffect(() => {
		const sync = () => {
			setConsent(getTelemetryConsent());
			setCrashReports(getCrashReportsEnabled());
		};
		sync();
		setHydrated(true);
		return onTelemetryConsentChange(sync);
	}, []);

	useEffect(() => {
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

		const client = createTelemetryClient({
			apiState: backend.apiState,
			getProfile: () => profileRef.current,
			isEnabled: usageEnabled,
			isCrashEnabled: crashEnabled,
			getAnonId: () => getTelemetryAnonId(),
			source: "web",
			platform: "web",
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
		const removeEventSink = setTelemetryEventSink(client.sink);
		const removeErrorSink = setTelemetryErrorSink(client.errorSink);
		const removeSessionSink = setTelemetrySessionSink(client.sessionSink);
		const removePerfSink = setTelemetryPerfSink(client.perfSink);
		const removeSpanSink = setTelemetrySpanSink(client.spanSink);
		const removeMetricsSink = setFlowPilotProductionMetricsSink((metrics) => {
			captureTelemetryEvent("flowpilot_generation_metrics", { ...metrics });
		});
		const removeConsentListener = onTelemetryConsentChange(() => {
			if (!usageEnabled()) client.clear();
			if (!crashEnabled()) client.clearCrashReports();
		});
		return () => {
			removeConsentListener();
			removeEventSink();
			removeErrorSink();
			removeSessionSink();
			removePerfSink();
			removeSpanSink();
			removeMetricsSink();
			client.dispose();
		};
	}, [backend, usageEnabled, crashEnabled]);

	const crashReportingActive = available && crashReports;
	const usageActive = available && consent === "granted" && hydrated;
	const sessionActive =
		available && (crashReports || consent === "granted") && hydrated;

	useWebVitals(usageActive);
	useScreenLoadMetric(pathname, usageActive);

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

	const handleDecision = useCallback((enabled: boolean) => {
		setTelemetryConsent(enabled);
	}, []);

	const showConsentPrompt = hydrated && consent === undefined && available;

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
