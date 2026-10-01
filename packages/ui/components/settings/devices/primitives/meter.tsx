"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import { cx } from "./tone";

export type MeterTone =
	| "neutral"
	| "good"
	| "warning"
	| "critical"
	| "reserved"
	| "unknown"
	| "locked";

const METER_FILL: Record<MeterTone, string> = {
	neutral: "bg-ink-2",
	good: "bg-good-solid",
	warning: "bg-warning-solid",
	critical: "bg-critical-solid",
	reserved: "bg-info-solid",
	unknown: "bg-unknown-solid",
	locked: "bg-locked-solid",
};

export interface MeterSegment {
	/** Share of the track in percent (0–100). */
	value: number;
	tone?: MeterTone;
}

const clampPercent = (value: number) =>
	Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

/** SPEC §4.37 `.meter`: a 6 px track with flat segments, never colour alone (the caption states the numbers). */
export function Meter({
	segments,
	label,
	caption,
	className,
}: Readonly<{
	segments: readonly MeterSegment[];
	/** Accessible summary ("2 live, 1 snapshot, 2 locked"). */
	label: string;
	caption?: ReactNode;
	className?: string;
}>) {
	return (
		<span className={cx("flex min-w-10 flex-col", className)}>
			<span
				role="img"
				aria-label={label}
				data-meter=""
				className="flex h-1.5 min-w-10 gap-px overflow-hidden rounded-[3px] bg-muted"
			>
				{segments.map((segment, index) => (
					<i
						// biome-ignore lint/suspicious/noArrayIndexKey: segments are positional
						key={index}
						data-tone={segment.tone ?? "neutral"}
						className={cx(
							"block h-full shrink-0",
							METER_FILL[segment.tone ?? "neutral"],
						)}
						style={{ width: `${clampPercent(segment.value)}%` }}
					/>
				))}
			</span>
			{caption ? (
				<span className="mt-1 text-xs tabular-nums text-muted-foreground">
					{caption}
				</span>
			) : null}
		</span>
	);
}

const moneyFormatters = new Map<string, Intl.NumberFormat>();

/** One formatter per locale and currency. A code the runtime rejects (it comes from the hub) falls back to plain amounts. */
function moneyFormatter(locale: string, currency: string): Intl.NumberFormat {
	const key = `${locale}|${currency}`;
	let formatter = moneyFormatters.get(key);
	if (!formatter) {
		try {
			formatter = new Intl.NumberFormat(locale, {
				style: "currency",
				currency,
			});
		} catch {
			formatter = new Intl.NumberFormat(locale, {
				minimumFractionDigits: 2,
				maximumFractionDigits: 2,
			});
		}
		moneyFormatters.set(key, formatter);
	}
	return formatter;
}

/** Spending: used + reserved against the limit, amounts in currency units. */
export function SpendMeter({
	used,
	reserved,
	limit,
	currency = "EUR",
	className,
}: Readonly<{
	used: number;
	reserved: number;
	limit: number;
	currency?: string;
	className?: string;
}>) {
	const { t, i18n } = useTranslation("devices");
	const money = moneyFormatter(i18n?.language ?? "en", currency);
	const share = (value: number) => (limit > 0 ? (value / limit) * 100 : 0);
	const text = t(
		"view.meter.spend",
		"{{used}} used · {{reserved}} reserved · {{limit}} limit",
		{
			used: money.format(used),
			reserved: money.format(reserved),
			limit: money.format(limit),
		},
	);
	return (
		<Meter
			label={text}
			caption={text}
			className={className}
			segments={[
				{ value: share(used), tone: "neutral" },
				{ value: share(reserved), tone: "reserved" },
			]}
		/>
	);
}

export type ProgressTone = "info" | "good" | "warning" | "critical" | "unknown";

const PROGRESS_FILL: Record<ProgressTone | "waiting", string> = {
	info: "bg-info-solid",
	good: "bg-good-solid",
	warning: "bg-warning-solid",
	critical: "bg-critical-solid",
	unknown: "bg-unknown-solid",
	waiting: "bg-info-line",
};

const INDETERMINATE =
	"absolute inset-y-0 left-0 w-[35%] animate-[indeterminate_1.3s_ease-in-out_infinite] motion-reduce:animate-none";

/** 4 px status bar (tray items, uploads). Never `ui/progress`: its fill is coral (R2). */
export function ProgressBar({
	value,
	tone = "info",
	waiting = false,
	label,
	className,
}: Readonly<{
	/** 0–100; omitted = indeterminate. */
	value?: number;
	tone?: ProgressTone;
	/** Queued behind something else: a pale fill. */
	waiting?: boolean;
	label: string;
	className?: string;
}>) {
	const fill = PROGRESS_FILL[waiting ? "waiting" : tone];
	const known = value !== undefined;
	return (
		// biome-ignore lint/a11y/useFocusableInteractive: a read-only status bar, never a control
		<div
			role="progressbar"
			aria-label={label}
			aria-valuemin={known ? 0 : undefined}
			aria-valuemax={known ? 100 : undefined}
			aria-valuenow={known ? Math.round(clampPercent(value)) : undefined}
			data-indeterminate={known ? undefined : ""}
			className={cx(
				"relative h-1 overflow-hidden rounded-xs bg-muted",
				className,
			)}
		>
			<i
				className={cx(
					"block h-full",
					fill,
					known ? "transition-[width] duration-300" : INDETERMINATE,
				)}
				style={known ? { width: `${clampPercent(value)}%` } : undefined}
			/>
		</div>
	);
}

export type StepState = "done" | "active" | "todo" | "fail";

/** One step segment (rollout steps, fleet mini steps). `active` animates; `fail` takes the fail tone. */
export function StepBar({
	state,
	failTone = "critical",
	className,
	title,
}: Readonly<{
	state: StepState;
	failTone?: "critical" | "warning";
	className?: string;
	title?: string;
}>) {
	return (
		<i
			data-s={state}
			title={title}
			className={cx(
				"relative block h-1 overflow-hidden rounded-xs",
				state === "done" && "bg-ink-2",
				state === "active" && "bg-info-bg",
				state === "todo" && "bg-muted",
				state === "fail" &&
					(failTone === "warning" ? "bg-warning-solid" : "bg-critical-solid"),
				className,
			)}
		>
			{state === "active" ? (
				<i className={cx(INDETERMINATE, "bg-info-solid")} />
			) : null}
		</i>
	);
}
