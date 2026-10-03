"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { createContext, useContext, useMemo } from "react";
import {
	formatAbsoluteDateTimeZoned,
	formatCountdown,
	formatMoment,
	formatRelativeTime,
	formatTimeOfDay,
} from "../../../../lib/date";

/*
 * The only contexts primitives may read (M-UI §2.4). W2's AreaProvider fills
 * them; without a provider every hook falls back to a safe default so a
 * primitive renders on its own (gallery tests, overlays outside the area).
 */

/** `t` from `useTranslation("devices")`. Module-level helpers taking it use `devices:`-prefixed keys. */
export type DevicesT = TFunction<"devices">;

/** Area clock in epoch milliseconds, hub-corrected, ticking once per second. `null` = no provider. */
export const AreaNowContext = createContext<number | null>(null);

export function useAreaNow(): number {
	return useContext(AreaNowContext) ?? Date.now();
}

export interface HubFreshnessState {
	/** True while hub refreshes fail: every Hub stamp switches to its error variant (R5). */
	failing: boolean;
	/** Unix seconds of the last successful hub read. */
	dataFrom?: number;
	/** Unix seconds of the next automatic retry. */
	retryAt?: number;
	/** Translated one-line cause, shown in the stamp's hover. */
	reason?: string;
	onRetry?: () => void;
}

export const HubFreshnessContext = createContext<HubFreshnessState>({
	failing: false,
});

export function useHubFreshness(): HubFreshnessState {
	return useContext(HubFreshnessContext);
}

export interface AreaPrefs {
	/** Developer mode: render `.tech` spans with raw keys and codes (R3). */
	showTechnicalKeys: boolean;
}

export const AreaPrefsContext = createContext<AreaPrefs>({
	showTechnicalKeys: false,
});

export function useAreaPrefs(): AreaPrefs {
	return useContext(AreaPrefsContext);
}

export interface AreaTime {
	/** Epoch milliseconds. */
	now: number;
	/** Unix seconds. */
	nowS: number;
	locale: string;
	/** "13s ago" / "in 4m" for a unix-seconds instant. */
	ago(atS: number, style?: Intl.RelativeTimeFormatStyle): string;
	/** "13:59:58". */
	clock(atS: number, seconds?: boolean): string;
	/** "30 Sept, 13:59:47 CEST" (year only when it differs from now). */
	abs(atS: number): string;
	/** "11:00" today, "29 Sept, 11:00" on another day: for "since …" phrases. */
	at(atS: number): string;
	/** "0:27" until a unix-seconds instant. */
	countdown(untilS: number): string;
}

/** One formatter set per render, bound to the area clock and the active i18n language. */
export function useAreaTime(): AreaTime {
	const now = useAreaNow();
	const { i18n } = useTranslation("devices");
	const locale = i18n?.language ?? "en";
	return useMemo(
		() => ({
			now,
			nowS: now / 1000,
			locale,
			ago: (atS, style = "narrow") =>
				formatRelativeTime(atS * 1000, style, "", { now, locale }),
			clock: (atS, seconds = true) =>
				formatTimeOfDay(atS * 1000, { locale, seconds }),
			abs: (atS) => formatAbsoluteDateTimeZoned(atS * 1000, { now, locale }),
			at: (atS) => formatMoment(atS * 1000, { now, locale }),
			countdown: (untilS) => formatCountdown(untilS - now / 1000),
		}),
		[now, locale],
	);
}
