"use client";

import { type AreaTime, useAreaTime } from "./area-context";

const dayFormatters = new Map<string, Intl.DateTimeFormat>();
const untilFormatters = new Map<string, Intl.RelativeTimeFormat>();

const HOUR = 3600;
const DAY = 86_400;

/** "5 Oct" (the year only when it differs from now). */
export function dayText(
	time: Pick<AreaTime, "locale" | "now">,
	atS: number,
): string {
	const date = new Date(atS * 1000);
	const withYear = date.getFullYear() !== new Date(time.now).getFullYear();
	const key = `${time.locale}|${withYear}`;
	let formatter = dayFormatters.get(key);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat(time.locale, {
			day: "numeric",
			month: "short",
			year: withYear ? "numeric" : undefined,
		});
		dayFormatters.set(key, formatter);
	}
	return formatter.format(date);
}

/** A day with the full moment on hover (R16). */
export function DayOf({
	at,
	className,
}: Readonly<{ at: number; className?: string }>) {
	const time = useAreaTime();
	return (
		<time
			dateTime={new Date(at * 1000).toISOString()}
			title={time.abs(at)}
			className={className}
		>
			{dayText(time, at)}
		</time>
	);
}

/**
 * "in 14h", "in 31d", "2d ago": the distance to an end as a count (hours under
 * two days, days beyond). `time.ago` rounds the same instants to "tomorrow"
 * and "next mo.", which hides how long access or a certificate still lasts.
 */
export function untilText(
	time: Pick<AreaTime, "locale" | "nowS" | "ago">,
	atS: number,
): string {
	const seconds = atS - time.nowS;
	if (Math.abs(seconds) < HOUR) return time.ago(atS);
	let formatter = untilFormatters.get(time.locale);
	if (!formatter) {
		formatter = new Intl.RelativeTimeFormat(time.locale, {
			numeric: "always",
			style: "narrow",
		});
		untilFormatters.set(time.locale, formatter);
	}
	return Math.abs(seconds) < 2 * DAY
		? formatter.format(Math.round(seconds / HOUR), "hour")
		: formatter.format(Math.round(seconds / DAY), "day");
}
