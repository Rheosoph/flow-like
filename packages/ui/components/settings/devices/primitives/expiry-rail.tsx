"use client";

import { useTranslation } from "@flow-like/locales";
import { useAreaTime } from "./area-context";
import { cx } from "./tone";

const DAY = 86_400;
const PAST_DAYS = 14;
const FUTURE_DAYS = 90;
const WIDTH = 120;
const TODAY_X = (PAST_DAYS / (PAST_DAYS + FUTURE_DAYS)) * WIDTH;
/** The largest time value a Date can hold; a reported expiry beyond it (or NaN) can't be drawn or formatted. */
const MAX_DATE_MS = 8.64e15;

export type ExpiryState = "expired" | "expiring" | "valid";

export function expiryState(notAfter: number, nowS: number): ExpiryState {
	const days = (notAfter - nowS) / DAY;
	return days < 0 ? "expired" : days <= 7 ? "expiring" : "valid";
}

const MARK_FILL: Record<ExpiryState, string> = {
	expired: "fill-critical-solid",
	expiring: "fill-warning-solid",
	valid: "fill-good-solid",
};

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

function dayMonth(locale: string, atS: number, nowS: number): string {
	const withYear =
		new Date(atS * 1000).getFullYear() !== new Date(nowS * 1000).getFullYear();
	const key = `${locale}|${withYear}`;
	let formatter = dayFormatters.get(key);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat(locale, {
			day: "numeric",
			month: "short",
			year: withYear ? "numeric" : undefined,
		});
		dayFormatters.set(key, formatter);
	}
	return formatter.format(atS * 1000);
}

/**
 * SPEC §4.27: −14 d … +90 d at 120 px with a "today" line at the same x in
 * every row, so the marks of a column compare at a glance.
 */
export function ExpiryRail({
	notAfter,
	className,
}: Readonly<{
	/** Unix seconds. */
	notAfter: number;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!Number.isFinite(notAfter) || Math.abs(notAfter * 1000) > MAX_DATE_MS)
		return null;
	const state = expiryState(notAfter, time.nowS);
	const days = (notAfter - time.nowS) / DAY;
	const x = Math.max(
		0,
		Math.min(WIDTH, ((days + PAST_DAYS) / (PAST_DAYS + FUTURE_DAYS)) * WIDTH),
	);
	const params = {
		date: dayMonth(time.locale, notAfter, time.nowS),
		ago: time.ago(notAfter, "long"),
	};
	const label =
		state === "expired"
			? t("view.xrail.expired", "Expired {{date}}, {{ago}}", params)
			: t("view.xrail.expires", "Expires {{date}}, {{ago}}", params);
	return (
		<span
			role="img"
			aria-label={label}
			title={t(
				"view.xrail.title",
				"{{label}} · scale: 14 days ago to 90 days ahead",
				{
					label,
				},
			)}
			data-xrail={state}
			className={cx("inline-flex shrink-0 align-middle", className)}
		>
			<svg
				viewBox={`0 0 ${WIDTH} 14`}
				width={WIDTH}
				height="14"
				aria-hidden="true"
				className="block overflow-visible"
			>
				<rect
					className="fill-hairline"
					x="0"
					y="6"
					width={WIDTH}
					height="2"
					rx="1"
				/>
				<rect
					className="fill-foreground"
					x={TODAY_X - 0.5}
					y="1"
					width="1"
					height="12"
				/>
				<path
					data-mark={days > FUTURE_DAYS ? "beyond" : "at"}
					className={cx("stroke-card", MARK_FILL[state])}
					strokeWidth="1"
					d={
						days > FUTURE_DAYS
							? `M${WIDTH - 7} 3 ${WIDTH} 7 ${WIDTH - 7} 11Z`
							: `M${x.toFixed(2)} 3.2 ${(x + 3.8).toFixed(2)} 7 ${x.toFixed(2)} 10.8 ${(x - 3.8).toFixed(2)} 7Z`
					}
				/>
			</svg>
		</span>
	);
}

/** The column header scale ("−14 d · today · +90 d"), aligned to the rail's today line. */
export function ExpiryRailScale({
	className,
}: Readonly<{ className?: string }>) {
	const { t } = useTranslation("devices");
	return (
		<span
			aria-hidden
			title={t("view.xrail.scale", "Scale: 14 days ago to 90 days ahead")}
			className={cx(
				"relative inline-block h-7 w-30 align-middle font-mono text-[11px] leading-3.5 font-normal tracking-normal text-muted-foreground normal-case",
				className,
			)}
		>
			<span
				className="absolute top-0 border-l border-foreground pl-0.75 whitespace-nowrap"
				style={{ left: `${TODAY_X - 0.5}px` }}
			>
				{t("view.xrail.today", "today")}
			</span>
			<span className="absolute bottom-0 left-0 whitespace-nowrap">
				{t("view.xrail.past", "−14 d")}
			</span>
			<span className="absolute right-0 bottom-0 whitespace-nowrap">
				{t("view.xrail.future", "+90 d")}
			</span>
		</span>
	);
}
