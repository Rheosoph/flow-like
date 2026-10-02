"use client";

import type { ReactNode } from "react";
import type {
	DeviceTab,
	Presence,
} from "../../../../lib/device-management/model/types";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { CellSub } from "../primitives/dv-table";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { type CertificateRow, DAY_S } from "./certificates-model";

/**
 * The app's base layer gives every `table` a margin and every cell a full
 * border; inside a block only the row hairlines of the table primitive remain.
 * Chips in cells get 4 px corners and may wrap.
 */
export const TABLE_RESET =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0 [&_td_[data-slot=badge]]:h-auto [&_td_[data-slot=badge]]:min-h-5.5 [&_td_[data-slot=badge]]:rounded-md [&_td_[data-slot=badge]]:py-0.5 [&_td_[data-slot=badge]]:whitespace-normal [&_td_[data-slot=badge]>span]:whitespace-normal";

export const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
export const LINK_BUTTON = cx(LINK, "cursor-pointer text-left text-xs");
/** A device or service named in a cell or a sentence: mono, underlined on hover only. */
export const OBJECT_LINK =
	"font-mono font-semibold text-foreground no-underline hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
export const PROSE = "max-w-[88ch] text-ui text-ink-2";
export const LABEL =
	"text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground";

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

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

/** "in 5d", "2d ago": a distance as a count; the area's relative times round to "tomorrow" and "next mo.". */
export function untilText(time: AreaTime, atS: number): string {
	const seconds = atS - time.nowS;
	if (Math.abs(seconds) < 3600) return time.ago(atS);
	const format = new Intl.RelativeTimeFormat(time.locale, {
		numeric: "always",
		style: "narrow",
	});
	return Math.abs(seconds) < 48 * 3600
		? format.format(Math.round(seconds / 3600), "hour")
		: format.format(Math.round(seconds / DAY_S), "day");
}

/** "5 d", "18 h": a remaining lifetime inside a chip. */
export function spanText(t: DevicesT, seconds: number): string {
	const span = Math.abs(seconds);
	if (span >= 2 * DAY_S)
		return t("devices:certificates.span.days", "{{count, number}} d", {
			count: Math.floor(span / DAY_S),
		});
	if (span >= 3600)
		return t("devices:certificates.span.hours", "{{count, number}} h", {
			count: Math.floor(span / 3600),
		});
	return t("devices:certificates.span.minutes", "{{count, number}} min", {
		count: Math.max(1, Math.floor(span / 60)),
	});
}

/** "5 days": whole days, for sentences. */
export function daysText(t: DevicesT, days: number): string {
	return t("devices:certificates.span.wholeDays", {
		count: Math.max(0, Math.round(days)),
		defaultValue_one: "{{count, number}} day",
		defaultValue_other: "{{count, number}} days",
	});
}

export const shortId = (id: string) => id.slice(0, 8);

/** The name a live read gave the certificate, else the first block of its ID. */
export const certificateTitle = (
	row: Pick<CertificateRow, "detail" | "certificateId">,
) => row.detail?.label ?? shortId(row.certificateId);

export interface DeviceOption {
	id: string;
	name: string;
}

const EVERY_DEVICE = "all";

/** A compact device chooser for a block toolbar; `allLabel` adds the entry for every device (`null`). */
export function DeviceSelect({
	label,
	devices,
	value,
	onChange,
	allLabel,
}: Readonly<{
	label: string;
	devices: readonly DeviceOption[];
	value: string | null;
	onChange(deviceId: string | null): void;
	allLabel?: string;
}>) {
	return (
		<Select
			value={value ?? EVERY_DEVICE}
			onValueChange={(next) => onChange(next === EVERY_DEVICE ? null : next)}
		>
			<SelectTrigger
				aria-label={label}
				className={cx(
					"h-7 w-auto max-w-60 gap-1.5 rounded-lg border-input bg-card px-2.5 text-[13px]/[18px] shadow-none hover:border-border-strong focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-ring data-[size=default]:h-7 dark:bg-card",
					value !== null && "font-mono",
				)}
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{allLabel ? (
					<SelectItem
						value={EVERY_DEVICE}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{allLabel}
					</SelectItem>
				) : null}
				{devices.map((device) => (
					<SelectItem
						key={device.id}
						value={device.id}
						className="font-mono focus:bg-row-hover focus:text-foreground"
					>
						{device.name}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

export function DeviceLink({
	deviceId,
	name,
	tab = "certificates",
	className,
}: Readonly<{
	deviceId: string;
	name: string;
	tab?: DeviceTab;
	className?: string;
}>) {
	const link = useRouteLink();
	return (
		<a
			{...link({ screen: "device", deviceId, tab })}
			title={name}
			className={cx(OBJECT_LINK, className)}
		>
			{name}
		</a>
	);
}

/** Presence glyph + device name, with the row's own stamp underneath. */
export function DeviceCell({
	deviceId,
	name,
	presence,
	tab,
	children,
}: Readonly<{
	deviceId: string;
	name: string;
	presence: Presence;
	tab?: DeviceTab;
	children?: ReactNode;
}>) {
	return (
		<>
			<span className="flex min-w-0 items-center gap-1.5">
				<PresenceGlyph kind={presence.kind} />
				<DeviceLink
					deviceId={deviceId}
					name={name}
					tab={tab}
					className="block min-w-0 truncate"
				/>
			</span>
			{children ? <CellSub>{children}</CellSub> : null}
		</>
	);
}
