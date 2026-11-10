"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { RefreshCw } from "lucide-react";
import { Button } from "../../ui/button";

export function formatCost(microDollars: number) {
	return new Intl.NumberFormat(undefined, {
		style: "currency",
		currency: "USD",
		maximumFractionDigits: 4,
	}).format(microDollars / 1_000_000);
}

export function formatCount(value: number) {
	return new Intl.NumberFormat().format(value);
}

export function formatCompactCount(value: number) {
	return new Intl.NumberFormat(undefined, {
		notation: "compact",
		maximumFractionDigits: 1,
	}).format(value);
}

export function formatDuration(ms: number | null) {
	if (ms === null) return "n/a";
	if (ms < 1000)
		return i18next.t("valMs", "{{val}} ms", { val: Math.round(ms) });
	if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
	if (ms < 3_600_000) return `${Math.round(ms / 60_000)} min`;
	return `${(ms / 3_600_000).toFixed(1)} h`;
}

export function formatPercent(value: number) {
	if (!Number.isFinite(value)) return "0%";
	return `${Math.round(value)}%`;
}

export function chartTick(value: string | number) {
	const numeric = Number(value);
	return Number.isFinite(numeric) ? formatCompactCount(numeric) : String(value);
}

export function QueryErrorState({
	message,
	onRetry,
	retrying,
}: { message: string; onRetry: () => void; retrying: boolean }) {
	const { t } = useTranslation("admin");
	return (
		<div
			role="alert"
			className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/25 bg-destructive/5 p-4 text-sm"
		>
			<p className="text-destructive">{message}</p>
			<Button variant="outline" size="sm" onClick={onRetry} disabled={retrying}>
				<RefreshCw
					className={`mr-2 size-3.5 ${retrying ? "animate-spin" : ""}`}
				/>
				{t("retry", "Retry")}
			</Button>
		</div>
	);
}
