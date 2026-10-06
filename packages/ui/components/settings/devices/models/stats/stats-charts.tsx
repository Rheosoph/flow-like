"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { STATS_STEPS } from "../../../../../lib/device-management/models";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import {
	type ChartSeries,
	TimeSeriesChart,
} from "../../primitives/time-series-chart";
import { type StatsFormats, statsFormats } from "./stats-copy";
import { type StatsView, reported } from "./stats-view";

/*
 * Small multiples over one time window (plan §3.7 item 4): counts as
 * columns, timings as hourly lines, one value axis each. Charts of timings a
 * model never reported (embeddings have no first token) are left out.
 */

const HOUR = STATS_STEPS.hour;

const GRID =
	"grid grid-cols-[repeat(auto-fill,minmax(min(100%,300px),1fr))] gap-x-6 gap-y-5";

interface Shared {
	view: StatsView;
	formats: StatsFormats;
	/** The window's last hour still runs. */
	partialLast: boolean;
	busy: boolean;
	noData: string;
}

interface ChartSpec {
	id: string;
	title: string;
	summary?: string;
	kind: "columns" | "lines";
	series: ChartSeries[];
	format: (value: number) => string;
}

const inAll = (t: DevicesT, formats: StatsFormats, total: number) =>
	t("devices:models.stats.inAll", "{{total}} in all", {
		total: formats.count(total),
	});

/** Prompt tokens in, generated tokens out: the context under the work. */
function tokensSpec(t: DevicesT, view: StatsView, formats: StatsFormats) {
	const { totals, columns } = view;
	const spec: ChartSpec = {
		id: "tokens",
		title: t("devices:models.stats.tokens.title", "Tokens"),
		summary: inAll(t, formats, totals.tokensIn + totals.tokensOut),
		kind: "columns",
		format: formats.count,
		series: [
			{
				id: "in",
				label: t("devices:models.stats.tokens.in", "In"),
				tone: "context",
				values: columns.tokensIn,
				note: formats.count(totals.tokensIn),
			},
			{
				id: "out",
				label: t("devices:models.stats.tokens.out", "Out"),
				tone: "accent",
				values: columns.tokensOut,
				note: formats.count(totals.tokensOut),
			},
		],
	};
	return spec;
}

/** Answered under failed; failures wear the critical tone. */
function requestsSpec(t: DevicesT, view: StatsView, formats: StatsFormats) {
	const { totals, columns } = view;
	const spec: ChartSpec = {
		id: "requests",
		title: t("devices:models.stats.requests.title", "Requests"),
		summary: inAll(t, formats, totals.requests),
		kind: "columns",
		format: formats.count,
		series: [
			{
				id: "answered",
				label: t("devices:models.stats.requests.answered", "Answered"),
				tone: "accent",
				values: columns.answered,
				note: formats.count(totals.requests - totals.errors),
			},
			{
				id: "failed",
				label: t("devices:models.stats.requests.failed", "Failed"),
				tone: "critical",
				values: columns.failed,
				note: formats.count(totals.errors),
			},
		],
	};
	return spec;
}

const upTo = (t: DevicesT, formats: StatsFormats, value?: number) =>
	value === undefined
		? undefined
		: t("devices:models.stats.upTo", "up to {{value}}", {
				value: formats.ms(value),
			});

const p95Label = (t: DevicesT) =>
	t("devices:models.stats.p95", "95th percentile");

function ttftSpec(t: DevicesT, view: StatsView, formats: StatsFormats) {
	const { hourly, totals } = view;
	const spec: ChartSpec = {
		id: "ttft",
		title: t("devices:models.stats.ttft.title", "Time to first token"),
		summary: upTo(t, formats, totals.ttftP95Max),
		kind: "lines",
		format: formats.ms,
		series: [
			{
				id: "p50",
				label: t("devices:models.stats.median", "Median"),
				tone: "context",
				values: hourly.ttftP50,
			},
			{
				id: "p95",
				label: p95Label(t),
				tone: "accent",
				values: hourly.ttftP95,
			},
		],
	};
	return spec;
}

function speedSpec(t: DevicesT, view: StatsView, formats: StatsFormats) {
	const { hourly, totals } = view;
	const average = totals.tokensPerSecond;
	const spec: ChartSpec = {
		id: "speed",
		title: t("devices:models.stats.speed.title", "Generation speed"),
		summary:
			average === undefined
				? undefined
				: t("devices:models.stats.speed.summary", "{{value}} on average", {
						value: formats.speed(average),
					}),
		kind: "lines",
		format: formats.speed,
		series: [
			{
				id: "speed",
				label: t("devices:models.stats.speed.series", "Generated"),
				tone: "accent",
				values: hourly.tokensPerSecond,
			},
		],
	};
	return spec;
}

function queueSpec(t: DevicesT, view: StatsView, formats: StatsFormats) {
	const spec: ChartSpec = {
		id: "queue",
		title: t("devices:models.stats.queue.title", "Queue wait"),
		summary: upTo(t, formats, view.totals.queueP95Max),
		kind: "lines",
		format: formats.ms,
		series: [
			{
				id: "queue",
				label: p95Label(t),
				tone: "accent",
				values: view.hourly.queueP95,
			},
		],
	};
	return spec;
}

/** Timing charts the device reported something for. */
function lineSpecs(t: DevicesT, view: StatsView, formats: StatsFormats) {
	const { hourly } = view;
	const specs: ChartSpec[] = [];
	if (reported(hourly.ttftP50) || reported(hourly.ttftP95))
		specs.push(ttftSpec(t, view, formats));
	if (reported(hourly.tokensPerSecond)) specs.push(speedSpec(t, view, formats));
	if (reported(hourly.queueP95)) specs.push(queueSpec(t, view, formats));
	return specs;
}

function StatsChart({
	spec,
	shared,
}: Readonly<{ spec: ChartSpec; shared: Shared }>) {
	const { view } = shared;
	return (
		<TimeSeriesChart
			title={spec.title}
			summary={spec.summary}
			kind={spec.kind}
			series={spec.series}
			start={view.from}
			step={spec.kind === "columns" ? view.columns.step : HOUR}
			format={spec.format}
			partialLast={shared.partialLast}
			emptyText={shared.noData}
			busy={shared.busy}
		/>
	);
}

/** Tokens and requests always; first-token time, speed and queue wait where the device reported them. */
export function StatsCharts({
	view,
	partialLast,
	busy,
}: Readonly<{ view: StatsView; partialLast: boolean; busy: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const formats = useMemo(() => statsFormats(t, time.locale), [t, time.locale]);
	const specs = useMemo(
		() => [
			tokensSpec(t, view, formats),
			requestsSpec(t, view, formats),
			...lineSpecs(t, view, formats),
		],
		[t, view, formats],
	);
	const shared: Shared = {
		view,
		formats,
		partialLast,
		busy,
		noData: t("devices:models.stats.noData", "Nothing reported in this range."),
	};
	return (
		<div data-stats-charts="" className={GRID}>
			{specs.map((spec) => (
				<StatsChart key={spec.id} spec={spec} shared={shared} />
			))}
		</div>
	);
}
