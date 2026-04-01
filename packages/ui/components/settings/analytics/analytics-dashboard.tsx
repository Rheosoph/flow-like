"use client";

import { ResponsiveBar } from "@nivo/bar";
import { ResponsiveLine } from "@nivo/line";
import { ResponsivePie } from "@nivo/pie";
import {
	ActivityIcon,
	ArrowDownIcon,
	ArrowUpIcon,
	BrainIcon,
	CheckCircleIcon,
	ClockIcon,
	MessageSquareIcon,
	StarIcon,
	ThumbsDownIcon,
	ThumbsUpIcon,
	UsersIcon,
	XCircleIcon,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

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
import { Separator } from "../../ui/separator";
import { Skeleton } from "../../ui/skeleton";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";

function formatPercent(value: number | null): string {
	if (value === null) return "N/A";
	return `${value >= 0 ? "+" : ""}${value.toFixed(1)}%`;
}

function formatDate(dateStr: string): string {
	return new Date(dateStr).toLocaleDateString("en-US", {
		month: "short",
		day: "numeric",
		year: "numeric",
	});
}

function formatLatency(ms: number | null): string {
	if (ms === null) return "N/A";
	if (ms < 1000) return `${Math.round(ms)}ms`;
	return `${(ms / 1000).toFixed(1)}s`;
}

function formatCost(cost: number): string {
	if (cost === 0) return "$0.00";
	if (cost < 0.01) return `$${cost.toFixed(4)}`;
	return `$${cost.toFixed(2)}`;
}

const analyticsCardClassName =
	"border-border/60 bg-card/90 shadow-sm shadow-black/5 dark:bg-card/70 dark:shadow-black/20";

const analyticsInsetClassName =
	"rounded-xl border border-border/50 bg-background/40 dark:bg-background/20";

const nivoTheme = {
	axis: {
		ticks: { text: { fill: "hsl(var(--muted-foreground))" } },
		legend: { text: { fill: "hsl(var(--muted-foreground))" } },
	},
	grid: { line: { stroke: "hsl(var(--border))" } },
	legends: { text: { fill: "hsl(var(--muted-foreground))" } },
	crosshair: { line: { stroke: "hsl(var(--primary))" } },
	labels: { text: { fill: "hsl(var(--foreground))" } },
	tooltip: {
		container: {
			background: "hsl(var(--popover))",
			color: "hsl(var(--popover-foreground))",
			borderRadius: "8px",
			boxShadow: "0 4px 12px hsl(var(--foreground) / 0.1)",
			border: "1px solid hsl(var(--border))",
			fontSize: "12px",
		},
	},
};

const chartColors = {
	executions: ["hsl(217, 91%, 60%)", "hsl(262, 83%, 58%)"],
	latency: ["hsl(217, 91%, 60%)"],
	feedback: ["hsl(142, 71%, 45%)", "hsl(0, 84%, 60%)"],
	cost: { llm: "hsl(217, 91%, 60%)", embedding: "hsl(142, 71%, 45%)" },
};

function StatCard({
	title,
	value,
	change,
	icon: Icon,
	subtitle,
}: {
	title: string;
	value: string;
	change?: number | null;
	icon: React.ComponentType<{ className?: string }>;
	subtitle?: string;
}) {
	return (
		<Card className={analyticsCardClassName}>
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium">{title}</CardTitle>
				<Icon className="h-4 w-4 text-muted-foreground" />
			</CardHeader>
			<CardContent>
				<div className="text-2xl font-bold">{value}</div>
				{change !== undefined && change !== null && (
					<p
						className={`text-xs ${change >= 0 ? "text-green-600" : "text-red-600"} flex items-center gap-1`}
					>
						{change >= 0 ? (
							<ArrowUpIcon className="h-3 w-3" />
						) : (
							<ArrowDownIcon className="h-3 w-3" />
						)}
						{formatPercent(change)} from last period
					</p>
				)}
				{subtitle && (
					<p className="text-xs text-muted-foreground mt-1">{subtitle}</p>
				)}
			</CardContent>
		</Card>
	);
}

function ExecutionsChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const chartData = useMemo(
		() =>
			data.map((d) => ({
				date: new Date(d.date).toLocaleDateString("en-US", {
					month: "short",
					day: "numeric",
				}),
				executions: d.executions,
				uniqueUsers: d.uniqueUsers,
			})),
		[data],
	);

	if (data.length === 0) {
		return (
			<div className="flex h-75 items-center justify-center text-muted-foreground">
				No execution data available
			</div>
		);
	}

	return (
		<div className="h-75">
			<ResponsiveBar
				data={chartData}
				keys={["executions", "uniqueUsers"]}
				indexBy="date"
				margin={{ top: 20, right: 100, bottom: 50, left: 60 }}
				padding={0.3}
				groupMode="grouped"
				colors={chartColors.executions}
				axisBottom={{
					tickRotation: -45,
					legend: "Date",
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: "Count",
					legendOffset: -50,
					legendPosition: "middle",
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "bottom-right",
						direction: "column",
						translateX: 100,
						itemWidth: 80,
						itemHeight: 20,
						itemTextColor: "hsl(var(--muted-foreground))",
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function LatencyChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const chartData = useMemo(
		() => [
			{
				id: "Avg Latency",
				data: data
					.filter((d) => d.avgLatency !== null)
					.map((d) => ({
						x: new Date(d.date).toLocaleDateString("en-US", {
							month: "short",
							day: "numeric",
						}),
						y: d.avgLatency,
					})),
			},
		],
		[data],
	);

	if (chartData[0].data.length === 0) {
		return (
			<div className="flex h-75 items-center justify-center text-muted-foreground">
				No latency data available
			</div>
		);
	}

	return (
		<div className="h-75">
			<ResponsiveLine
				data={chartData}
				margin={{ top: 20, right: 20, bottom: 50, left: 60 }}
				xScale={{ type: "point" }}
				yScale={{ type: "linear", min: 0, max: "auto" }}
				axisBottom={{
					tickRotation: -45,
					legend: "Date",
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: "Latency (ms)",
					legendOffset: -50,
					legendPosition: "middle",
					format: (v) => `${v}ms`,
				}}
				colors={chartColors.latency}
				pointSize={8}
				pointBorderWidth={2}
				pointBorderColor={{ from: "serieColor" }}
				enableArea={true}
				areaOpacity={0.1}
				useMesh={true}
				enableGridX={false}
				theme={nivoTheme}
			/>
		</div>
	);
}

function FeedbackChart({ data }: { data: IDailyAnalyticsStat[] }) {
	const chartData = useMemo(
		() =>
			data
				.filter(
					(d) =>
						d.feedbackCount > 0 ||
						d.positiveFeedback > 0 ||
						d.negativeFeedback > 0,
				)
				.map((d) => ({
					date: new Date(d.date).toLocaleDateString("en-US", {
						month: "short",
						day: "numeric",
					}),
					positive: d.positiveFeedback,
					negative: d.negativeFeedback,
				})),
		[data],
	);

	if (chartData.length === 0) {
		return (
			<div className="flex h-75 items-center justify-center text-muted-foreground">
				No feedback data available
			</div>
		);
	}

	return (
		<div className="h-75">
			<ResponsiveBar
				data={chartData}
				keys={["positive", "negative"]}
				indexBy="date"
				margin={{ top: 20, right: 100, bottom: 50, left: 60 }}
				padding={0.3}
				groupMode="stacked"
				colors={chartColors.feedback}
				axisBottom={{
					tickRotation: -45,
					legend: "Date",
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: "Feedback",
					legendOffset: -50,
					legendPosition: "middle",
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "bottom-right",
						direction: "column",
						translateX: 100,
						itemWidth: 80,
						itemHeight: 20,
						itemTextColor: "hsl(var(--muted-foreground))",
					},
				]}
				theme={nivoTheme}
			/>
		</div>
	);
}

function CostBreakdownChart({ overview }: { overview: IAnalyticsOverview }) {
	const chartData = useMemo(
		() =>
			[
				{
					id: "LLM Cost",
					value: overview.totalLlmCost,
					color: chartColors.cost.llm,
				},
				{
					id: "Embedding Cost",
					value: overview.totalEmbeddingCost,
					color: chartColors.cost.embedding,
				},
			].filter((d) => d.value > 0),
		[overview],
	);

	if (chartData.length === 0) {
		return (
			<div className="flex h-62.5 items-center justify-center text-muted-foreground">
				No cost data available
			</div>
		);
	}

	return (
		<div className="h-62.5">
			<ResponsivePie
				data={chartData}
				margin={{ top: 20, right: 80, bottom: 20, left: 80 }}
				innerRadius={0.5}
				padAngle={0.7}
				cornerRadius={3}
				activeOuterRadiusOffset={8}
				colors={{ datum: "data.color" }}
				arcLinkLabelsSkipAngle={10}
				arcLinkLabelsTextColor="hsl(var(--muted-foreground))"
				arcLinkLabelsThickness={2}
				arcLabelsSkipAngle={10}
				arcLabelsTextColor="white"
				valueFormat={(v) => formatCost(v)}
				theme={nivoTheme}
			/>
		</div>
	);
}

function RatingBadge({ rating }: { rating: number }) {
	if (rating >= 4) {
		return (
			<Badge className="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200">
				<ThumbsUpIcon className="h-3 w-3 mr-1" />
				{rating}/5
			</Badge>
		);
	}
	if (rating >= 3) {
		return (
			<Badge variant="secondary">
				<StarIcon className="h-3 w-3 mr-1" />
				{rating}/5
			</Badge>
		);
	}
	return (
		<Badge className="bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200">
			<ThumbsDownIcon className="h-3 w-3 mr-1" />
			{rating}/5
		</Badge>
	);
}

export function AnalyticsDashboard() {
	const backend = useBackend();
	const analyticsState = backend.analyticsState;
	const searchParams = useSearchParams();
	const appId = searchParams.get("id");

	const [loading, setLoading] = useState(true);
	const [overview, setOverview] = useState<IAnalyticsOverview | null>(null);
	const [dailyStats, setDailyStats] = useState<IDailyAnalyticsStat[]>([]);
	const [feedbackItems, setFeedbackItems] = useState<IFeedbackItem[]>([]);
	const [feedbackTotal, setFeedbackTotal] = useState(0);
	const [feedbackPage, setFeedbackPage] = useState(0);

	const [dateRange, setDateRange] = useState<"7d" | "30d" | "90d">("30d");
	const [feedbackFilter, setFeedbackFilter] = useState<
		"all" | "positive" | "negative"
	>("all");

	const FEEDBACK_LIMIT = 20;

	const loadData = useCallback(async () => {
		if (!appId || !analyticsState) return;

		setLoading(true);
		try {
			const days = dateRange === "7d" ? 7 : dateRange === "30d" ? 30 : 90;
			const endDate = new Date().toISOString().split("T")[0];
			const startDate = new Date(Date.now() - days * 24 * 60 * 60 * 1000)
				.toISOString()
				.split("T")[0];

			const minRating =
				feedbackFilter === "positive"
					? 4
					: feedbackFilter === "negative"
						? undefined
						: undefined;
			const maxRating =
				feedbackFilter === "negative"
					? 2
					: feedbackFilter === "positive"
						? undefined
						: undefined;

			const [dashboardData, feedbackData] = await Promise.all([
				analyticsState.getAnalyticsDashboard(appId, startDate, endDate),
				analyticsState.listFeedback(
					appId,
					feedbackPage * FEEDBACK_LIMIT,
					FEEDBACK_LIMIT,
					minRating,
					maxRating,
				),
			]);

			setOverview(dashboardData.overview);
			setDailyStats(dashboardData.stats.dailyStats);
			setFeedbackItems(feedbackData.items);
			setFeedbackTotal(feedbackData.total);
		} catch (error) {
			toast.error(
				error instanceof Error
					? `Failed to load analytics: ${error.message}`
					: "Failed to load analytics data",
			);
		} finally {
			setLoading(false);
		}
	}, [appId, dateRange, feedbackFilter, feedbackPage, analyticsState]);

	useEffect(() => {
		loadData();
	}, [loadData]);

	const feedbackTotalPages = Math.ceil(feedbackTotal / FEEDBACK_LIMIT);

	if (!analyticsState) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">
					Analytics is not available
				</p>
			</div>
		);
	}

	if (!appId) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">No app selected</p>
			</div>
		);
	}

	if (loading) {
		return (
			<div className="p-6 space-y-6">
				<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
					{Array.from({ length: 4 }).map((_, i) => (
						<Skeleton key={`skeleton-stat-${i}`} className="h-32" />
					))}
				</div>
				<Skeleton className="h-75" />
			</div>
		);
	}

	return (
		<div className="space-y-6 text-foreground">
				{/* Header */}
				<div className="flex items-center justify-between">
					<div>
						<h1 className="text-2xl font-bold">Analytics Dashboard</h1>
						<p className="text-muted-foreground">
							Track your app&apos;s usage, performance, and feedback
						</p>
					</div>
					<Select
						value={dateRange}
						onValueChange={(v) => setDateRange(v as typeof dateRange)}
					>
						<SelectTrigger className="w-32 bg-background/70 dark:bg-background/30">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="7d">Last 7 days</SelectItem>
							<SelectItem value="30d">Last 30 days</SelectItem>
							<SelectItem value="90d">Last 90 days</SelectItem>
						</SelectContent>
					</Select>
				</div>

				{/* Stats Cards */}
				<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
					<StatCard
						title="Total Executions"
						value={(overview?.totalExecutions ?? 0).toLocaleString()}
						change={overview?.executionsChangePercent}
						icon={ActivityIcon}
						subtitle={`${overview?.successfulExecutions ?? 0} successful, ${overview?.failedExecutions ?? 0} failed`}
					/>
					<StatCard
						title="Unique Users"
						value={(overview?.uniqueUsers ?? 0).toLocaleString()}
						change={overview?.usersChangePercent}
						icon={UsersIcon}
						subtitle={`${overview?.periodUniqueUsers ?? 0} in selected period`}
					/>
					<StatCard
						title="Avg Rating"
						value={
							overview?.avgFeedbackRating !== null &&
							overview?.avgFeedbackRating !== undefined
								? `${overview.avgFeedbackRating.toFixed(1)}/5`
								: "N/A"
						}
						icon={StarIcon}
						subtitle={`${overview?.totalFeedback ?? 0} total ratings`}
					/>
					<StatCard
						title="Avg Latency"
						value={formatLatency(overview?.avgLatencyMs ?? null)}
						icon={ClockIcon}
					/>
				</div>

				{/* Secondary Stats */}
				<div className="grid gap-4 md:grid-cols-3">
					<Card className={analyticsCardClassName}>
						<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
							<CardTitle className="text-sm font-medium">
								Feedback Sentiment
							</CardTitle>
							<MessageSquareIcon className="h-4 w-4 text-muted-foreground" />
						</CardHeader>
						<CardContent>
							<div className="flex items-center gap-4">
								<div className="flex items-center gap-1 text-green-600">
									<ThumbsUpIcon className="h-4 w-4" />
									<span className="text-lg font-semibold">
										{overview?.positiveFeedback ?? 0}
									</span>
								</div>
								<div className="flex items-center gap-1 text-red-600">
									<ThumbsDownIcon className="h-4 w-4" />
									<span className="text-lg font-semibold">
										{overview?.negativeFeedback ?? 0}
									</span>
								</div>
							</div>
							{overview && overview.totalFeedback > 0 && (
								<div className="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted/80 dark:bg-muted/50">
									<div
										className="h-full bg-green-600 dark:bg-green-500"
										style={{
											width: `${(overview.positiveFeedback / overview.totalFeedback) * 100}%`,
										}}
									/>
								</div>
							)}
						</CardContent>
					</Card>

					<Card className={analyticsCardClassName}>
						<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
							<CardTitle className="text-sm font-medium">
								Success Rate
							</CardTitle>
							<CheckCircleIcon className="h-4 w-4 text-muted-foreground" />
						</CardHeader>
						<CardContent>
							<div className="text-2xl font-bold">
								{overview && overview.totalExecutions > 0
									? `${((overview.successfulExecutions / overview.totalExecutions) * 100).toFixed(1)}%`
									: "N/A"}
							</div>
							<div className="flex items-center gap-2 mt-1 text-xs text-muted-foreground">
								<span className="flex items-center gap-1 text-green-600">
									<CheckCircleIcon className="h-3 w-3" />
									{overview?.successfulExecutions ?? 0}
								</span>
								<span className="flex items-center gap-1 text-red-600">
									<XCircleIcon className="h-3 w-3" />
									{overview?.failedExecutions ?? 0}
								</span>
							</div>
						</CardContent>
					</Card>

					<Card className={analyticsCardClassName}>
						<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
							<CardTitle className="text-sm font-medium">AI Costs</CardTitle>
							<BrainIcon className="h-4 w-4 text-muted-foreground" />
						</CardHeader>
						<CardContent>
							<div className="text-2xl font-bold">
								{formatCost(
									(overview?.totalLlmCost ?? 0) +
										(overview?.totalEmbeddingCost ?? 0),
								)}
							</div>
							<div className="flex items-center gap-2 mt-1 text-xs text-muted-foreground">
								<span>LLM: {formatCost(overview?.totalLlmCost ?? 0)}</span>
								<span>
									Embed: {formatCost(overview?.totalEmbeddingCost ?? 0)}
								</span>
							</div>
						</CardContent>
					</Card>
				</div>

				{/* Charts */}
				<Tabs defaultValue="executions" className="space-y-4">
					<TabsList className="bg-muted/80 dark:bg-muted/40">
						<TabsTrigger value="executions">Executions</TabsTrigger>
						<TabsTrigger value="latency">Latency</TabsTrigger>
						<TabsTrigger value="feedback">Feedback</TabsTrigger>
						<TabsTrigger value="costs">Costs</TabsTrigger>
					</TabsList>

					<TabsContent value="executions">
						<Card className={analyticsCardClassName}>
							<CardHeader>
								<CardTitle>Executions Over Time</CardTitle>
								<CardDescription>
									Daily executions and unique users
								</CardDescription>
							</CardHeader>
							<CardContent className={analyticsInsetClassName}>
								<ExecutionsChart data={dailyStats} />
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="latency">
						<Card className={analyticsCardClassName}>
							<CardHeader>
								<CardTitle>Average Latency</CardTitle>
								<CardDescription>
									Response time trends over the selected period
								</CardDescription>
							</CardHeader>
							<CardContent className={analyticsInsetClassName}>
								<LatencyChart data={dailyStats} />
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="feedback">
						<Card className={analyticsCardClassName}>
							<CardHeader>
								<CardTitle>Feedback Over Time</CardTitle>
								<CardDescription>
									Positive vs negative feedback distribution
								</CardDescription>
							</CardHeader>
							<CardContent className={analyticsInsetClassName}>
								<FeedbackChart data={dailyStats} />
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="costs">
						<Card className={analyticsCardClassName}>
							<CardHeader>
								<CardTitle>Cost Breakdown</CardTitle>
								<CardDescription>
									LLM and embedding costs for the selected period
								</CardDescription>
							</CardHeader>
							<CardContent className={analyticsInsetClassName}>
								{overview && <CostBreakdownChart overview={overview} />}
							</CardContent>
						</Card>
					</TabsContent>
				</Tabs>

				<Separator />

				{/* Feedback List */}
				<div className="space-y-4">
					<div className="flex items-center justify-between">
						<div>
							<h2 className="text-xl font-semibold">Recent Feedback</h2>
							<p className="text-sm text-muted-foreground">
								{feedbackTotal} total feedback entries
							</p>
						</div>
						<Select
							value={feedbackFilter}
							onValueChange={(v) => {
								setFeedbackFilter(v as typeof feedbackFilter);
								setFeedbackPage(0);
							}}
						>
							<SelectTrigger className="w-32 bg-background/70 dark:bg-background/30">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="all">All</SelectItem>
								<SelectItem value="positive">Positive</SelectItem>
								<SelectItem value="negative">Negative</SelectItem>
							</SelectContent>
						</Select>
					</div>

					{feedbackItems.length === 0 ? (
						<Card className={analyticsCardClassName}>
							<CardContent className="flex flex-col items-center justify-center py-12">
								<MessageSquareIcon className="h-12 w-12 text-muted-foreground mb-4" />
								<p className="text-muted-foreground">No feedback yet</p>
							</CardContent>
						</Card>
					) : (
						<>
							<Card className={analyticsCardClassName}>
								<Table>
									<TableHeader>
										<TableRow>
											<TableHead>Rating</TableHead>
											<TableHead>Comment</TableHead>
											<TableHead>Date</TableHead>
										</TableRow>
									</TableHeader>
									<TableBody>
										{feedbackItems.map((item) => (
											<TableRow key={item.id}>
												<TableCell>
													<RatingBadge rating={item.rating} />
												</TableCell>
												<TableCell className="max-w-md">
													<p className="truncate">
														{item.comment || (
															<span className="text-muted-foreground italic">
																No comment
															</span>
														)}
													</p>
												</TableCell>
												<TableCell className="whitespace-nowrap">
													{formatDate(item.createdAt)}
												</TableCell>
											</TableRow>
										))}
									</TableBody>
								</Table>
							</Card>

							{feedbackTotalPages > 1 && (
								<div className="flex items-center justify-center gap-2">
									<Button
										variant="outline"
										size="sm"
										disabled={feedbackPage === 0}
										onClick={() => setFeedbackPage((p) => p - 1)}
									>
										Previous
									</Button>
									<span className="text-sm text-muted-foreground">
										Page {feedbackPage + 1} of {feedbackTotalPages}
									</span>
									<Button
										variant="outline"
										size="sm"
										disabled={feedbackPage >= feedbackTotalPages - 1}
										onClick={() => setFeedbackPage((p) => p + 1)}
									>
										Next
									</Button>
								</div>
							)}
						</>
					)}
				</div>
		</div>
	);
}
