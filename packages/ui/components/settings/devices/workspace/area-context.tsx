"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useDeveloperMode } from "../../../../hooks/use-developer-mode";
import { toHubError } from "../../../../lib/device-management/hub/endpoints";
import {
	HUB_CADENCE,
	pollInterval,
} from "../../../../lib/device-management/hub/queries";
import type { HubErrorCode } from "../../../../lib/device-management/model/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import {
	AreaNowContext,
	type AreaPrefs,
	AreaPrefsContext,
	type DevicesT,
	HubFreshnessContext,
	type HubFreshnessState,
} from "../primitives/area-context";
import { useDeviceWorkspace } from "./device-workspace-provider";
import { type AttentionState, useAttentionState } from "./use-attention";

export type WidthBucket = "phone" | "narrow" | "medium" | "wide";

/** SPEC §3.8 breakpoints of the area's own width (never the window's). */
export function widthBucketOf(width: number): WidthBucket {
	if (width < 720) return "phone";
	if (width < 900) return "narrow";
	if (width < 1280) return "medium";
	return "wide";
}

const WidthContext = createContext<WidthBucket>("wide");
const AreaRootContext = createContext<(element: HTMLElement | null) => void>(
	() => undefined,
);

const HUB_ERROR_COPY: Record<HubErrorCode, (t: DevicesT) => string> = {
	network: (t) =>
		t("devices:action.hubError.network", "The hub can't be reached."),
	timeout: (t) =>
		t("devices:action.hubError.timeout", "The hub took too long to answer."),
	unauthorized: (t) =>
		t("devices:action.hubError.unauthorized", "Your sign-in has expired."),
	forbidden: (t) =>
		t("devices:action.hubError.forbidden", "The hub refused this request."),
	token_restricted: (t) =>
		t(
			"devices:action.hubError.tokenRestricted",
			"Your access token can't manage devices.",
		),
	not_found: (t) =>
		t("devices:action.hubError.notFound", "The hub doesn't know this."),
	rate_limited: (t) =>
		t(
			"devices:action.hubError.rateLimited",
			"The hub is limiting requests. It retries by itself.",
		),
	server_error: (t) =>
		t("devices:action.hubError.serverError", "The hub reported an error."),
	invalid_response: (t) =>
		t(
			"devices:action.hubError.invalidResponse",
			"The hub sent an answer this app can't read.",
		),
};

/** One sentence for a failed hub read (stamp hover, gates, inline results). */
export function hubErrorCopy(t: DevicesT, code: HubErrorCode): string {
	return HUB_ERROR_COPY[code](t);
}

/** One hub-corrected tick per area; it pauses while the page is hidden. */
function useAreaClock(workspace: DeviceWorkspace, tickMs: number | false) {
	const [now, setNow] = useState(() => workspace.clock.now());
	useEffect(() => {
		const read = () => setNow(workspace.clock.now());
		read();
		if (tickMs === false) return;
		let timer: ReturnType<typeof setInterval> | undefined;
		const sync = () => {
			const hidden = globalThis.document?.visibilityState === "hidden";
			if (hidden && timer !== undefined) {
				clearInterval(timer);
				timer = undefined;
			} else if (!hidden && timer === undefined) {
				read();
				timer = setInterval(read, tickMs);
			}
		};
		sync();
		globalThis.document?.addEventListener("visibilitychange", sync);
		return () => {
			if (timer !== undefined) clearInterval(timer);
			globalThis.document?.removeEventListener("visibilitychange", sync);
		};
	}, [workspace, tickMs]);
	return now;
}

const toHubSeconds = (workspace: DeviceWorkspace, localMs: number) =>
	Math.floor((localMs - (workspace.clock.hubOffsetS ?? 0) * 1000) / 1000);

/**
 * R5: while `GET /devices` fails after a good read, every Hub stamp says so.
 * A hub that is off, unreachable or still being checked refuses the list as
 * part of that state; its gate and the Hub status screen name it, so it is
 * not a failed refresh.
 */
function hubFreshnessOf(
	{ rows, workspace, input }: AttentionState,
	t: DevicesT,
): HubFreshnessState {
	if (!rows.error || input.hub.state !== "on") return { failing: false };
	const error = toHubError(rows.error);
	const retryInMs = pollInterval(HUB_CADENCE.list.intervalMs || 30_000, {
		data: rows.data,
		error: rows.error,
		dataUpdateCount: rows.data === undefined ? 0 : 1,
		errorUpdateCount: rows.errorUpdateCount,
		dataUpdatedAt: rows.dataUpdatedAt,
		errorUpdatedAt: rows.errorUpdatedAt,
	});
	return {
		failing: true,
		...(rows.dataUpdatedAt
			? { dataFrom: toHubSeconds(workspace, rows.dataUpdatedAt) }
			: {}),
		...(rows.errorUpdatedAt
			? { retryAt: toHubSeconds(workspace, rows.errorUpdatedAt + retryInMs) }
			: {}),
		reason: hubErrorCopy(t, error.code),
		onRetry: () => void rows.refetch(),
	};
}

/**
 * Fills the three contexts the primitives read (area clock, hub freshness,
 * prefs) and measures the area for `useWidthBucket()`. Needs a
 * `DeviceWorkspaceProvider` above it.
 */
export function AreaProvider({
	children,
	widthBucket,
	tickMs = 1000,
}: Readonly<{
	children: ReactNode;
	/** Fixes the bucket instead of measuring (tests, previews). */
	widthBucket?: WidthBucket;
	/** `false` stops the clock (tests). */
	tickMs?: number | false;
}>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const attention = useAttentionState();
	const { developerMode } = useDeveloperMode();
	const now = useAreaClock(workspace, tickMs);
	const [measured, setMeasured] = useState<WidthBucket>("wide");
	const observer = useRef<ResizeObserver | null>(null);

	const setRoot = useCallback((element: HTMLElement | null) => {
		observer.current?.disconnect();
		observer.current = null;
		if (!element || typeof ResizeObserver === "undefined") return;
		const measure = (width: number) => {
			if (width > 0) setMeasured(widthBucketOf(width));
		};
		measure(element.getBoundingClientRect().width);
		const next = new ResizeObserver((entries) => {
			const width = entries.at(-1)?.contentRect.width;
			if (width !== undefined) measure(width);
		});
		next.observe(element);
		observer.current = next;
	}, []);
	useEffect(() => () => observer.current?.disconnect(), []);

	const hubFreshness = useMemo(
		() => hubFreshnessOf(attention, t),
		[attention, t],
	);
	const prefs = useMemo<AreaPrefs>(
		() => ({ showTechnicalKeys: developerMode }),
		[developerMode],
	);

	return (
		<AreaRootContext.Provider value={setRoot}>
			<WidthContext.Provider value={widthBucket ?? measured}>
				<AreaPrefsContext.Provider value={prefs}>
					<HubFreshnessContext.Provider value={hubFreshness}>
						<AreaNowContext.Provider value={now}>
							{children}
						</AreaNowContext.Provider>
					</HubFreshnessContext.Provider>
				</AreaPrefsContext.Provider>
			</WidthContext.Provider>
		</AreaRootContext.Provider>
	);
}

/** Ref callback for the area's root element; its width drives `useWidthBucket()`. */
export function useAreaRootRef(): (element: HTMLElement | null) => void {
	return useContext(AreaRootContext);
}

/** `phone` < 720 · `narrow` < 900 · `medium` < 1280 · `wide`; `wide` until the area is measured. */
export function useWidthBucket(): WidthBucket {
	return useContext(WidthContext);
}
