"use client";

import { useTranslation } from "@flow-like/locales";
import { ResponsiveBar } from "@nivo/bar";
import type { BarDatum, BarTooltipProps } from "@nivo/bar";
import { ResponsiveLine } from "@nivo/line";
import type { PointTooltipProps } from "@nivo/line";
import {
	ActivityIcon,
	AlertTriangleIcon,
	BrainIcon,
	CheckCircleIcon,
	ClockIcon,
	ExternalLinkIcon,
	FilterIcon,
	MessageSquareIcon,
	ThumbsDownIcon,
	ThumbsUpIcon,
	TrendingUpIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
	type ComponentType,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { toast } from "sonner";

import type { IEvent } from "../../../lib";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type {
	IAnalyticsOverview,
	IDailyAnalyticsStat,
	IFeedbackItem,
} from "../../../state/backend-state/analytics-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Skeleton } from "../../ui/skeleton";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";

const MICRO_DOLLARS_PER_DOLLAR = 1_000_000;

const analyticsCardClassName =
	"border-border/60 bg-card/85 shadow-sm shadow-black/5 dark:bg-card/70 dark:shadow-black/20";

const chartContainerClassName = "h-[280px] min-w-0";

const chartColors = {
	executions: "var(--chart-1)",
	users: "var(--chart-2)",
	success: "oklch(0.72 0.16 150)",
	failure: "var(--destructive)",
	latencyAvg: "var(--chart-3)",
	latencyP95: "var(--chart-4)",
	llm: "var(--chart-1)",
	embedding: "var(--chart-5)",
	positive: "oklch(0.72 0.16 150)",
	negative: "var(--destructive)",
};

const nivoTheme = {
	text: {
		fill: "var(--foreground)",
		fontSize: 11,
	},
	axis: {
		domain: { line: { stroke: "var(--border)" } },
		ticks: {
			line: { stroke: "var(--border)" },
			text: { fill: "var(--muted-foreground)", fontSize: 11 },
		},
		legend: { text: { fill: "var(--muted-foreground)", fontSize: 11 } },
	},
	grid: { line: { stroke: "var(--border)", strokeOpacity: 0.55 } },
	legends: { text: { fill: "var(--muted-foreground)", fontSize: 11 } },
	crosshair: { line: { stroke: "var(--primary)" } },
	tooltip: {
		container: {
			background: "var(--popover)",
			color: "var(--popover-foreground)",
			borderRadius: "8px",
			boxShadow:
				"0 12px 30px color-mix(in oklch, var(--foreground) 14%, transparent)",
			border: "1px solid var(--border)",
			fontSize: "12px",
		},
	},
};

type AnalyticsLineSeries = {
	id: string;
	data: Array<{ x: string; y: number }>;
};

function parseDate(dateStr: string): Date {
	const [year, month, day] = dateStr.split("-").map(Number);
	return new Date(year, (month ?? 1) - 1, day ?? 1);
}

function formatChartDate(dateStr: string): string {
	return parseDate(dateStr).toLocaleDateString("en-US", {
		month: "short",
		day: "numeric",
	});
}

function formatDateTime(dateStr: string): string {
	return new Date(dateStr).toLocaleDateString("en-US", {
		month: "short",
		day: "numeric",
		year: "numeric",
	});
}

function compactIdentifier(value?: string | null): string | null {
	const trimmed = value?.trim();
	if (!trimmed) return null;
	if (trimmed.length <= 18) return trimmed;
	return `${trimmed.slice(0, 8)}...${trimmed.slice(-6)}`;
}

function formatFeedbackPageSource(
	item: IFeedbackItem,
	event?: IEvent,
): string | null {
	const path =
		item.pagePath?.trim() ||
		item.routePathname?.trim() ||
		item.eventRoute?.trim() ||
		getEventRoute(event) ||
		null;
	const search = item.pageSearch?.trim() || "";
	const hash = item.pageHash?.trim() || "";

	if (!path && !search && !hash) return null;
	return `${path ?? ""}${search}${hash}`;
}

function getEventRoute(event?: IEvent): string | null {
	const route = typeof event?.route === "string" ? event.route.trim() : "";
	return route || null;
}

function getEventPageId(event?: IEvent): string | null {
	const pageId =
		typeof event?.default_page_id === "string"
			? event.default_page_id.trim()
			: "";
	return pageId || null;
}

function formatFeedbackEventSource(
	item: IFeedbackItem,
	event?: IEvent,
): string | null {
	return (
		item.eventName?.trim() ||
		event?.name?.trim() ||
		compactIdentifier(item.eventId)
	);
}

function getAnalyticsEventHref(
	appId: string | null | undefined,
	eventId: string | null | undefined,
): string | null {
	const app = appId?.trim();
	const event = eventId?.trim();
	if (!app || !event) return null;

	const params = new URLSearchParams();
	params.set("id", app);
	params.set("eventId", event);
	return `/library/config/analytics?${params.toString()}`;
}

function formatLocalDate(date: Date): string {
	const localDate = new Date(
		date.getTime() - date.getTimezoneOffset() * 60_000,
	);
	return localDate.toISOString().split("T")[0] ?? "";
}

function formatNumber(value: number): string {
	return new Intl.NumberFormat("en-US").format(value);
}

function formatCompactNumber(value: number): string {
	return new Intl.NumberFormat("en-US", {
		notation: "compact",
		maximumFractionDigits: value < 10 ? 1 : 0,
	}).format(value);
}

function formatPercent(value: number): string {
	return `${value.toFixed(value < 10 ? 1 : 0)}%`;
}

function formatLatency(ms: number | null | undefined): string {
	if (ms === null || ms === undefined) return "N/A";
	if (ms < 1000) return `${Math.round(ms)}ms`;
	return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`;
}

function microDollarsToDollars(value: number): number {
	return value / MICRO_DOLLARS_PER_DOLLAR;
}

function formatDollarAmount(amount: number): string {
	if (amount === 0) return "$0.00";
	if (Math.abs(amount) < 0.0001) return "<$0.0001";
	if (Math.abs(amount) < 0.01) return `$${amount.toFixed(4)}`;
	return new Intl.NumberFormat("en-US", {
		style: "currency",
		currency: "USD",
		minimumFractionDigits: 2,
		maximumFractionDigits: 2,
	}).format(amount);
}

function formatCost(microDollars: number): string {
	return formatDollarAmount(microDollarsToDollars(microDollars));
}

function getTickValues(labels: string[]): string[] {
	if (labels.length <= 8) return labels;
	const step = Math.ceil(labels.length / 8);
	return labels.filter(
		(_, index) => index % step === 0 || index === labels.length - 1,
	);
}

function getSuccessfulExecutions(day: IDailyAnalyticsStat): number {
	return Math.max(day.executions - getFailedExecutions(day), 0);
}

function getFailedExecutions(day: IDailyAnalyticsStat): number {
	return day.failedExecutions ?? 0;
}

function ChartTooltip({
	color,
	title,
	subtitle,
	value,
}: {
	color: string;
	title: string;
	subtitle: string;
	value: string;
}) {
	return (
		<div className="rounded-lg border border-border bg-popover px-3 py-2 text-popover-foreground shadow-xl">
			<div className="flex items-center gap-2">
				<span
					className="h-2.5 w-2.5 rounded-sm"
					style={{ backgroundColor: color }}
				/>
				<span className="text-sm font-semibold">{title}</span>
			</div>
			<div className="mt-1 flex items-center justify-between gap-5 text-xs">
				<span className="text-muted-foreground">{subtitle}</span>
				<span className="font-medium text-foreground">{value}</span>
			</div>
		</div>
	);
}

function CountLineTooltip({ point }: PointTooltipProps<AnalyticsLineSeries>) {
	return (
		<ChartTooltip
			color={point.seriesColor}
			title={String(point.seriesId)}
			subtitle={String(point.data.xFormatted)}
			value={formatNumber(Number(point.data.y))}
		/>
	);
}

function LatencyLineTooltip({ point }: PointTooltipProps<AnalyticsLineSeries>) {
	return (
		<ChartTooltip
			color={point.seriesColor}
			title={String(point.seriesId)}
			subtitle={String(point.data.xFormatted)}
			value={formatLatency(Number(point.data.y))}
		/>
	);
}

const reliabilityTooltipLabels: Record<string, string> = {
	successful: "Successful executions",
	failed: "Failed executions",
};

const costTooltipLabels: Record<string, string> = {
	llm: "LLM cost",
	embeddings: "Embedding cost",
};

const feedbackTooltipLabels: Record<string, string> = {
	positive: "Positive feedback",
	negative: "Negative feedback",
};

function CountBarTooltip({
	color,
	id,
	indexValue,
	value,
}: BarTooltipProps<BarDatum>) {
	return (
		<ChartTooltip
			color={color}
			title={reliabilityTooltipLabels[String(id)] ?? String(id)}
			subtitle={String(indexValue)}
			value={formatNumber(value)}
		/>
	);
}

function CostBarTooltip({
	color,
	id,
	indexValue,
	value,
}: BarTooltipProps<BarDatum>) {
	return (
		<ChartTooltip
			color={color}
			title={costTooltipLabels[String(id)] ?? String(id)}
			subtitle={String(indexValue)}
			value={formatDollarAmount(value)}
		/>
	);
}

function FeedbackBarTooltip({
	color,
	id,
	indexValue,
	value,
}: BarTooltipProps<BarDatum>) {
	return (
		<ChartTooltip
			color={color}
			title={feedbackTooltipLabels[String(id)] ?? String(id)}
			subtitle={String(indexValue)}
			value={formatNumber(value)}
		/>
	);
}

function EmptyChart({ label }: { label: string }) {
	return (
		<div className="flex h-full min-h-[220px] items-center justify-center rounded-lg border border-dashed border-border/70 bg-muted/20 px-4 text-center text-sm text-muted-foreground">
			{label}
		</div>
	);
}

function MetricCard({
	title,
	value,
	detail,
	icon: Icon,
	tone = "neutral",
}: {
	title: string;
	value: string;
	detail: string;
	icon: ComponentType<{ className?: string }>;
	tone?: "neutral" | "success" | "warning" | "danger";
}) {
	const { t } = useTranslation("settings");
	const toneClassName = {
		neutral:
			"bg-[color-mix(in_oklch,var(--primary)_12%,transparent)] text-primary",
		success: `bg-emerald-500/10 text-emerald-600 dark:text-emerald-400`,
		warning: `bg-amber-500/10 text-amber-600 dark:text-amber-400`,
		danger: "bg-destructive/10 text-destructive",
	}[tone];

	return (
		<Card className={analyticsCardClassName}>
			<CardContent className="p-4">
				<div className="flex items-start justify-between gap-3">
					<div className="min-w-0 space-y-1">
						<p className="text-sm font-medium text-muted-foreground">{title}</p>
						<p className="truncate text-2xl font-semibold">{value}</p>
					</div>
					<div className={cn("rounded-md p-2", toneClassName)}>
						<Icon className="h-4 w-4" />
					</div>
				</div>
				<p className="mt-3 min-h-4 truncate text-xs text-muted-foreground">
					{detail}
				</p>
			</CardContent>
		</Card>
	);
}

function ActivityTrendChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const { t } = useTranslation("settings");
	const labels = useMemo(
		() => data.map((d) => formatChartDate(d.date)),
		[data],
	);
	const hasActivity = data.some((d) => d.executions > 0 || d.uniqueUsers > 0);
	const chartData = useMemo(
		() => [
			{
				id: "Executions",
				data: data.map((d) => ({
					x: formatChartDate(d.date),
					y: d.executions,
				})),
			},
			{
				id: "Unique users",
				data: data.map((d) => ({
					x: formatChartDate(d.date),
					y: d.uniqueUsers,
				})),
			},
		],
		[data],
	);

	if (data.length === 0 || !hasActivity) {
		return <EmptyChart label={t('noExecutionsInThisDateRange', 'No executions in this date range')} />;
	}

	return (
		<div className={chartContainerClassName}>
			<ResponsiveLine
				data={chartData}
				margin={{ top: 18, right: 24, bottom: 42, left: 46 }}
				xScale={{ type: "point" }}
				yScale={{ type: "linear", min: 0, max: "auto" }}
				yFormat={(value) => formatNumber(Number(value))}
				axisBottom={{
					tickRotation: 0,
					tickValues: getTickValues(labels),
				}}
				axisLeft={{
					format: (value) => formatCompactNumber(Number(value)),
				}}
				colors={[chartColors.executions, chartColors.users]}
				curve="monotoneX"
				enableArea={true}
				areaOpacity={0.08}
				enableGridX={false}
				pointSize={data.length <= 31 ? 7 : 4}
				pointBorderWidth={2}
				pointBorderColor={{ from: "serieColor" }}
				useMesh={true}
				tooltip={CountLineTooltip}
				legends={[
					{
						anchor: "top-right",
						direction: "row",
						translateY: -16,
						itemWidth: 102,
						itemHeight: 16,
						symbolSize: 8,
						symbolShape: "circle",
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function ReliabilityChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const { t } = useTranslation("settings");
	const chartData = useMemo(
		() =>
			data.map((d) => ({
				date: formatChartDate(d.date),
				successful: getSuccessfulExecutions(d),
				failed: getFailedExecutions(d),
			})),
		[data],
	);
	const hasExecutions = data.some((d) => d.executions > 0);

	if (data.length === 0 || !hasExecutions) {
		return <EmptyChart label={t('noReliabilityDataInThisDateRange', 'No reliability data in this date range')} />;
	}

	return (
		<div className={chartContainerClassName}>
			<ResponsiveBar
				data={chartData}
				keys={["successful", "failed"]}
				indexBy="date"
				margin={{ top: 18, right: 18, bottom: 42, left: 44 }}
				padding={0.25}
				groupMode="stacked"
				colors={[chartColors.success, chartColors.failure]}
				enableLabel={false}
				borderRadius={2}
				valueFormat={(value) => formatNumber(Number(value))}
				tooltip={CountBarTooltip}
				axisBottom={{
					tickRotation: 0,
					tickValues: getTickValues(chartData.map((d) => d.date)),
				}}
				axisLeft={{
					format: (value) => formatCompactNumber(Number(value)),
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "top-right",
						direction: "row",
						translateY: -16,
						itemWidth: 78,
						itemHeight: 16,
						symbolSize: 8,
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function LatencyChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const { t } = useTranslation("settings");
	const chartData = useMemo(
		() =>
			[
				{
					id: "Average",
					data: data
						.filter((d) => d.avgLatency !== null)
						.map((d) => ({
							x: formatChartDate(d.date),
							y: d.avgLatency ?? 0,
						})),
				},
				{
					id: "Daily p95",
					data: data
						.filter((d) => d.p95Latency !== null)
						.map((d) => ({
							x: formatChartDate(d.date),
							y: d.p95Latency ?? 0,
						})),
				},
			].filter((series) => series.data.length > 0),
		[data],
	);

	if (chartData.length === 0) {
		return <EmptyChart label={t('noLatencySamplesInThisDateRange', 'No latency samples in this date range')} />;
	}

	return (
		<div className={chartContainerClassName}>
			<ResponsiveLine
				data={chartData}
				margin={{ top: 18, right: 24, bottom: 42, left: 56 }}
				xScale={{ type: "point" }}
				yScale={{ type: "linear", min: 0, max: "auto" }}
				yFormat={(value) => formatLatency(Number(value))}
				axisBottom={{
					tickRotation: 0,
					tickValues: getTickValues(data.map((d) => formatChartDate(d.date))),
				}}
				axisLeft={{
					format: (value) => formatLatency(Number(value)),
				}}
				colors={[chartColors.latencyAvg, chartColors.latencyP95]}
				curve="monotoneX"
				enableArea={false}
				enableGridX={false}
				pointSize={data.length <= 31 ? 7 : 4}
				pointBorderWidth={2}
				pointBorderColor={{ from: "serieColor" }}
				useMesh={true}
				tooltip={LatencyLineTooltip}
				legends={[
					{
						anchor: "top-right",
						direction: "row",
						translateY: -16,
						itemWidth: 86,
						itemHeight: 16,
						symbolSize: 8,
						symbolShape: "circle",
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function CostTrendChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const { t } = useTranslation("settings");
	const chartData = useMemo(
		() =>
			data.map((d) => ({
				date: formatChartDate(d.date),
				llm: microDollarsToDollars(d.llmCost),
				embeddings: microDollarsToDollars(d.embeddingCost),
			})),
		[data],
	);
	const hasCost = chartData.some((d) => d.llm > 0 || d.embeddings > 0);

	if (data.length === 0 || !hasCost) {
		return <EmptyChart label={t('noAiCostInThisDateRange', 'No AI cost in this date range')} />;
	}

	return (
		<div className={chartContainerClassName}>
			<ResponsiveBar
				data={chartData}
				keys={["llm", "embeddings"]}
				indexBy="date"
				margin={{ top: 18, right: 18, bottom: 42, left: 58 }}
				padding={0.25}
				groupMode="stacked"
				colors={[chartColors.llm, chartColors.embedding]}
				enableLabel={false}
				borderRadius={2}
				valueFormat={(value) => formatDollarAmount(Number(value))}
				tooltip={CostBarTooltip}
				axisBottom={{
					tickRotation: 0,
					tickValues: getTickValues(chartData.map((d) => d.date)),
				}}
				axisLeft={{
					format: (value) => formatDollarAmount(Number(value)),
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "top-right",
						direction: "row",
						translateY: -16,
						itemWidth: 88,
						itemHeight: 16,
						symbolSize: 8,
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function FeedbackTrendChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const { t } = useTranslation("settings");
	const chartData = useMemo(
		() =>
			data.map((d) => ({
				date: formatChartDate(d.date),
				positive: d.positiveFeedback,
				negative: d.negativeFeedback,
			})),
		[data],
	);
	const hasFeedback = data.some(
		(d) => d.positiveFeedback > 0 || d.negativeFeedback > 0,
	);

	if (data.length === 0 || !hasFeedback) {
		return <EmptyChart label={t('noFeedbackSignalInThisDateRange', 'No feedback signal in this date range')} />;
	}

	return (
		<div className={chartContainerClassName}>
			<ResponsiveBar
				data={chartData}
				keys={["positive", "negative"]}
				indexBy="date"
				margin={{ top: 18, right: 18, bottom: 42, left: 44 }}
				padding={0.25}
				groupMode="stacked"
				colors={[chartColors.positive, chartColors.negative]}
				enableLabel={false}
				borderRadius={2}
				valueFormat={(value) => formatNumber(Number(value))}
				tooltip={FeedbackBarTooltip}
				axisBottom={{
					tickRotation: 0,
					tickValues: getTickValues(chartData.map((d) => d.date)),
				}}
				axisLeft={{
					format: (value) => formatCompactNumber(Number(value)),
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "top-right",
						direction: "row",
						translateY: -16,
						itemWidth: 82,
						itemHeight: 16,
						symbolSize: 8,
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function FeedbackBadge({ rating }: { rating: number }) {
	const { t } = useTranslation("settings");
	if (rating > 0) {
		return (
			<Badge className="bg-emerald-500/10 text-emerald-700 dark:text-emerald-300">
				<ThumbsUpIcon className="mr-1 h-3 w-3" />
				{t('positive', 'Positive')}
			</Badge>
		);
	}

	if (rating < 0) {
		return (
			<Badge className="bg-destructive/10 text-destructive">
				<ThumbsDownIcon className="mr-1 h-3 w-3" />
				{t('negative', 'Negative')}
			</Badge>
		);
	}

	return <Badge variant="secondary">{t('neutral', 'Neutral')}</Badge>;
}

export function AnalyticsDashboard() {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const analyticsState = backend.analyticsState;
	const eventState = backend.eventState;
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const appId = searchParams.get("id");
	const eventIdParam = searchParams.get("eventId")?.trim() || "all";

	const [loading, setLoading] = useState(true);
	const [overview, setOverview] = useState<IAnalyticsOverview | null>(null);
	const [dailyStats, setDailyStats] = useState<IDailyAnalyticsStat[]>([]);
	const [feedbackItems, setFeedbackItems] = useState<IFeedbackItem[]>([]);
	const [feedbackTotal, setFeedbackTotal] = useState(0);
	const [feedbackPage, setFeedbackPage] = useState(0);
	const [events, setEvents] = useState<IEvent[]>([]);

	const [dateRange, setDateRange] = useState<"7d" | "30d" | "90d">("30d");
	const [selectedEventId, setSelectedEventId] = useState(eventIdParam);
	const [feedbackFilter, setFeedbackFilter] = useState<
		"all" | "positive" | "negative"
	>("all");

	const FEEDBACK_LIMIT = 20;

	const loadData = useCallback(async () => {
		if (!appId || !analyticsState) return;

		setLoading(true);
		try {
			const days = dateRange === "7d" ? 7 : dateRange === "30d" ? 30 : 90;
			const end = new Date();
			const start = new Date(end);
			start.setDate(end.getDate() - (days - 1));

			const endDate = formatLocalDate(end);
			const startDate = formatLocalDate(start);

			const minRating = feedbackFilter === "positive" ? 1 : undefined;
			const maxRating = feedbackFilter === "negative" ? -1 : undefined;
			const eventFilter =
				selectedEventId === "all" ? undefined : selectedEventId;

			const [dashboardData, feedbackData, eventsData] = await Promise.all([
				analyticsState.getAnalyticsDashboard(
					appId,
					startDate,
					endDate,
					"day",
					eventFilter,
				),
				analyticsState.listFeedback(
					appId,
					feedbackPage * FEEDBACK_LIMIT,
					FEEDBACK_LIMIT,
					minRating,
					maxRating,
					eventFilter,
				),
				eventState.getEvents(appId).catch(() => []),
			]);

			setOverview(dashboardData.stats.summary ?? dashboardData.overview);
			setDailyStats(dashboardData.stats.dailyStats);
			setFeedbackItems(feedbackData.items);
			setFeedbackTotal(feedbackData.total);
			setEvents(eventsData);
		} catch (error) {
			toast.error(
				error instanceof Error
					? t('failedToLoadAnalyticsMessage', 'Failed to load analytics: {{message}}', { message: error.message })
					: t('failedToLoadAnalyticsData', 'Failed to load analytics data'),
			);
		} finally {
			setLoading(false);
		}
	}, [
		appId,
		dateRange,
		feedbackFilter,
		feedbackPage,
		selectedEventId,
		analyticsState,
		eventState,
	]);

	useEffect(() => {
		loadData();
	}, [loadData]);

	useEffect(() => {
		setSelectedEventId(eventIdParam);
		setFeedbackPage(0);
	}, [eventIdParam]);

	const updateEventFilter = useCallback(
		(value: string) => {
			setSelectedEventId(value);
			setFeedbackPage(0);

			const params = new URLSearchParams(searchParams.toString());
			if (value === "all") {
				params.delete("eventId");
			} else {
				params.set("eventId", value);
			}

			const query = params.toString();
			router.replace(query ? `${pathname}?${query}` : pathname, {
				scroll: false,
			});
		},
		[pathname, router, searchParams],
	);

	const feedbackTotalPages = Math.ceil(feedbackTotal / FEEDBACK_LIMIT);
	const dateRangeLabel =
		dateRange === "7d"
			? t('last7Days', 'Last 7 days')
			: dateRange === "30d"
				? t('last30Days', 'Last 30 days')
				: t('last90Days', 'Last 90 days');
	const analyticsScopeCopy =
		selectedEventId === "all"
			? t('usageReliabilityCostAndFeedback', 'Usage, reliability, cost, and feedback')
			: t('usageReliabilityAndFeedback', 'Usage, reliability, and feedback');
	const eventOptions = useMemo(
		() =>
			[...events]
				.filter((event) => event.id?.trim())
				.sort((a, b) => a.name.localeCompare(b.name)),
		[events],
	);
	const eventLookup = useMemo(
		() => new Map(eventOptions.map((event) => [event.id, event])),
		[eventOptions],
	);
	const selectedEvent =
		selectedEventId === "all" ? undefined : eventLookup.get(selectedEventId);
	const selectedEventLabel =
		selectedEventId === "all"
			? t('allEvents', 'all events')
			: selectedEvent?.name ||
				compactIdentifier(selectedEventId) ||
				"selected event";
	const selectedEventIsMissing =
		selectedEventId !== "all" && !selectedEvent && selectedEventId.trim();

	const totalExecutions = overview?.totalExecutions ?? 0;
	const failedExecutions = overview?.failedExecutions ?? 0;
	const successfulExecutions =
		totalExecutions > 0
			? Math.max(totalExecutions - failedExecutions, 0)
			: (overview?.successfulExecutions ?? 0);
	const uniqueUsers = overview?.uniqueUsers ?? 0;
	const successRate =
		totalExecutions > 0 ? (successfulExecutions / totalExecutions) * 100 : null;
	const totalCost =
		(overview?.totalLlmCost ?? 0) + (overview?.totalEmbeddingCost ?? 0);
	const costPerExecution =
		totalExecutions > 0 ? totalCost / totalExecutions : 0;
	const totalFeedback = overview?.totalFeedback ?? 0;
	const positiveFeedback = overview?.positiveFeedback ?? 0;
	const negativeFeedback = overview?.negativeFeedback ?? 0;
	const positiveRate =
		totalFeedback > 0 ? (positiveFeedback / totalFeedback) * 100 : null;
	const runsPerUser =
		uniqueUsers > 0 ? (totalExecutions / uniqueUsers).toFixed(1) : "0";
	const peakP95Latency = useMemo(() => {
		const values = dailyStats
			.map((day) => day.p95Latency)
			.filter(
				(value): value is number => value !== null && value !== undefined,
			);
		return values.length > 0 ? Math.max(...values) : null;
	}, [dailyStats]);

	if (!analyticsState) {
		return (
			<div className="flex h-full items-center justify-center">
				<p className="text-muted-foreground">{t('analyticsIsNotAvailable', 'Analytics is not available')}</p>
			</div>
		);
	}

	if (!appId) {
		return (
			<div className="flex h-full items-center justify-center">
				<p className="text-muted-foreground">{t('noAppSelected2', 'No app selected')}</p>
			</div>
		);
	}

	if (loading) {
		return (
			<div className="space-y-5 p-6">
				<div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
					{Array.from({ length: 5 }).map((_, index) => (
						<Skeleton key={`analytics-card-${index}`} className="h-32" />
					))}
				</div>
				<div className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.65fr)]">
					<Skeleton className="h-[370px]" />
					<Skeleton className="h-[370px]" />
				</div>
				<div className="grid gap-4 xl:grid-cols-2">
					<Skeleton className="h-[370px]" />
					<Skeleton className="h-[370px]" />
				</div>
			</div>
		);
	}

	return (
		<div className="space-y-5 text-foreground">
			<div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
				<div className="space-y-1">
					<h1 className="text-2xl font-semibold">{t('analytics', 'Analytics')}</h1>
					<p className="text-sm text-muted-foreground">{t('analyticsscopecopyFor', '{{analyticsScopeCopy}} for', { analyticsScopeCopy })}{dateRangeLabel.toLowerCase()} across{" "}
						{selectedEventLabel}
					</p>
				</div>
				<div className="flex flex-col gap-2 sm:flex-row sm:items-center">
					<Select
						value={dateRange}
						onValueChange={(value) => setDateRange(value as typeof dateRange)}
					>
						<SelectTrigger className="w-full bg-background/70 sm:w-40 dark:bg-background/30">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="7d">{t('last7Days', 'Last 7 days')}</SelectItem>
							<SelectItem value="30d">{t('last30Days', 'Last 30 days')}</SelectItem>
							<SelectItem value="90d">{t('last90Days', 'Last 90 days')}</SelectItem>
						</SelectContent>
					</Select>
					<Select value={selectedEventId} onValueChange={updateEventFilter}>
						<SelectTrigger className="w-full bg-background/70 sm:w-60 dark:bg-background/30">
							<FilterIcon className="mr-2 h-4 w-4 text-muted-foreground" />
							<SelectValue placeholder={t('allEvents2', 'All events')} />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="all">{t('allEvents2', 'All events')}</SelectItem>
							{selectedEventIsMissing && (
								<SelectItem value={selectedEventId}>
									{compactIdentifier(selectedEventId) || "Selected event"}
								</SelectItem>
							)}
							{eventOptions.map((event) => (
								<SelectItem key={event.id} value={event.id}>
									{event.name ||
										compactIdentifier(event.id) ||
										"Untitled event"}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					{selectedEventId !== "all" && (
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => updateEventFilter("all")}
						>
							<XIcon className="mr-1 h-3.5 w-3.5" />
							{t('clear', 'Clear')}
						</Button>
					)}
				</div>
			</div>

			<div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
				<MetricCard
					title="Executions"
					value={formatNumber(totalExecutions)}
					detail={t('valSuccessfulVal2Failed', '{{val}} successful, {{val2}} failed', { val: formatNumber(successfulExecutions), val2: formatNumber(failedExecutions) })}
					icon={ActivityIcon}
					tone={failedExecutions > 0 ? "warning" : "neutral"}
				/>
				<MetricCard
					title="Users"
					value={formatNumber(uniqueUsers)}
					detail={t('runsperuserExecutionsPerUser', '{{runsPerUser}} executions per user', { runsPerUser })}
					icon={UsersIcon}
				/>
				<MetricCard
					title="Reliability"
					value={successRate === null ? "N/A" : formatPercent(successRate)}
					detail={
						failedExecutions > 0
							? t('valFailedExecutions', '{{val}} failed executions', { val: formatNumber(failedExecutions) })
							: t('noFailedExecutions', 'No failed executions')
					}
					icon={
						successRate === null || failedExecutions > 0
							? AlertTriangleIcon
							: CheckCircleIcon
					}
					tone={failedExecutions > 0 ? "warning" : "success"}
				/>
				<MetricCard
					title="Latency"
					value={formatLatency(overview?.avgLatencyMs)}
					detail={
						peakP95Latency === null
							? t('noP95Samples', 'No p95 samples')
							: t('peakDailyP95Val', 'Peak daily p95 {{val}}', { val: formatLatency(peakP95Latency) })
					}
					icon={ClockIcon}
				/>
				<MetricCard
					title={t('aiSpend', 'AI Spend')}
					value={selectedEventId === "all" ? formatCost(totalCost) : "N/A"}
					detail={
						selectedEventId === "all"
							? t('valPerExecution', '{{val}} per execution', { val: formatCost(costPerExecution) })
							: t('eventlevelCostIsNotTrackedYet', 'Event-level cost is not tracked yet')
					}
					icon={BrainIcon}
				/>
			</div>

			<div className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.65fr)]">
				<Card className={analyticsCardClassName}>
					<CardHeader className="pb-3">
						<div className="flex items-start justify-between gap-3">
							<div>
								<CardTitle>{t('runsAndUsers', 'Runs and Users')}</CardTitle>
								<CardDescription>
									{t('dailyExecutionVolumeAndActiveUsers', 'Daily execution volume and active users')}
								</CardDescription>
							</div>
							<Badge variant="outline">
								<TrendingUpIcon className="mr-1 h-3 w-3" />
								{dateRangeLabel}
							</Badge>
						</div>
					</CardHeader>
					<CardContent className="pt-0">
						<ActivityTrendChart data={dailyStats} />
					</CardContent>
				</Card>

				<Card className={analyticsCardClassName}>
					<CardHeader className="pb-3">
						<CardTitle>{t('reliability', 'Reliability')}</CardTitle>
						<CardDescription>{t('successfulAndFailedExecutions', 'Successful and failed executions')}</CardDescription>
					</CardHeader>
					<CardContent className="pt-0">
						<ReliabilityChart data={dailyStats} />
					</CardContent>
				</Card>
			</div>

			<div className="grid gap-4 xl:grid-cols-2">
				<Card className={analyticsCardClassName}>
					<CardHeader className="pb-3">
						<CardTitle>{t('responseTime', 'Response Time')}</CardTitle>
						<CardDescription>
							{t('averageAndDailyP95ExecutionLatency', 'Average and daily p95 execution latency')}
						</CardDescription>
					</CardHeader>
					<CardContent className="pt-0">
						<LatencyChart data={dailyStats} />
					</CardContent>
				</Card>

				<Card className={analyticsCardClassName}>
					<CardHeader className="pb-3">
						<div className="flex items-start justify-between gap-3">
							<div>
								<CardTitle>{t('aiCost', 'AI Cost')}</CardTitle>
								<CardDescription>
									{selectedEventId === "all"
										? t('dailyLlmAndEmbeddingSpend', 'Daily LLM and embedding spend')
										: t('eventlevelAiSpendIsNotTrackedYet', 'Event-level AI spend is not tracked yet')}
								</CardDescription>
							</div>
							<div className="text-right text-xs text-muted-foreground">
								<div>LLM {formatCost(overview?.totalLlmCost ?? 0)}</div>
								<div>
									{t('embeddings', 'Embeddings')} {formatCost(overview?.totalEmbeddingCost ?? 0)}
								</div>
							</div>
						</div>
					</CardHeader>
					<CardContent className="pt-0">
						{selectedEventId === "all" ? (
							<CostTrendChart data={dailyStats} />
						) : (
							<EmptyChart label={t('eventlevelAiSpendIsNotTrackedYet', 'Event-level AI spend is not tracked yet')} />
						)}
					</CardContent>
				</Card>
			</div>

			<div className="grid gap-4 xl:grid-cols-[minmax(320px,0.8fr)_minmax(0,1.2fr)]">
				<Card className={analyticsCardClassName}>
					<CardHeader className="pb-3">
						<div className="flex items-start justify-between gap-3">
							<div>
								<CardTitle>{t('feedbackSignal', 'Feedback Signal')}</CardTitle>
								<CardDescription>{t('positiveAndNegativeResponsesForSelectedeventlabel', 'Positive and negative responses for {{selectedEventLabel}}', { selectedEventLabel })}</CardDescription>
							</div>
							<Badge
								variant={positiveRate === null ? "secondary" : "outline"}
								className={cn(
									positiveRate !== null &&
										positiveRate >= 80 &&
										"text-emerald-600 dark:text-emerald-400",
									positiveRate !== null &&
										positiveRate < 50 &&
										"text-destructive",
								)}
							>
								<MessageSquareIcon className="mr-1 h-3 w-3" />
								{positiveRate === null
									? t('noSignal', 'No signal')
									: t('valPositive', '{{val}} positive', { val: formatPercent(positiveRate) })}
							</Badge>
						</div>
					</CardHeader>
					<CardContent className="pt-0">
						<FeedbackTrendChart data={dailyStats} />
					</CardContent>
				</Card>

				<Card className={analyticsCardClassName}>
					<CardHeader className="pb-3">
						<div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
							<div>
								<CardTitle>{t('recentFeedback', 'Recent Feedback')}</CardTitle>
								<CardDescription>
									{formatNumber(feedbackTotal)}{t('entriesForSelectedeventlabel', 'entries for {{selectedEventLabel}}', { selectedEventLabel })}</CardDescription>
							</div>
							<Select
								value={feedbackFilter}
								onValueChange={(value) => {
									setFeedbackFilter(value as typeof feedbackFilter);
									setFeedbackPage(0);
								}}
							>
								<SelectTrigger className="w-full bg-background/70 sm:w-36 dark:bg-background/30">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="all">{t('all', 'All')}</SelectItem>
									<SelectItem value="positive">{t('positive', 'Positive')}</SelectItem>
									<SelectItem value="negative">{t('negative', 'Negative')}</SelectItem>
								</SelectContent>
							</Select>
						</div>
					</CardHeader>
					<CardContent className="pt-0">
						{feedbackItems.length === 0 ? (
							<div className="flex min-h-[220px] flex-col items-center justify-center rounded-lg border border-dashed border-border/70 bg-muted/20 px-4 text-center text-sm text-muted-foreground">
								<MessageSquareIcon className="mb-3 h-8 w-8" />
								{t('noFeedbackYet', 'No feedback yet')}
							</div>
						) : (
							<div className="space-y-3">
								<div className="overflow-hidden rounded-lg border border-border/70">
									<Table>
										<TableHeader>
											<TableRow>
												<TableHead>{t('signal', 'Signal')}</TableHead>
												<TableHead className="w-80">{t('source3', 'Source')}</TableHead>
												<TableHead>{t('comment', 'Comment')}</TableHead>
												<TableHead className="w-32">{t('date', 'Date')}</TableHead>
											</TableRow>
										</TableHeader>
										<TableBody>
											{feedbackItems.map((item) => {
												const eventFromList = item.eventId
													? eventLookup.get(item.eventId)
													: undefined;
												const pageSource = formatFeedbackPageSource(
													item,
													eventFromList,
												);
												const eventSource = formatFeedbackEventSource(
													item,
													eventFromList,
												);
												const eventHref = getAnalyticsEventHref(
													appId,
													item.eventId,
												);
												const eventTitle = [
													item.eventName || eventFromList?.name,
													item.eventId,
													item.eventRoute || getEventRoute(eventFromList),
													item.eventPageId || getEventPageId(eventFromList),
												]
													.filter(Boolean)
													.join(" | ");

												return (
													<TableRow key={item.id}>
														<TableCell>
															<FeedbackBadge rating={item.rating} />
														</TableCell>
														<TableCell className="max-w-[20rem]">
															<div className="min-w-0 space-y-1">
																<div className="min-w-0 truncate text-sm font-medium">
																	{eventSource && eventHref ? (
																		<a
																			href={eventHref}
																			className="inline-flex max-w-full items-center gap-1 text-primary underline-offset-2 hover:underline"
																			title={eventTitle || eventSource}
																		>
																			<span className="truncate">
																				{eventSource}
																			</span>
																			<ExternalLinkIcon className="h-3 w-3 shrink-0" />
																		</a>
																	) : eventSource ? (
																		<span title={eventTitle || eventSource}>
																			{eventSource}
																		</span>
																	) : (
																		<span className="text-muted-foreground italic">
																			{t('unknownEvent', 'Unknown event')}
																		</span>
																	)}
																</div>
																{pageSource ? (
																	<p
																		className="truncate text-xs text-muted-foreground"
																		title={pageSource}
																	>
																		{pageSource}
																	</p>
																) : (
																	<p className="text-xs text-muted-foreground italic">
																		{t('noPageContext', 'No page context')}
																	</p>
																)}
															</div>
														</TableCell>
														<TableCell className="max-w-sm">
															<p className="truncate">
																{item.comment || (
																	<span className="text-muted-foreground italic">
																		{t('noComment', 'No comment')}
																	</span>
																)}
															</p>
														</TableCell>
														<TableCell className="whitespace-nowrap text-muted-foreground">
															{formatDateTime(item.createdAt)}
														</TableCell>
													</TableRow>
												);
											})}
										</TableBody>
									</Table>
								</div>

								{feedbackTotalPages > 1 && (
									<div className="flex items-center justify-end gap-2">
										<Button
											variant="outline"
											size="sm"
											disabled={feedbackPage === 0}
											onClick={() => setFeedbackPage((page) => page - 1)}
										>
											{t('previous', 'Previous')}
										</Button>
										<span className="text-sm text-muted-foreground">
											{t('page', 'Page')} {feedbackPage + 1}{t('ofFeedbacktotalpages', 'of {{feedbackTotalPages}}', { feedbackTotalPages })}</span>
										<Button
											variant="outline"
											size="sm"
											disabled={feedbackPage >= feedbackTotalPages - 1}
											onClick={() => setFeedbackPage((page) => page + 1)}
										>
											{t('next', 'Next')}
										</Button>
									</div>
								)}
							</div>
						)}
					</CardContent>
				</Card>
			</div>
		</div>
	);
}
