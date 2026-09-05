"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle, RefreshCw, Save, Scale, Shield } from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { Bar, BarChart, CartesianGrid, Cell, ComposedChart, Line, Pie, PieChart, XAxis, YAxis } from "recharts";
import { useFeatures } from "../../../hooks/use-features";
import type { IAdminAppUsage, IAdminPaginated, IAdminTechnicalUserUsage, IAdminUsageAlert, IAdminUsageInvocation, IAdminUsageOverview, IAppUsageLimits, IUsageLimitPeriod, IUsageReconciliationResult } from "../../../lib/schema/usage";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../ui/card";
import { type ChartConfig, ChartContainer, ChartTooltip, ChartTooltipContent } from "../../ui/chart";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { Skeleton } from "../../ui/skeleton";

const PERIODS: IUsageLimitPeriod[] = ["weekly", "monthly", "yearly"];

const activityChartConfig = {
	aiCalls: {
		label: "AI calls",
		color: "var(--chart-1)",
	},
	executions: {
		label: "Executions",
		color: "var(--chart-2)",
	},
	activeUsers: {
		label: "Active users",
		color: "var(--chart-3)",
	},
	newUsers: {
		label: "New users",
		color: "var(--chart-4)",
	},
} satisfies ChartConfig;

const spendChartConfig = {
	costDollars: {
		label: "Cost",
		color: "var(--chart-1)",
	},
	tokens: {
		label: "Tokens",
		color: "var(--chart-5)",
	},
} satisfies ChartConfig;

const appActivityChartConfig = {
	aiCalls: {
		label: "AI calls",
		color: "var(--chart-1)",
	},
	executions: {
		label: "Executions",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

const modelChartConfig = {
	invocations: {
		label: "Calls",
		color: "var(--chart-3)",
	},
	tokens: {
		label: "Tokens",
		color: "var(--chart-5)",
	},
} satisfies ChartConfig;

const spendMixChartConfig = {
	llm: {
		label: "LLM",
		color: "var(--chart-1)",
	},
	embedding: {
		label: "Embeddings",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

function emptyLimits(): IAppUsageLimits {
	const window = {
		costMicroDollars: null,
		tokenLimit: null,
		enabled: true,
		hard: true,
		warningThresholdPercent: 80,
	};
	return {
		weekly: { ...window },
		monthly: { ...window },
		yearly: { ...window },
	};
}

function formatCost(microDollars: number) {
	return new Intl.NumberFormat(undefined, {
		style: "currency",
		currency: "USD",
		maximumFractionDigits: 4,
	}).format(microDollars / 1_000_000);
}

function formatDollars(value: number | null) {
	if (value === null) return "n/a";
	return new Intl.NumberFormat(undefined, {
		style: "currency",
		currency: "USD",
		maximumFractionDigits: 4,
	}).format(value);
}

function formatCount(value: number) {
	return new Intl.NumberFormat().format(value);
}

function formatCompactCount(value: number) {
	return new Intl.NumberFormat(undefined, {
		notation: "compact",
		maximumFractionDigits: 1,
	}).format(value);
}

function formatDuration(ms: number | null) {
	if (ms === null) return "n/a";
	if (ms < 1000)
		return i18next.t("valMs", "{{val}} ms", { val: Math.round(ms) });
	return `${(ms / 1000).toFixed(1)} s`;
}

function formatPercent(value: number) {
	if (!Number.isFinite(value)) return "0%";
	return `${Math.round(value)}%`;
}

function chartTick(value: string | number) {
	const numeric = Number(value);
	return Number.isFinite(numeric) ? formatCompactCount(numeric) : String(value);
}

function currencyTick(value: string | number) {
	const numeric = Number(value);
	if (!Number.isFinite(numeric)) return String(value);
	return `$${formatCompactCount(numeric)}`;
}

function dollarValue(microDollars: number) {
	return microDollars / 1_000_000;
}

function truncateChartLabel(value: string, maxLength = 18) {
	return value.length > maxLength
		? `${value.slice(0, maxLength - 3)}...`
		: value;
}

function periodTitle(period: IUsageLimitPeriod) {
	return period[0].toUpperCase() + period.slice(1);
}

function costInputValue(
	limits: IAppUsageLimits | null,
	period: IUsageLimitPeriod,
) {
	const value = limits?.[period]?.costMicroDollars;
	return value == null ? "" : String(value / 1_000_000);
}

function tokenInputValue(
	limits: IAppUsageLimits | null,
	period: IUsageLimitPeriod,
) {
	const value = limits?.[period]?.tokenLimit;
	return value == null ? "" : String(value);
}

function toMicroDollars(value: string) {
	const parsed = Number(value);
	if (!Number.isFinite(parsed) || parsed < 0) return null;
	return Math.round(parsed * 1_000_000);
}

function toTokenLimit(value: string) {
	const parsed = Number(value);
	if (!Number.isFinite(parsed) || parsed < 0) return null;
	return Math.floor(parsed);
}

function EmptyChart({ label }: { label: string }) {
	return (
		<div className="flex h-64 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
			{label}
		</div>
	);
}

function UsageHealthSummary({
	overview,
	loading,
	period,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
	period: IUsageLimitPeriod;
}) {
	const { t } = useTranslation("admin");
	const stats = overview?.userStats;
	const totals = overview?.totals;

	const groups = [
		{
			title: t("audience", "Audience"),
			items: [
				["DAU", formatCount(stats?.activeUsersDaily ?? 0)],
				["WAU", formatCount(stats?.activeUsersWeekly ?? 0)],
				["MAU", formatCount(stats?.activeUsersMonthly ?? 0)],
			],
		},
		{
			title: t("growth", "Growth"),
			items: [
				["Today", formatCount(stats?.newUsersToday ?? 0)],
				["7d", formatCount(stats?.newUsersWeekly ?? 0)],
				["30d", formatCount(stats?.newUsersMonthly ?? 0)],
			],
		},
		{
			title: t("workload", "Workload"),
			items: [
				[
					"AI",
					formatCount(
						(totals?.llmInvocations ?? 0) + (totals?.embeddingInvocations ?? 0),
					),
				],
				["Runs", formatCount(totals?.executions ?? 0)],
				["Apps", formatCount(stats?.activeAppsMonthly ?? 0)],
			],
		},
		{
			title: t("efficiency", "Efficiency"),
			items: [
				["Spend", formatCost(totals?.totalPrice ?? 0)],
				["/ MAU", formatDollars(stats?.averageCostPerActiveUser ?? null)],
				["Runtime", formatDuration(totals?.averageExecutionMs ?? null)],
			],
		},
	];

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{periodTitle(period)} {t("health", "Health")}
				</CardTitle>
				<CardDescription>
					{t(
						"usersGrowthWorkloadAndSpendInOneOperationalView",
						"Users, growth, workload, and spend in one operational view.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-64 w-full" />
				) : (
					<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-1">
						{groups.map((group) => (
							<div key={group.title} className="rounded-lg border p-3">
								<div className="mb-3 text-sm font-medium">{group.title}</div>
								<div className="grid grid-cols-3 gap-2">
									{group.items.map(([label, value]) => (
										<div key={label} className="min-w-0">
											<div className="truncate text-[11px] text-muted-foreground">
												{label}
											</div>
											<div className="truncate text-sm font-semibold">
												{value}
											</div>
										</div>
									))}
								</div>
							</div>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function ActivityTrendChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() =>
			overview?.trend.map((point) => ({
				label: point.label,
				aiCalls: point.aiInvocations,
				executions: point.executions,
				activeUsers: point.activeUsers,
				newUsers: point.newUsers,
			})) ?? [],
		[overview?.trend],
	);
	const hasData = data.some(
		(point) =>
			point.aiCalls || point.executions || point.activeUsers || point.newUsers,
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("activityOverTime", "Activity Over Time")}
				</CardTitle>
				<CardDescription>
					{t(
						"aiCallsAppExecutionsActiveUsersAndSignups",
						"AI calls, app executions, active users, and signups.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-80 w-full" />
				) : !hasData ? (
					<EmptyChart
						label={t(
							"noActivityRecordedForThisPeriod",
							"No activity recorded for this period.",
						)}
					/>
				) : (
					<ChartContainer config={activityChartConfig} className="h-80 w-full">
						<ComposedChart
							data={data}
							margin={{ top: 12, right: 12, left: 0, bottom: 0 }}
						>
							<CartesianGrid strokeDasharray="3 3" vertical={false} />
							<XAxis
								dataKey="label"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								minTickGap={24}
							/>
							<YAxis
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								width={36}
								tickFormatter={chartTick}
								allowDecimals={false}
							/>
							<ChartTooltip
								cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<Bar
								dataKey="aiCalls"
								fill="var(--color-aiCalls)"
								radius={[3, 3, 0, 0]}
							/>
							<Bar
								dataKey="executions"
								fill="var(--color-executions)"
								radius={[3, 3, 0, 0]}
							/>
							<Line
								type="monotone"
								dataKey="activeUsers"
								stroke="var(--color-activeUsers)"
								strokeWidth={2}
								dot={false}
							/>
							<Line
								type="monotone"
								dataKey="newUsers"
								stroke="var(--color-newUsers)"
								strokeWidth={2}
								dot={false}
							/>
						</ComposedChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function SpendTokenTrendChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() =>
			overview?.trend.map((point) => ({
				label: point.label,
				costDollars: dollarValue(point.cost),
				tokens: point.tokens,
			})) ?? [],
		[overview?.trend],
	);
	const hasData = data.some((point) => point.costDollars || point.tokens);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("spendTokens", "Spend & Tokens")}
				</CardTitle>
				<CardDescription>
					{t(
						"remoteModelCostAgainstTokenVolume",
						"Remote model cost against token volume.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : !hasData ? (
					<EmptyChart
						label={t(
							"noRemoteModelSpendForThisPeriod",
							"No remote model spend for this period.",
						)}
					/>
				) : (
					<ChartContainer config={spendChartConfig} className="h-72 w-full">
						<ComposedChart
							data={data}
							margin={{ top: 12, right: 8, left: 0, bottom: 0 }}
						>
							<CartesianGrid strokeDasharray="3 3" vertical={false} />
							<XAxis
								dataKey="label"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								minTickGap={24}
							/>
							<YAxis
								yAxisId="cost"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								width={42}
								tickFormatter={currencyTick}
							/>
							<YAxis
								yAxisId="tokens"
								orientation="right"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								width={42}
								tickFormatter={chartTick}
							/>
							<ChartTooltip
								cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<Bar
								yAxisId="tokens"
								dataKey="tokens"
								fill="var(--color-tokens)"
								radius={[3, 3, 0, 0]}
							/>
							<Line
								yAxisId="cost"
								type="monotone"
								dataKey="costDollars"
								stroke="var(--color-costDollars)"
								strokeWidth={2}
								dot={false}
							/>
						</ComposedChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function SpendMixChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() => [
			{
				key: "llm",
				name: "LLM",
				value: overview?.totals.llmPrice ?? 0,
			},
			{
				key: "embedding",
				name: t("embeddings", "Embeddings"),
				value: overview?.totals.embeddingPrice ?? 0,
			},
		],
		[overview?.totals.embeddingPrice, overview?.totals.llmPrice],
	);
	const total = data.reduce((sum, item) => sum + item.value, 0);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("spendMix", "Spend Mix")}
				</CardTitle>
				<CardDescription>
					{t("llmCostVersusEmbeddingCost", "LLM cost versus embedding cost.")}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : total === 0 ? (
					<EmptyChart
						label={t("noPaidRemoteUsageYet", "No paid remote usage yet.")}
					/>
				) : (
					<div className="grid gap-4 md:grid-cols-[1fr_auto] lg:grid-cols-1 xl:grid-cols-[1fr_auto]">
						<ChartContainer
							config={spendMixChartConfig}
							className="h-56 w-full"
						>
							<PieChart>
								<ChartTooltip
									content={<ChartTooltipContent nameKey="key" hideLabel />}
								/>
								<Pie
									data={data}
									dataKey="value"
									nameKey="key"
									innerRadius={54}
									outerRadius={86}
									paddingAngle={2}
								>
									{data.map((entry) => (
										<Cell key={entry.key} fill={`var(--color-${entry.key})`} />
									))}
								</Pie>
							</PieChart>
						</ChartContainer>
						<div className="space-y-3 text-sm">
							{data.map((item) => (
								<div key={item.key} className="min-w-36">
									<div className="flex items-center justify-between gap-3">
										<div className="flex items-center gap-2">
											<span
												className="h-2.5 w-2.5 rounded-sm"
												style={{ background: `var(--color-${item.key})` }}
											/>
											<span>{item.name}</span>
										</div>
										<span className="font-medium">
											{formatCost(item.value)}
										</span>
									</div>
									<div className="text-xs text-muted-foreground">
										{formatPercent((item.value / total) * 100)}
									</div>
								</div>
							))}
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function TechnicalUsers({
	overview,
	loading,
	profile,
	period,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
	profile: IProfile | undefined;
	period: IUsageLimitPeriod;
}) {
	const { t } = useTranslation("admin");
	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("technicalUsers", "Technical Users")}
				</CardTitle>
				<CardDescription>
					{t(
						"apiKeysWithTheHighestTrackedUsage",
						"API keys with the highest tracked usage.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-2">
				{loading && <Skeleton className="h-36 w-full" />}
				{overview?.technicalUsers.map((technicalUser) => {
					return (
						<div
							key={technicalUser.technicalUserId}
							className="space-y-3 rounded-md border p-3 text-sm"
						>
							<div className="grid grid-cols-[1fr_auto] gap-3">
								<div className="min-w-0">
									<div className="truncate font-medium">
										{technicalUser.name ?? technicalUser.technicalUserId}
									</div>
									<div className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
										<UserProfileLink
											userId={technicalUser.creatorUserId}
											name={technicalUser.creatorDisplayName}
											email={technicalUser.creatorEmail}
											fallbackLabel="Unknown owner"
											className="min-w-0"
											muted
										/>
										{(technicalUser.appName || technicalUser.appId) && (
											<>
												<span className="shrink-0">-</span>
												<span className="truncate">
													{technicalUser.appName ?? technicalUser.appId}
												</span>
											</>
										)}
									</div>
								</div>
								<div className="text-right">
									<div className="font-medium">
										{formatCost(technicalUser.totalPrice)}
									</div>
									<div className="text-xs text-muted-foreground">
										{formatCount(technicalUser.totalTokens)} tokens
									</div>
								</div>
							</div>
							{profile && technicalUser.appId && (
								<TechnicalUserLimitEditor
									technicalUser={technicalUser}
									period={period}
									profile={profile}
								/>
							)}
						</div>
					);
				})}
				{overview && overview.technicalUsers.length === 0 && (
					<div className="rounded-md border p-4 text-sm text-muted-foreground">
						{t(
							"noApikeyUsageForThisPeriod",
							"No API-key usage for this period.",
						)}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function TechnicalUserLimitEditor({
	technicalUser,
	period,
	profile,
}: {
	technicalUser: IAdminTechnicalUserUsage;
	period: IUsageLimitPeriod;
	profile: IProfile;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [cost, setCost] = useState(
		costInputValue(technicalUser.limits, period),
	);
	const [tokens, setTokens] = useState(
		tokenInputValue(technicalUser.limits, period),
	);

	useEffect(() => {
		setCost(costInputValue(technicalUser.limits, period));
		setTokens(tokenInputValue(technicalUser.limits, period));
	}, [technicalUser.limits, period]);

	const mutation = useMutation({
		mutationFn: async () => {
			if (!technicalUser.appId) throw new Error("Missing app id");
			const next = technicalUser.limits ?? emptyLimits();
			return backend.apiState.put<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${technicalUser.appId}/technical-users/${technicalUser.technicalUserId}/limits`,
				{
					...next,
					[period]: {
						...next[period],
						costMicroDollars: cost.trim() ? toMicroDollars(cost) : null,
						tokenLimit: tokens.trim() ? toTokenLimit(tokens) : null,
					},
				},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});

	return (
		<div className="grid gap-2 border-t pt-3 sm:grid-cols-[1fr_1fr_auto]">
			<Input
				inputMode="decimal"
				placeholder={t("limit", "$ limit")}
				value={cost}
				onChange={(event) => setCost(event.target.value)}
				className="h-8"
			/>
			<Input
				inputMode="numeric"
				placeholder={t("tokenLimit", "Token limit")}
				value={tokens}
				onChange={(event) => setTokens(event.target.value)}
				className="h-8"
			/>
			<Button
				size="icon"
				variant="outline"
				disabled={mutation.isPending}
				onClick={() => mutation.mutate()}
				aria-label={t(
					"saveTechnicalUserUsageLimit",
					"Save technical user usage limit",
				)}
			>
				<Save className="h-4 w-4" />
			</Button>
		</div>
	);
}

function PowerUsers({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const maxInteractions = Math.max(
		1,
		...(overview?.powerUsers.map((user) => user.totalInteractions) ?? []),
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("powerUsers", "Power Users")}
				</CardTitle>
				<CardDescription>
					{t(
						"highestActivityAcrossTheLast30Days",
						"Highest activity across the last 30 days.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-2">
				{loading && <Skeleton className="h-36 w-full" />}
				{overview?.powerUsers.map((user) => (
					<div
						key={user.userId}
						className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
					>
						<div className="min-w-0">
							<div className="truncate font-medium">
								{user.displayName ?? user.email ?? user.userId}
							</div>
							<div className="truncate text-xs text-muted-foreground">
								{formatCount(user.aiInvocations)} {t("ai", "AI,")}{" "}
								{formatCount(user.executions)} runs, {user.activeDays}{" "}
								{t("activeDays", "active days")}
							</div>
						</div>
						<div className="text-right">
							<div className="font-medium">
								{formatCount(user.totalInteractions)}
							</div>
							<div className="text-xs text-muted-foreground">
								{formatCost(user.totalPrice)}
							</div>
						</div>
						<div className="col-span-2 h-1.5 overflow-hidden rounded-full bg-muted">
							<div
								className="h-full rounded-full bg-primary"
								style={{
									width: `${Math.max(
										4,
										(user.totalInteractions / maxInteractions) * 100,
									)}%`,
								}}
							/>
						</div>
					</div>
				))}
				{overview && overview.powerUsers.length === 0 && (
					<div className="rounded-md border p-4 text-sm text-muted-foreground">
						{t(
							"noPowerUsersInTheLast30Days",
							"No power users in the last 30 days.",
						)}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function TopAppsActivityChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() =>
			(overview?.apps ?? []).slice(0, 8).map((app, index) => ({
				name: truncateChartLabel(
					app.appName ??
						app.appId ??
						t("appVal", "App {{val}}", { val: index + 1 }),
				),
				fullName: app.appName ?? app.appId ?? "Unknown app",
				aiCalls: app.llmInvocations + app.embeddingInvocations,
				executions: app.executions,
				cost: app.totalPrice,
			})),
		[overview?.apps],
	);
	const hasData = data.some((item) => item.aiCalls || item.executions);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("topAppsByActivity", "Top Apps by Activity")}
				</CardTitle>
				<CardDescription>
					{t(
						"remoteAiCallsAndAppExecutions",
						"Remote AI calls and app executions.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : !hasData ? (
					<EmptyChart
						label={t(
							"noAppActivityForThisPeriod",
							"No app activity for this period.",
						)}
					/>
				) : (
					<ChartContainer
						config={appActivityChartConfig}
						className="h-72 w-full"
					>
						<BarChart
							data={data}
							layout="vertical"
							margin={{ top: 8, right: 16, left: 8, bottom: 0 }}
						>
							<CartesianGrid strokeDasharray="3 3" horizontal={false} />
							<XAxis
								type="number"
								tickLine={false}
								axisLine={false}
								tickFormatter={chartTick}
								allowDecimals={false}
							/>
							<YAxis
								type="category"
								dataKey="name"
								tickLine={false}
								axisLine={false}
								width={112}
								tick={{ fontSize: 11 }}
							/>
							<ChartTooltip
								cursor={{ fill: "var(--muted)" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<Bar
								dataKey="aiCalls"
								stackId="activity"
								fill="var(--color-aiCalls)"
								radius={[0, 3, 3, 0]}
							/>
							<Bar
								dataKey="executions"
								stackId="activity"
								fill="var(--color-executions)"
								radius={[0, 3, 3, 0]}
							/>
						</BarChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function TopModelsChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() =>
			(overview?.models ?? []).slice(0, 8).map((model) => ({
				name: truncateChartLabel(model.modelId, 22),
				fullName: model.modelId,
				invocations: model.invocations,
				tokens: model.tokens,
				cost: model.price,
				kind: model.kind,
			})),
		[overview?.models],
	);
	const hasData = data.some((item) => item.invocations || item.tokens);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("remoteModelMix", "Remote Model Mix")}
				</CardTitle>
				<CardDescription>
					{t(
						"whichRemoteModelsAreCarryingTraffic",
						"Which remote models are carrying traffic.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : !hasData ? (
					<EmptyChart
						label={t(
							"noRemoteModelUsageForThisPeriod",
							"No remote model usage for this period.",
						)}
					/>
				) : (
					<div className="space-y-4">
						<ChartContainer config={modelChartConfig} className="h-64 w-full">
							<BarChart
								data={data}
								layout="vertical"
								margin={{ top: 8, right: 16, left: 8, bottom: 0 }}
							>
								<CartesianGrid strokeDasharray="3 3" horizontal={false} />
								<XAxis
									type="number"
									tickLine={false}
									axisLine={false}
									tickFormatter={chartTick}
									allowDecimals={false}
								/>
								<YAxis
									type="category"
									dataKey="name"
									tickLine={false}
									axisLine={false}
									width={132}
									tick={{ fontSize: 11 }}
								/>
								<ChartTooltip
									cursor={{ fill: "var(--muted)" }}
									content={<ChartTooltipContent indicator="dot" />}
								/>
								<Bar
									dataKey="invocations"
									fill="var(--color-invocations)"
									radius={[0, 3, 3, 0]}
								/>
							</BarChart>
						</ChartContainer>
						<div className="grid gap-2 text-xs sm:grid-cols-2">
							{data.slice(0, 4).map((model) => (
								<div
									key={model.fullName}
									className="rounded-md border px-3 py-2"
								>
									<div className="truncate font-medium">{model.fullName}</div>
									<div className="text-muted-foreground">
										{model.kind} - {formatCount(model.tokens)} tokens -{" "}
										{formatCost(model.cost)}
									</div>
								</div>
							))}
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function LimitUtilization({
	overview,
	loading,
	period,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
	period: IUsageLimitPeriod;
}) {
	const { t } = useTranslation("admin");
	const rows = useMemo(
		() =>
			(overview?.apps ?? [])
				.map((app) => {
					const window = app.limits?.[period];
					if (!window?.enabled) return null;
					const costPercent =
						window.costMicroDollars && window.costMicroDollars > 0
							? (app.totalPrice / window.costMicroDollars) * 100
							: null;
					const tokenPercent =
						window.tokenLimit && window.tokenLimit > 0
							? (app.totalTokens / window.tokenLimit) * 100
							: null;
					const utilization = Math.max(costPercent ?? 0, tokenPercent ?? 0);
					if (!costPercent && !tokenPercent) return null;
					return {
						id: app.appId ?? app.appName ?? "unknown",
						name: app.appName ?? app.appId ?? "Unknown app",
						utilization,
						costPercent,
						tokenPercent,
						hard: window.hard,
					};
				})
				.filter((row): row is NonNullable<typeof row> => Boolean(row))
				.sort((a, b) => b.utilization - a.utilization)
				.slice(0, 7),
		[overview?.apps, period],
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("limitUtilization", "Limit Utilization")}
				</CardTitle>
				<CardDescription>
					{t(
						"appsClosestToTheirPeriodGuardrails",
						"Apps closest to their {{period}} guardrails.",
						{ period },
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{loading && <Skeleton className="h-64 w-full" />}
				{!loading &&
					rows.map((row) => {
						const color =
							row.utilization >= 100
								? "hsl(var(--destructive))"
								: row.utilization >= 80
									? "hsl(var(--chart-4))"
									: "hsl(var(--chart-2))";
						return (
							<div key={row.id} className="space-y-1.5">
								<div className="flex items-center justify-between gap-3 text-sm">
									<div className="min-w-0 truncate font-medium">{row.name}</div>
									<div className="shrink-0 text-xs text-muted-foreground">
										{formatPercent(row.utilization)}
										{row.hard ? " hard" : " soft"}
									</div>
								</div>
								<div className="h-2 overflow-hidden rounded-full bg-muted">
									<div
										className="h-full rounded-full"
										style={{
											width: `${Math.min(100, Math.max(3, row.utilization))}%`,
											backgroundColor: color,
										}}
									/>
								</div>
								<div className="flex gap-3 text-[11px] text-muted-foreground">
									<span>
										{t("cost", "Cost")}{" "}
										{row.costPercent === null
											? "n/a"
											: formatPercent(row.costPercent)}
									</span>
									<span>
										{t("tokens", "Tokens")}{" "}
										{row.tokenPercent === null
											? "n/a"
											: formatPercent(row.tokenPercent)}
									</span>
								</div>
							</div>
						);
					})}
				{!loading && rows.length === 0 && (
					<div className="rounded-md border p-4 text-sm text-muted-foreground">
						{t(
							"noLimitsConfiguredForActiveAppsInThisPeriod",
							"No limits configured for active apps in this period.",
						)}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function LimitEditor({
	app,
	period,
	profile,
}: {
	app: IAdminAppUsage;
	period: IUsageLimitPeriod;
	profile: IProfile;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [cost, setCost] = useState(costInputValue(app.limits, period));
	const [tokens, setTokens] = useState(tokenInputValue(app.limits, period));

	useEffect(() => {
		setCost(costInputValue(app.limits, period));
		setTokens(tokenInputValue(app.limits, period));
	}, [app.limits, period]);

	const mutation = useMutation({
		mutationFn: async () => {
			if (!app.appId) throw new Error("Missing app id");
			const next = app.limits ?? emptyLimits();
			return backend.apiState.put<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${app.appId}/limits`,
				{
					...next,
					[period]: {
						...next[period],
						costMicroDollars: cost.trim() ? toMicroDollars(cost) : null,
						tokenLimit: tokens.trim() ? toTokenLimit(tokens) : null,
					},
				},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});

	if (!app.appId) {
		return (
			<span className="text-xs text-muted-foreground">
				{t("noAppContext", "No app context")}
			</span>
		);
	}

	return (
		<div className="grid gap-2 sm:grid-cols-[1fr_1fr_auto]">
			<Input
				inputMode="decimal"
				placeholder={t("limit", "$ limit")}
				value={cost}
				onChange={(event) => setCost(event.target.value)}
				className="h-8"
			/>
			<Input
				inputMode="numeric"
				placeholder={t("tokenLimit", "Token limit")}
				value={tokens}
				onChange={(event) => setTokens(event.target.value)}
				className="h-8"
			/>
			<Button
				size="icon"
				variant="outline"
				disabled={mutation.isPending}
				onClick={() => mutation.mutate()}
				aria-label={t("saveUsageLimit", "Save usage limit")}
			>
				<Save className="h-4 w-4" />
			</Button>
		</div>
	);
}

function ManualLimitEditor({
	profile,
	period,
}: {
	profile: IProfile;
	period: IUsageLimitPeriod;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [appId, setAppId] = useState("");
	const [cost, setCost] = useState("");
	const [tokens, setTokens] = useState("");

	const mutation = useMutation({
		mutationFn: async () => {
			const existing = await backend.apiState.get<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${appId.trim()}/limits`,
			);
			return backend.apiState.put<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${appId.trim()}/limits`,
				{
					...existing,
					[period]: {
						...existing[period],
						costMicroDollars: cost.trim() ? toMicroDollars(cost) : null,
						tokenLimit: tokens.trim() ? toTokenLimit(tokens) : null,
					},
				},
			);
		},
		onSuccess: async () => {
			setCost("");
			setTokens("");
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});

	return (
		<div className="grid gap-3 border-t pt-4 sm:grid-cols-[1.5fr_1fr_1fr_auto]">
			<div className="space-y-1">
				<Label htmlFor="usage-limit-app-id">{t("appId", "App ID")}</Label>
				<Input
					id="usage-limit-app-id"
					value={appId}
					onChange={(event) => setAppId(event.target.value)}
					placeholder="app_..."
				/>
			</div>
			<div className="space-y-1">
				<Label htmlFor="usage-limit-cost">{t("costLimit", "Cost limit")}</Label>
				<Input
					id="usage-limit-cost"
					inputMode="decimal"
					value={cost}
					onChange={(event) => setCost(event.target.value)}
					placeholder="$"
				/>
			</div>
			<div className="space-y-1">
				<Label htmlFor="usage-limit-tokens">
					{t("tokenLimit", "Token limit")}
				</Label>
				<Input
					id="usage-limit-tokens"
					inputMode="numeric"
					value={tokens}
					onChange={(event) => setTokens(event.target.value)}
					placeholder="tokens"
				/>
			</div>
			<div className="flex items-end">
				<Button
					className="w-full sm:w-auto"
					disabled={!appId.trim() || mutation.isPending}
					onClick={() => mutation.mutate()}
				>
					<Save className="mr-2 h-4 w-4" />
					{t("save", "Save")}
				</Button>
			</div>
		</div>
	);
}

function UsageOperations({
	profile,
	period,
}: {
	profile: IProfile;
	period: IUsageLimitPeriod;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const alerts = useQuery<IAdminPaginated<IAdminUsageAlert>>({
		queryKey: ["admin", "usage", "alerts"],
		queryFn: () =>
			backend.apiState.get<IAdminPaginated<IAdminUsageAlert>>(
				profile,
				"admin/usage/alerts?page_size=5",
			),
	});
	const invocations = useQuery<IAdminPaginated<IAdminUsageInvocation>>({
		queryKey: ["admin", "usage", "invocations", period],
		queryFn: () =>
			backend.apiState.get<IAdminPaginated<IAdminUsageInvocation>>(
				profile,
				`admin/usage/invocations?period=${period}&page_size=8`,
			),
	});
	const reconcile = useMutation({
		mutationFn: () =>
			backend.apiState.post<IUsageReconciliationResult>(
				profile,
				"admin/usage/reconcile?older_than_minutes=30",
				{},
			),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});
	const acknowledge = useMutation({
		mutationFn: (alertId: string) =>
			backend.apiState.post<IAdminUsageAlert>(
				profile,
				`admin/usage/alerts/${alertId}/ack`,
				{},
			),
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "usage", "alerts"],
			});
		},
	});

	return (
		<div className="grid gap-4 lg:grid-cols-2">
			<Card>
				<CardHeader className="flex flex-row items-start justify-between gap-3">
					<div>
						<CardTitle className="text-base">
							{t("usageLedger", "Usage Ledger")}
						</CardTitle>
						<CardDescription>
							{t(
								"recentProviderCallsIncludingPendingAndUnknownUsage",
								"Recent provider calls, including pending and unknown usage.",
							)}
						</CardDescription>
					</div>
					<Button
						size="sm"
						variant="outline"
						disabled={reconcile.isPending}
						onClick={() => reconcile.mutate()}
					>
						<RefreshCw className="mr-2 h-4 w-4" />
						{t("reconcile", "Reconcile")}
					</Button>
				</CardHeader>
				<CardContent className="space-y-2">
					{invocations.isLoading && <Skeleton className="h-32 w-full" />}
					{invocations.data?.items.map((item) => (
						<div
							key={item.id}
							className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
						>
							<div className="min-w-0">
								<div className="truncate font-medium">
									{item.modelId ?? item.kind}
								</div>
								<div className="truncate text-xs text-muted-foreground">
									{item.provider ?? "provider"} - {item.status} -{" "}
									{item.appId ?? t("noApp", "no app")}
									{item.technicalUserId
										? t("keyTechnicaluserid", " - key {{technicalUserId}}", {
												technicalUserId: item.technicalUserId,
											})
										: ""}
								</div>
							</div>
							<div className="text-right">
								<div className="font-medium">
									{formatCost(
										item.costMicroDollars || item.estimatedCostMicroDollars,
									)}
								</div>
								<div className="text-xs text-muted-foreground">
									{formatCount(
										item.inputTokens +
											item.outputTokens +
											item.embeddingTokens || item.estimatedTokens,
									)}{" "}
									tokens
								</div>
							</div>
						</div>
					))}
					{invocations.data?.items.length === 0 && (
						<div className="rounded-md border p-4 text-sm text-muted-foreground">
							{t(
								"noLedgerEntriesForThisPeriod",
								"No ledger entries for this period.",
							)}
						</div>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle className="text-base">
						{t("usageAlerts", "Usage Alerts")}
					</CardTitle>
					<CardDescription>
						{t(
							"limitWarningsHardBlocksAndCostAnomalies",
							"Limit warnings, hard blocks, and cost anomalies.",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-2">
					{alerts.isLoading && <Skeleton className="h-32 w-full" />}
					{alerts.data?.items.map((alert) => (
						<div
							key={alert.id}
							className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
						>
							<div className="min-w-0">
								<div className="flex items-center gap-2 font-medium">
									<AlertTriangle className="h-4 w-4 text-amber-500" />
									<span className="truncate">{alert.message}</span>
								</div>
								<div className="truncate text-xs text-muted-foreground">
									{alert.severity} - {alert.period ?? "period"} -{" "}
									{alert.appId ?? t("noApp", "no app")}
								</div>
							</div>
							<Button
								size="sm"
								variant="outline"
								disabled={
									Boolean(alert.acknowledgedAt) || acknowledge.isPending
								}
								onClick={() => acknowledge.mutate(alert.id)}
							>
								<CheckCircle className="mr-2 h-4 w-4" />
								{t("ack", "Ack")}
							</Button>
						</div>
					))}
					{alerts.data?.items.length === 0 && (
						<div className="rounded-md border p-4 text-sm text-muted-foreground">
							{t("noUsageAlerts", "No usage alerts.")}
						</div>
					)}
				</CardContent>
			</Card>
		</div>
	);
}

export function UsageOverviewSection({
	profile,
	hasAdminAccess,
}: {
	profile: IProfile | undefined;
	hasAdminAccess: boolean;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const [period, setPeriod] = useState<IUsageLimitPeriod>("monthly");
	const overview = useQuery<IAdminUsageOverview>({
		queryKey: ["admin", "usage", period],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IAdminUsageOverview>(
				profile,
				`admin/usage/overview?period=${period}`,
			);
		},
		enabled: Boolean(profile && hasAdminAccess),
	});

	if (!hasAdminAccess) return null;

	return (
		<div className="space-y-4">
			<div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<div>
					<h2 className="text-lg font-semibold">
						{t("usageDashboard", "Usage Dashboard")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t(
							"remoteModelSpendAppExecutionsUserActivityAndLimits",
							"Remote model spend, app executions, user activity, and limits.",
						)}
					</p>
				</div>
				<Select
					value={period}
					onValueChange={(value) => setPeriod(value as IUsageLimitPeriod)}
				>
					<SelectTrigger className="w-full sm:w-40">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{PERIODS.map((item) => (
							<SelectItem key={item} value={item}>
								{item[0].toUpperCase() + item.slice(1)}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			<div className="grid gap-4 xl:grid-cols-[2fr_1fr]">
				<ActivityTrendChart
					overview={overview.data}
					loading={overview.isLoading}
				/>
				<UsageHealthSummary
					overview={overview.data}
					loading={overview.isLoading}
					period={period}
				/>
			</div>

			<div className="grid gap-4 xl:grid-cols-[1.5fr_1fr]">
				<SpendTokenTrendChart
					overview={overview.data}
					loading={overview.isLoading}
				/>
				<SpendMixChart overview={overview.data} loading={overview.isLoading} />
			</div>

			<div className="grid gap-4 lg:grid-cols-2">
				<TopAppsActivityChart
					overview={overview.data}
					loading={overview.isLoading}
				/>
				<TopModelsChart overview={overview.data} loading={overview.isLoading} />
			</div>

			<div className="grid gap-4 xl:grid-cols-3">
				<LimitUtilization
					overview={overview.data}
					loading={overview.isLoading}
					period={period}
				/>
				<PowerUsers overview={overview.data} loading={overview.isLoading} />
				<TechnicalUsers
					overview={overview.data}
					loading={overview.isLoading}
					profile={profile}
					period={period}
				/>
			</div>

			{profile && <UsageOperations profile={profile} period={period} />}

			<Card>
				<CardHeader>
					<CardTitle className="text-base">
						{t("topApps", "Top Apps")}
					</CardTitle>
					<CardDescription>
						{t(
							"setRollingPeriodCostAndTokenLimitsPerApp",
							"Set rolling {{period}} cost and token limits per app.",
							{ period },
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						{overview.isLoading && <Skeleton className="h-32 w-full" />}
						{overview.data?.apps.map((app) => (
							<div
								key={app.appId ?? "unknown"}
								className="grid gap-3 rounded-md border p-3 lg:grid-cols-[1.2fr_0.8fr_1.2fr]"
							>
								<div className="min-w-0">
									<div className="truncate text-sm font-medium">
										{app.appName ?? app.appId ?? "Unknown app"}
									</div>
									<div className="truncate text-xs text-muted-foreground">
										{app.appId ??
											t("usageWithoutAppContext", "usage without app context")}
									</div>
								</div>
								<div className="grid grid-cols-3 gap-2 text-xs">
									<div>
										<div className="text-muted-foreground">
											{t("cost", "Cost")}
										</div>
										<div className="font-medium">
											{formatCost(app.totalPrice)}
										</div>
									</div>
									<div>
										<div className="text-muted-foreground">
											{t("tokens", "Tokens")}
										</div>
										<div className="font-medium">
											{formatCount(app.totalTokens)}
										</div>
									</div>
									<div>
										<div className="text-muted-foreground">
											{t("runs", "Runs")}
										</div>
										<div className="font-medium">
											{formatCount(app.executions)}
										</div>
									</div>
								</div>
								{profile && (
									<LimitEditor app={app} period={period} profile={profile} />
								)}
							</div>
						))}
						{overview.data && overview.data.apps.length === 0 && (
							<div className="rounded-md border p-4 text-sm text-muted-foreground">
								{t(
									"noUsageRecordedForThisPeriod",
									"No usage recorded for this period.",
								)}
							</div>
						)}
					</div>
					{profile && <ManualLimitEditor profile={profile} period={period} />}
				</CardContent>
			</Card>

			<div className="grid gap-4 lg:grid-cols-2">
				<Card>
					<CardHeader>
						<CardTitle className="text-base">
							{t("topUsers", "Top Users")}
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-2">
						{overview.isLoading && <Skeleton className="h-28 w-full" />}
						{overview.data?.users.map((user) => (
							<div
								key={user.userId ?? "unknown"}
								className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
							>
								<div className="min-w-0">
									<div className="truncate font-medium">
										{user.displayName ??
											user.email ??
											user.userId ??
											"Unknown user"}
									</div>
									<div className="truncate text-xs text-muted-foreground">
										{user.email ??
											user.userId ??
											t(
												"usageWithoutUserContext",
												"usage without user context",
											)}
									</div>
								</div>
								<div className="text-right">
									<div className="font-medium">
										{formatCost(user.totalPrice)}
									</div>
									<div className="text-xs text-muted-foreground">
										{formatCount(user.totalTokens)} tokens
									</div>
								</div>
							</div>
						))}
					</CardContent>
				</Card>
				<Card>
					<CardHeader>
						<CardTitle className="text-base">
							{t("topRemoteModels", "Top Remote Models")}
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-2">
						{overview.isLoading && <Skeleton className="h-28 w-full" />}
						{overview.data?.models.map((model) => (
							<div
								key={`${model.kind}:${model.provider ?? ""}:${model.modelId}`}
								className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
							>
								<div className="min-w-0">
									<div className="truncate font-medium">{model.modelId}</div>
									<div className="truncate text-xs text-muted-foreground">
										{model.kind}
										{model.provider ? ` - ${model.provider}` : ""}
									</div>
								</div>
								<div className="text-right">
									<div className="font-medium">{formatCost(model.price)}</div>
									<div className="text-xs text-muted-foreground">
										{formatCount(model.invocations)} calls
									</div>
								</div>
							</div>
						))}
					</CardContent>
				</Card>
			</div>
		</div>
	);
}

interface GovernanceScoresSummaryData {
	criticalApps: number;
	flaggedApps: number;
	totalApps: number;
	worstApps: Array<{
		appId: string;
		appName: string | null;
		worstScore: number;
		security: number;
		privacy: number;
	}>;
}

interface AiActExportRow {
	appId: string;
	appName: string | null;
	riskCategory: string;
	status: string;
	conformityScore: number | null;
	conformityBand: string | null;
	updatedAt: string;
}

function AiActConformityPreview({
	profile,
}: { profile: IProfile | undefined }) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const features = useFeatures();
	const aiActEnabled = features.data?.ai_act === true;

	const inventory = useQuery<AiActExportRow[]>({
		queryKey: ["admin", "ai-act", "dashboard", "export"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<AiActExportRow[]>(
				profile,
				"admin/ai-act/inventory/export?format=json",
			);
		},
		enabled: !!profile && aiActEnabled,
	});

	const stats = useMemo(() => {
		const rows = inventory.data ?? [];
		const assessed = rows.filter(
			(r) => r.status.toUpperCase() !== "UNASSESSED",
		);
		const prohibited = rows.filter(
			(r) => r.riskCategory === "PROHIBITED",
		).length;
		const high = rows.filter((r) => r.riskCategory === "HIGH").length;
		const limited = rows.filter((r) => r.riskCategory === "LIMITED").length;
		const scored = rows.filter((r) => typeof r.conformityScore === "number");
		const avgConformity =
			scored.length > 0
				? Math.round(
						scored.reduce((sum, r) => sum + (r.conformityScore ?? 0), 0) /
							scored.length,
					)
				: null;
		return {
			total: assessed.length,
			prohibited,
			high,
			limited,
			avgConformity,
		};
	}, [inventory.data]);

	if (!aiActEnabled) return null;

	return (
		<div className="space-y-2 rounded-lg border border-indigo-500/30 bg-indigo-500/5 p-3">
			<div className="flex flex-col items-start gap-2 min-[400px]:flex-row min-[400px]:items-center min-[400px]:justify-between">
				<div className="flex items-center gap-2 text-sm font-medium">
					<Scale className="h-4 w-4 text-indigo-500" />
					{t("euAiActConformity", "EU AI Act Conformity")}
				</div>
				<Button asChild variant="ghost" size="sm" className="h-7 px-2 text-xs">
					<Link href="/admin/ai-act">
						{t("openInventory", "Open Inventory")}
					</Link>
				</Button>
			</div>
			{inventory.isLoading ? (
				<Skeleton className="h-16 w-full" />
			) : (
				<div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
					<div className="rounded-md border bg-background p-2">
						<div className="text-[11px] text-muted-foreground">
							{t("assessed", "Assessed")}
						</div>
						<div className="text-lg font-semibold">{stats.total}</div>
					</div>
					<div
						className={`rounded-md border bg-background p-2 ${
							stats.prohibited > 0 ? "border-red-500/50" : ""
						}`}
					>
						<div className="text-[11px] text-muted-foreground">
							{t("prohibited", "Prohibited")}
						</div>
						<div
							className={`text-lg font-semibold ${
								stats.prohibited > 0 ? "text-red-600 dark:text-red-400" : ""
							}`}
						>
							{stats.prohibited}
						</div>
					</div>
					<div
						className={`rounded-md border bg-background p-2 ${
							stats.high > 0 ? "border-amber-500/50" : ""
						}`}
					>
						<div className="text-[11px] text-muted-foreground">
							{t("highrisk", "High-risk")}
						</div>
						<div
							className={`text-lg font-semibold ${
								stats.high > 0 ? "text-amber-600 dark:text-amber-400" : ""
							}`}
						>
							{stats.high}
						</div>
					</div>
					<div className="rounded-md border bg-background p-2">
						<div className="text-[11px] text-muted-foreground">
							{t("limited", "Limited")}
						</div>
						<div className="text-lg font-semibold">{stats.limited}</div>
					</div>
					<div className="rounded-md border bg-background p-2">
						<div className="text-[11px] text-muted-foreground">
							{t("avgConformity", "Avg. conformity")}
						</div>
						<div className="text-lg font-semibold">
							{stats.avgConformity ?? "—"}
							{stats.avgConformity !== null && (
								<span className="text-xs text-muted-foreground">%</span>
							)}
						</div>
					</div>
				</div>
			)}
		</div>
	);
}

export function GovernanceScoresSummary({
	profile,
}: {
	profile: IProfile | undefined;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const summary = useQuery<GovernanceScoresSummaryData>({
		queryKey: ["admin", "governance", "scores", "summary"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<GovernanceScoresSummaryData>(
				profile,
				"admin/governance/scores/summary",
			);
		},
		enabled: !!profile,
	});

	const hasCriticalIssues = (summary.data?.criticalApps ?? 0) > 0;
	const hasFlaggedIssues = (summary.data?.flaggedApps ?? 0) > 0;

	return (
		<Card
			className={
				hasCriticalIssues
					? "border-red-500/50 bg-red-500/5"
					: hasFlaggedIssues
						? "border-yellow-500/50 bg-yellow-500/5"
						: ""
			}
		>
			<CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
				<div>
					<CardTitle className="flex items-center gap-2 text-base">
						<Shield
							className={`h-4 w-4 ${
								hasCriticalIssues
									? "text-red-500"
									: hasFlaggedIssues
										? "text-yellow-500"
										: "text-green-500"
							}`}
						/>
						{t("aiInventoryGovernance", "AI Inventory & Governance")}
					</CardTitle>
					<CardDescription>
						{t(
							"euAiActConformityTogetherWithSecurityAndQualityScoresAcrossPublishedApps",
							"EU AI Act conformity together with security and quality scores across published apps",
						)}
					</CardDescription>
				</div>
				<Button asChild variant="outline" size="sm">
					<Link href="/admin/ai-act">
						{t("viewFullInventory", "View Full Inventory")}
					</Link>
				</Button>
			</CardHeader>
			<CardContent>
				{summary.isLoading ? (
					<Skeleton className="h-32 w-full" />
				) : summary.error ? (
					<div className="rounded-md border border-destructive/40 p-4 text-center text-sm text-destructive">
						{t(
							"failedToLoadGovernanceScoresPleaseCheckTheApiLogs",
							"Failed to load governance scores. Please check the API logs.",
						)}
					</div>
				) : summary.data ? (
					<div className="space-y-4">
						<AiActConformityPreview profile={profile} />

						<div className="grid gap-3 sm:grid-cols-3">
							<div className="rounded-lg border p-3">
								<div className="text-xs text-muted-foreground">
									{t("totalApps", "Total Apps")}
								</div>
								<div className="text-2xl font-semibold">
									{summary.data.totalApps ?? 0}
								</div>
							</div>
							<div
								className={`rounded-lg border p-3 ${
									(summary.data.criticalApps ?? 0) > 0
										? "border-red-500/50 bg-red-500/5"
										: ""
								}`}
							>
								<div className="text-xs text-muted-foreground">
									{t("criticalIssues", "Critical Issues")}
								</div>
								<div
									className={`text-2xl font-semibold ${
										(summary.data.criticalApps ?? 0) > 0
											? "text-red-600 dark:text-red-400"
											: ""
									}`}
								>
									{summary.data.criticalApps ?? 0}
								</div>
								<div className="text-xs text-muted-foreground">
									{t("score3", "Score ≤ 3")}
								</div>
							</div>
							<div
								className={`rounded-lg border p-3 ${
									(summary.data.flaggedApps ?? 0) > 0
										? "border-yellow-500/50 bg-yellow-500/5"
										: ""
								}`}
							>
								<div className="text-xs text-muted-foreground">
									{t("flaggedApps", "Flagged Apps")}
								</div>
								<div
									className={`text-2xl font-semibold ${
										(summary.data.flaggedApps ?? 0) > 0
											? "text-yellow-600 dark:text-yellow-400"
											: ""
									}`}
								>
									{summary.data.flaggedApps ?? 0}
								</div>
								<div className="text-xs text-muted-foreground">
									{t("score6", "Score ≤ 6")}
								</div>
							</div>
						</div>

						{summary.data.worstApps && summary.data.worstApps.length > 0 && (
							<div className="space-y-2">
								<div className="text-sm font-medium">
									{t("appsRequiringAttention", "Apps Requiring Attention")}
								</div>
								{summary.data.worstApps.map((app) => (
									<Link
										key={app.appId}
										href={`/admin/ai-act?id=${encodeURIComponent(app.appId)}`}
										className="block"
									>
										<div className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm transition-colors hover:border-primary/40">
											<div className="min-w-0">
												<div className="truncate font-medium">
													{app.appName ?? app.appId}
												</div>
												<div className="flex items-center gap-3 text-xs text-muted-foreground">
													<span className="truncate">{app.appId}</span>
												</div>
											</div>
											<div className="flex items-center gap-3">
												<div className="text-right text-xs">
													<div className="text-muted-foreground">
														{t("security", "Security")}
													</div>
													<div
														className={`font-semibold ${
															app.security >= 7
																? "text-green-600 dark:text-green-400"
																: app.security >= 4
																	? "text-yellow-600 dark:text-yellow-400"
																	: "text-red-600 dark:text-red-400"
														}`}
													>
														{app.security}
													</div>
												</div>
												<div className="text-right text-xs">
													<div className="text-muted-foreground">
														{t("privacy", "Privacy")}
													</div>
													<div
														className={`font-semibold ${
															app.privacy >= 7
																? "text-green-600 dark:text-green-400"
																: app.privacy >= 4
																	? "text-yellow-600 dark:text-yellow-400"
																	: "text-red-600 dark:text-red-400"
														}`}
													>
														{app.privacy}
													</div>
												</div>
												<div className="text-right">
													<div className="text-xs text-muted-foreground">
														{t("worst", "Worst")}
													</div>
													<div
														className={`text-lg font-bold ${
															app.worstScore >= 7
																? "text-green-600 dark:text-green-400"
																: app.worstScore >= 4
																	? "text-yellow-600 dark:text-yellow-400"
																	: "text-red-600 dark:text-red-400"
														}`}
													>
														{app.worstScore}
													</div>
												</div>
											</div>
										</div>
									</Link>
								))}
							</div>
						)}
					</div>
				) : (
					<div className="rounded-md border p-4 text-center text-sm text-muted-foreground">
						{t(
							"noGovernanceDataAvailableYetScoresWillAppearAfterAppsArePublishedAndAnalyzed",
							"No governance data available yet. Scores will appear after apps are published and analyzed.",
						)}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

