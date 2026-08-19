"use client";

import { i18n as i18next } from "@flow-like/locales";
import { Badge } from "../../../ui";
import type { ITelemetryPerfRating } from "./types";

export const TRACE_HOUR_OPTIONS: { value: number; label: string }[] = [
	{ value: 1, label: "Last hour" },
	{ value: 6, label: "Last 6 hours" },
	{ value: 24, label: "Last 24 hours" },
	{ value: 72, label: "Last 3 days" },
	{ value: 168, label: "Last 7 days" },
];

export const TRACE_SOURCE_OPTIONS = [
	"backend",
	"desktop",
	"web",
	"desktop_native",
] as const;

export const TRACE_STATUS_OPTIONS = ["ok", "error"] as const;

export const MIN_DURATION_OPTIONS: { value: number; label: string }[] = [
	{ value: 0, label: "Any duration" },
	{ value: 100, label: "Slower than 100 ms" },
	{ value: 500, label: "Slower than 500 ms" },
	{ value: 1000, label: "Slower than 1 s" },
	{ value: 5000, label: "Slower than 5 s" },
];

export const PERF_METRIC_LABELS: Record<string, string> = {
	lcp: "Largest contentful paint",
	inp: "Interaction to next paint",
	cls: "Cumulative layout shift",
	ttfb: "Time to first byte",
	fcp: "First contentful paint",
	app_start: "App start",
	screen_load: "Screen load",
};

export const PERF_METRIC_ORDER = [
	"lcp",
	"inp",
	"cls",
	"ttfb",
	"fcp",
	"app_start",
	"screen_load",
];

export function perfMetricLabel(metric: string) {
	return PERF_METRIC_LABELS[metric] ?? metric;
}

export function isUnitlessMetric(metric: string) {
	return metric === "cls";
}

export function formatDurationMs(ms: number): string {
	if (!Number.isFinite(ms)) return "—";
	if (ms >= 60_000) {
		const minutes = Math.floor(ms / 60_000);
		const seconds = Math.round((ms % 60_000) / 1000);
		return `${minutes}m ${seconds}s`;
	}
	if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
	if (ms >= 1) return i18next.t("valMs", "{{val}} ms", { val: Math.round(ms) });
	return i18next.t("valMs", "{{val}} ms", { val: ms.toFixed(2) });
}

export function formatMetricValue(metric: string, value: number): string {
	if (!Number.isFinite(value)) return "—";
	return isUnitlessMetric(metric) ? value.toFixed(3) : formatDurationMs(value);
}

export function isErrorStatus(status: string) {
	return status === "error";
}

export function ratingLabel(rating: string): string {
	if (rating === "good") return "Good";
	if (rating === "needs-improvement") return "Needs improvement";
	if (rating === "poor") return "Poor";
	return rating;
}

interface RatingTone {
	tile: string;
	text: string;
	dot: string;
}

const RATING_TONES: Record<ITelemetryPerfRating, RatingTone> = {
	good: {
		tile: "border-emerald-500/40 bg-emerald-500/5",
		text: "text-emerald-600 dark:text-emerald-400",
		dot: "bg-emerald-500",
	},
	"needs-improvement": {
		tile: "border-amber-500/40 bg-amber-500/5",
		text: "text-amber-600 dark:text-amber-400",
		dot: "bg-amber-500",
	},
	poor: {
		tile: "border-destructive/40 bg-destructive/5",
		text: "text-destructive",
		dot: "bg-destructive",
	},
};

const NEUTRAL_TONE: RatingTone = {
	tile: "border-border bg-muted/40",
	text: "text-muted-foreground",
	dot: "bg-muted-foreground",
};

export function ratingTone(rating: string): RatingTone {
	return RATING_TONES[rating as ITelemetryPerfRating] ?? NEUTRAL_TONE;
}

export function RatingBadge({ rating }: { readonly rating: string }) {
	const tone = ratingTone(rating);
	return (
		<span
			className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium ${tone.tile} ${tone.text}`}
		>
			<span className={`h-1.5 w-1.5 rounded-full ${tone.dot}`} />
			{ratingLabel(rating)}
		</span>
	);
}

export function SpanStatusBadge({ status }: { readonly status: string }) {
	return (
		<Badge
			variant={isErrorStatus(status) ? "destructive" : "outline"}
			className="font-mono text-[10px] uppercase"
		>
			{status}
		</Badge>
	);
}
