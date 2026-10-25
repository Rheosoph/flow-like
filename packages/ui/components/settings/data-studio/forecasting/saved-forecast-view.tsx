"use client";

import { ExternalLink, Loader2, RefreshCw, TrendingUp } from "lucide-react";
import Link from "next/link";
import { useId, useMemo, useState } from "react";
import { cn } from "../../../../lib/utils";
import type { SavedStudioForecast } from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import { ForecastChart } from "./forecast-chart";
import { formatForecastTime } from "./forecast-data";
import { ForecastMetrics } from "./forecast-metrics";
import {
	type SavedForecastViewMode,
	savedForecastHref,
	savedForecastPresentation,
} from "./saved-forecast-data";
import { useSavedForecast } from "./use-saved-forecast";

export interface SavedForecastViewProps {
	appId: string;
	forecastId: string;
	userScoped?: boolean;
	target?: string;
	seriesId?: string;
	view?: SavedForecastViewMode;
	showCovariates?: boolean;
	showMetrics?: boolean;
	showControls?: boolean;
	showHeader?: boolean;
	compact?: boolean;
	height?: number;
	className?: string;
}

export function SavedForecastView(props: SavedForecastViewProps) {
	const saved = useSavedForecast(
		props.appId,
		props.forecastId,
		props.userScoped,
	);
	return (
		<div className={cn("min-w-0 space-y-3", props.className)}>
			{!props.appId || !props.forecastId ? (
				<div className="flex items-start gap-3 rounded-xl border border-dashed p-4 text-sm text-muted-foreground">
					<TrendingUp className="mt-0.5 size-4 shrink-0" />
					<p>
						Choose a saved Data Studio prediction to display its forecast here.
					</p>
				</div>
			) : saved.isLoading ? (
				<div
					className="flex min-h-40 items-center justify-center gap-2 text-sm text-muted-foreground"
					aria-live="polite"
				>
					<Loader2 className="size-4 animate-spin" />
					Loading saved prediction…
				</div>
			) : saved.isError || !saved.data ? (
				<div
					className="space-y-3 rounded-xl border border-dashed p-4 text-sm"
					aria-live="polite"
				>
					<p>
						This saved prediction is unavailable. It may have been deleted, or
						your access may have changed.
					</p>
					<Button
						type="button"
						size="sm"
						variant="outline"
						onClick={saved.refetch}
					>
						<RefreshCw className="mr-2 size-3.5" />
						Try again
					</Button>
				</div>
			) : (
				<SavedForecastContent
					key={JSON.stringify([
						saved.data.id,
						props.target,
						props.seriesId,
						props.view,
					])}
					{...props}
					saved={saved.data}
					refresh={saved.refetch}
				/>
			)}
		</div>
	);
}

function SavedForecastContent({
	saved,
	refresh,
	appId,
	forecastId,
	userScoped = false,
	target: initialTarget,
	seriesId: initialSeries,
	view: initialView = "forecast",
	showCovariates = true,
	showMetrics = true,
	showControls = true,
	showHeader = true,
	compact = false,
	height,
}: SavedForecastViewProps & {
	saved: SavedStudioForecast;
	refresh: () => void;
}) {
	const [target, setTarget] = useState(initialTarget);
	const [seriesId, setSeries] = useState(initialSeries);
	const [view, setView] = useState(initialView);
	const chart = useMemo(
		() => savedForecastPresentation(saved.result, { target, seriesId, view }),
		[saved.result, target, seriesId, view],
	);
	const evaluationWindow = chart.evaluationWindow && (
		<SavedEvaluationWindow
			window={chart.evaluationWindow}
			href={
				showControls
					? savedForecastHref(appId, forecastId, userScoped)
					: undefined
			}
		/>
	);
	return (
		<>
			{showHeader && (
				<header className="space-y-1">
					<h3 className="break-words text-sm font-medium">{saved.name}</h3>
					<p className="text-xs text-muted-foreground">
						{userScoped ? "Personal prediction" : "Project prediction"} · Saved{" "}
						{new Date(saved.created_at).toLocaleString()}
					</p>
				</header>
			)}
			{showControls && (
				<div className="flex min-w-0 flex-wrap items-end gap-2">
					{chart.targets.length > 1 ||
					(target && !chart.targets.includes(target)) ? (
						<ForecastChoice
							label="Target"
							value={chart.target}
							options={chart.targets}
							onChange={setTarget}
						/>
					) : null}
					{chart.seriesIds.length > 1 ||
					(seriesId && !chart.seriesIds.includes(seriesId)) ? (
						<ForecastChoice
							label={saved.request.mapping.series_id || "Group"}
							value={chart.seriesId}
							options={chart.seriesIds}
							onChange={setSeries}
						/>
					) : null}
					<div className="flex flex-wrap items-center gap-1">
						<Button
							type="button"
							size="sm"
							variant={view === "forecast" ? "secondary" : "ghost"}
							aria-pressed={view === "forecast"}
							onClick={() => setView("forecast")}
						>
							Outlook
						</Button>
						<Button
							type="button"
							size="sm"
							variant={view === "evaluation" ? "secondary" : "ghost"}
							aria-pressed={view === "evaluation"}
							disabled={!saved.result.backtest}
							onClick={() => setView("evaluation")}
						>
							Holdout
						</Button>
						<Button
							type="button"
							size="icon"
							variant="ghost"
							aria-label="Reload saved prediction"
							onClick={refresh}
						>
							<RefreshCw className="size-3.5" />
						</Button>
						<Button asChild size="sm" variant="ghost">
							<Link href={savedForecastHref(appId, forecastId, userScoped)}>
								Data Studio
								<ExternalLink className="ml-1.5 size-3.5" />
							</Link>
						</Button>
					</div>
				</div>
			)}
			{!chart.error && chart.evaluation && evaluationWindow}
			{chart.error ? (
				<p
					className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground"
					aria-live="polite"
				>
					{chart.error}
				</p>
			) : (
				<ForecastChart
					points={chart.points}
					cutoff={chart.cutoff}
					metric={chart.target}
					seriesLabel={
						saved.request.mapping.series_id || chart.seriesIds.length > 1
							? chart.seriesId || "(empty ID)"
							: undefined
					}
					seriesColumn={saved.request.mapping.series_id}
					seriesCount={chart.seriesIds.length}
					hasEvaluationMetrics={showMetrics && Boolean(chart.metrics)}
					quantileLevels={saved.result.quantile_levels}
					pointStatistic={saved.result.point_statistic}
					loading={false}
					stale={false}
					backtest={chart.evaluation}
					covariates={showCovariates ? chart.covariates : []}
					readOnly
					showControls={showControls}
					compact={compact}
					height={height}
				/>
			)}
			{!chart.error && showMetrics && chart.metrics && (
				<>
					{!chart.evaluation && evaluationWindow}
					<ForecastMetrics metrics={chart.metrics} compact={compact} />
				</>
			)}
			{!chart.error && chart.points.length < chart.fullPointCount && (
				<p className="text-xs text-muted-foreground">
					The chart is reduced for display. Holdout metrics use all observed
					values.
				</p>
			)}
		</>
	);
}

function SavedEvaluationWindow({
	window,
	href,
}: {
	window: NonNullable<
		ReturnType<typeof savedForecastPresentation>["evaluationWindow"]
	>;
	href?: string;
}) {
	return (
		<section
			className="space-y-1.5 rounded-lg border bg-muted/20 p-3 text-xs"
			aria-label="Saved evaluation window"
		>
			<h3 className="font-medium">Saved evaluation window</h3>
			{window.cutoff !== undefined && (
				<p className="leading-normal text-muted-foreground">
					Context through {formatForecastTime(window.cutoff, true)} UTC
				</p>
			)}
			<p className="leading-normal text-muted-foreground">
				Evaluated {formatForecastTime(window.start, true)} to{" "}
				{formatForecastTime(window.end, true)} UTC · {window.steps} steps
			</p>
			<p className="leading-normal text-muted-foreground">
				{href ? (
					<>
						<Link
							href={href}
							className="font-medium underline underline-offset-2"
						>
							Open Data Studio
						</Link>{" "}
						to choose another cutoff and run a new evaluation.
					</>
				) : (
					"This window belongs to the saved prediction. A new evaluation can be run in Data Studio."
				)}
			</p>
		</section>
	);
}

function ForecastChoice({
	label,
	value,
	options,
	onChange,
}: {
	label: string;
	value: string;
	options: string[];
	onChange: (value: string) => void;
}) {
	const id = useId();
	return (
		<div className="min-w-0 max-w-full space-y-1">
			<label htmlFor={id} className="block text-xs text-muted-foreground">
				{label}
			</label>
			<select
				id={id}
				value={value}
				onChange={(event) => onChange(event.target.value)}
				className="h-8 max-w-full rounded-md border bg-background px-2 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
			>
				{!options.includes(value) && (
					<option value={value}>Selection unavailable</option>
				)}
				{options.map((option) => (
					<option key={option} value={option}>
						{option}
					</option>
				))}
			</select>
		</div>
	);
}
