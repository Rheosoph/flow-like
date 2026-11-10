"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../../lib/utils";
import { Skeleton } from "../../../ui";

export type TelemetryBucket = "minute" | "hour" | "day";

export function formatBucketTick(value: string, bucket: TelemetryBucket) {
	const d = new Date(value);
	if (Number.isNaN(d.getTime())) return value;
	if (bucket === "minute") {
		return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
	}
	if (bucket === "hour") {
		return d.toLocaleTimeString([], { hour: "2-digit" });
	}
	return d.toLocaleDateString([], {
		month: "short",
		day: "numeric",
		timeZone: "UTC",
	});
}

export function trendBucketForHours(hours: number): TelemetryBucket {
	if (hours <= 6) return "minute";
	if (hours <= 168) return "hour";
	return "day";
}

export function StatTile({
	label,
	value,
	icon,
	extra,
	hint,
}: {
	label: string;
	value: string;
	icon?: React.ReactNode;
	extra?: React.ReactNode;
	hint?: string;
}) {
	return (
		<div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
			<div className="flex items-center justify-between text-muted-foreground">
				<span className="text-[10px] uppercase tracking-wide">{label}</span>
				{icon}
			</div>
			<div className="mt-0.5 flex items-center gap-2">
				<span className="truncate text-lg font-semibold tabular-nums">
					{value}
				</span>
				{extra}
			</div>
			{hint ? (
				<div className="mt-0.5 truncate text-[11px] text-muted-foreground">
					{hint}
				</div>
			) : null}
		</div>
	);
}

export function EmptyState({
	message,
	className,
}: {
	message: string;
	className?: string;
}) {
	return (
		<div
			className={`flex items-center justify-center rounded-lg border border-dashed py-6 text-xs text-muted-foreground ${className ?? ""}`}
		>
			{message}
		</div>
	);
}

export interface BarListRow {
	key: string;
	label: string;
	count: number;
	/** Shown instead of the raw count, e.g. a currency. */
	value?: string;
	/** A muted line under the bar. */
	detail?: React.ReactNode;
}

export function BarList({
	rows,
	loading,
	emptyMessage,
	mono = true,
}: {
	rows: BarListRow[];
	loading?: boolean;
	emptyMessage?: string;
	mono?: boolean;
}) {
	const { t } = useTranslation("admin");
	const max = Math.max(1, ...rows.map((r) => r.count));
	if (loading) {
		return (
			<div className="space-y-1.5">
				<Skeleton className="h-4 w-full" />
				<Skeleton className="h-4 w-full" />
				<Skeleton className="h-4 w-full" />
			</div>
		);
	}
	if (rows.length === 0) {
		return (
			<EmptyState
				message={
					emptyMessage ??
					t("noDataInTheSelectedWindow", "No data in the selected window.")
				}
			/>
		);
	}
	return (
		<ul className="space-y-1.5">
			{rows.map((r) => (
				<li key={r.key} className="rounded px-1 py-0.5">
					<div className="flex items-center gap-2">
						<span
							className={cn(
								"w-40 truncate text-xs font-medium",
								mono && "font-mono",
							)}
							title={r.label}
						>
							{r.label}
						</span>
						<div className="relative h-2 flex-1 overflow-hidden rounded-full bg-muted">
							<div
								className="h-full rounded-full bg-primary/60"
								style={{ width: `${(r.count / max) * 100}%` }}
							/>
						</div>
						<span className="min-w-14 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground">
							{r.value ?? r.count.toLocaleString()}
						</span>
					</div>
					{r.detail ? (
						<div className="mt-0.5 truncate text-[11px] text-muted-foreground">
							{r.detail}
						</div>
					) : null}
				</li>
			))}
		</ul>
	);
}
