"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Bot,
	CircleDollarSign,
	MessagesSquare,
	OctagonAlert,
	Users,
} from "lucide-react";
import { useMemo } from "react";
import { Bar, BarChart, CartesianGrid, XAxis, YAxis } from "recharts";
import type {
	IAdminBotActivity,
	IAdminBotFailure,
	IAdminBotHost,
	IAdminBotTrendPoint,
	IAdminBotUsage,
	IUsageLimitPeriod,
} from "../../../lib/schema/usage";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	type ChartConfig,
	ChartContainer,
	ChartLegend,
	ChartLegendContent,
	ChartTooltip,
	ChartTooltipContent,
} from "../../ui/chart";
import { Skeleton } from "../../ui/skeleton";
import { failureLabel } from "./admin-bot-failure";
import {
	QueryErrorState,
	chartTick,
	formatCost,
	formatCount,
	formatDuration,
	formatPercent,
} from "./admin-usage-shared";
import {
	BarList,
	type BarListRow,
	EmptyState,
	StatTile,
} from "./telemetry/telemetry-shared";

type Translate = ReturnType<typeof useTranslation>["t"];

const KPI_KEYS = ["turns", "bots", "people", "failed", "spend"] as const;

function share(part: number, whole: number) {
	return whole > 0 ? (part / whole) * 100 : 0;
}

function botName(t: Translate, bot: IAdminBotUsage) {
	return bot.name?.trim() || t("unnamedBot", "Unnamed bot");
}

function hostLabel(t: Translate, hostKind: string) {
	switch (hostKind) {
		case "server":
			return t("botHostServer", "Cloud");
		case "desktop":
			return t("botHostDesktop", "Desktop");
		case "device":
			return t("botHostDevice", "Device");
		default:
			return hostKind;
	}
}

function joinDetails(parts: (string | null | false)[]) {
	return parts.filter(Boolean).join(" · ");
}

export function BotActivitySection({
	profile,
	period,
}: {
	profile: IProfile;
	period: IUsageLimitPeriod;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const activity = useQuery<IAdminBotActivity>({
		queryKey: [
			"admin",
			"usage",
			"bot-activity",
			period,
			profile.hub,
			profile.id,
		],
		staleTime: 60_000,
		meta: { persist: false, adminDashboard: true },
		queryFn: () =>
			backend.apiState.get<IAdminBotActivity>(
				profile,
				`admin/usage/bot-activity?period=${period}`,
			),
	});
	const data = activity.data;
	const loading = activity.isLoading;

	return (
		<section className="space-y-4" aria-labelledby="admin-bot-activity">
			<div>
				<h3 id="admin-bot-activity" className="text-base font-semibold">
					{t("bots", "Bots")}
				</h3>
				<p className="text-sm text-muted-foreground">
					{t(
						"botActivityDescription",
						"Turns, reach, reliability and spend across every bot.",
					)}
				</p>
			</div>
			{activity.isError ? (
				<QueryErrorState
					message={t(
						"botActivityUnavailable",
						"Bot activity could not be loaded. Try again to see turns, reach and spend.",
					)}
					onRetry={() => void activity.refetch()}
					retrying={activity.isFetching}
				/>
			) : (
				<>
					<BotKpis activity={data} loading={loading} />
					<div className="grid gap-4 xl:grid-cols-[2fr_1fr]">
						<BotTurnsChart trend={data?.trend} loading={loading} />
						<div className="grid content-start gap-4">
							<BotHostsCard hosts={data?.hosts} loading={loading} />
							<BotFailuresCard failures={data?.failures} loading={loading} />
						</div>
					</div>
					<div className="grid gap-4 lg:grid-cols-2">
						<MostUsedBotsCard bots={data?.topBots} loading={loading} />
						<BotSpendCard
							bots={data?.topSpenders}
							spendSince={data?.spendSince}
							loading={loading}
						/>
					</div>
				</>
			)}
		</section>
	);
}

function BotKpis({
	activity,
	loading,
}: {
	activity: IAdminBotActivity | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const totals = activity?.totals;

	if (loading || !totals) {
		return (
			<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
				{KPI_KEYS.map((key) => (
					<Skeleton key={key} className="h-16.5 w-full" />
				))}
			</div>
		);
	}

	const iconClass = "size-3.5";
	return (
		<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
			<StatTile
				label={t("botTurns", "Bot turns")}
				value={formatCount(totals.turns)}
				icon={<MessagesSquare className={iconClass} />}
				hint={t("botTurnsCompletedShare", "{{share}} completed", {
					share: formatPercent(share(totals.completed, totals.turns)),
				})}
			/>
			<StatTile
				label={t("activeBots", "Active bots")}
				value={formatCount(totals.activeBots)}
				icon={<Bot className={iconClass} />}
				hint={t("activeBotsOfLive", "of {{live}} bots · {{fresh}} new", {
					live: formatCount(totals.liveBots),
					fresh: formatCount(totals.newBots),
				})}
			/>
			<StatTile
				label={t("peopleReached", "People reached")}
				value={formatCount(totals.people)}
				icon={<Users className={iconClass} />}
				hint={t("inConversations", "in {{conversations}} conversations", {
					conversations: formatCount(totals.conversations),
				})}
			/>
			<StatTile
				label={t("failedTurns", "Failed turns")}
				value={formatCount(totals.failed)}
				icon={<OctagonAlert className={iconClass} />}
				hint={t("failedTurnsShare", "{{share}} of turns", {
					share: formatPercent(share(totals.failed, totals.turns)),
				})}
			/>
			<StatTile
				label={t("botAiSpend", "Bot AI spend")}
				value={formatCost(totals.aiCostMicros)}
				icon={<CircleDollarSign className={iconClass} />}
				hint={t("botAiCallsThisMonth", "{{calls}} AI calls this month", {
					calls: formatCount(totals.aiCalls),
				})}
			/>
		</div>
	);
}

function BotTurnsChart({
	trend,
	loading,
}: {
	trend: IAdminBotTrendPoint[] | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const config = useMemo(
		() =>
			({
				completed: {
					label: t("botOutcomeCompleted", "Completed"),
					color: "var(--good-solid)",
				},
				other: {
					label: t("cancelledOrOpen", "Cancelled or open"),
					color: "var(--unknown-solid)",
				},
				failed: {
					label: t("botOutcomeFailed", "Failed"),
					color: "var(--destructive)",
				},
			}) satisfies ChartConfig,
		[t],
	);
	const data = useMemo(
		() =>
			(trend ?? []).map((point) => ({
				label: point.label,
				completed: point.completed,
				other: Math.max(0, point.turns - point.completed - point.failed),
				failed: point.failed,
			})),
		[trend],
	);
	const hasData = data.some(
		(point) => point.completed || point.other || point.failed,
	);

	return (
		<Card className="flex flex-col">
			<CardHeader>
				<CardTitle className="text-base">
					{t("botTurnsOverTime", "Bot Turns Over Time")}
				</CardTitle>
				<CardDescription>
					{t(
						"botTurnsOverTimeDescription",
						"Every message a bot answered, by outcome.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="flex flex-1 flex-col">
				{loading ? (
					<Skeleton className="min-h-72 w-full flex-1" />
				) : !hasData ? (
					<EmptyState
						className="min-h-72 flex-1"
						message={t(
							"noBotTurnsInThisPeriod",
							"No bot turns in this period.",
						)}
					/>
				) : (
					<ChartContainer
						config={config}
						className="aspect-auto min-h-72 w-full flex-1"
					>
						<BarChart
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
								cursor={{ fill: "var(--muted)" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<ChartLegend
								content={({ payload, verticalAlign }) => (
									<ChartLegendContent
										payload={payload}
										verticalAlign={verticalAlign}
									/>
								)}
							/>
							<Bar
								dataKey="completed"
								stackId="turns"
								fill="var(--color-completed)"
								stroke="var(--card)"
								strokeWidth={2}
							/>
							<Bar
								dataKey="other"
								stackId="turns"
								fill="var(--color-other)"
								stroke="var(--card)"
								strokeWidth={2}
							/>
							<Bar
								dataKey="failed"
								stackId="turns"
								fill="var(--color-failed)"
								stroke="var(--card)"
								strokeWidth={2}
								radius={[3, 3, 0, 0]}
							/>
						</BarChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function BotHostsCard({
	hosts,
	loading,
}: {
	hosts: IAdminBotHost[] | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const rows = useMemo<BarListRow[]>(() => {
		const all = hosts ?? [];
		const total = all.reduce((sum, host) => sum + host.turns, 0);
		return all.map((host) => ({
			key: host.hostKind,
			label: hostLabel(t, host.hostKind),
			count: host.turns,
			value: `${formatCount(host.turns)} · ${formatPercent(share(host.turns, total))}`,
		}));
	}, [hosts, t]);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("whereBotsRun", "Where Bots Run")}
				</CardTitle>
				<CardDescription>
					{t(
						"whereBotsRunDescription",
						"Turns by the host that answered them.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<BarList
					rows={rows}
					loading={loading}
					mono={false}
					emptyMessage={t(
						"noBotTurnsInThisPeriod",
						"No bot turns in this period.",
					)}
				/>
			</CardContent>
		</Card>
	);
}

function BotFailuresCard({
	failures,
	loading,
}: {
	failures: IAdminBotFailure[] | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const rows = useMemo<BarListRow[]>(
		() =>
			(failures ?? []).map((failure) => ({
				key: failure.errorCode ?? "none",
				label: failureLabel(t, failure.errorCode),
				count: failure.turns,
				detail: failure.errorCode ?? undefined,
			})),
		[failures, t],
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("whyTurnsFailed", "Why Turns Failed")}
				</CardTitle>
				<CardDescription>
					{t("whyTurnsFailedDescription", "Error codes of the failed turns.")}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<BarList
					rows={rows}
					loading={loading}
					mono={false}
					emptyMessage={t(
						"noFailedBotTurns",
						"No failed turns in this period.",
					)}
				/>
			</CardContent>
		</Card>
	);
}

function MostUsedBotsCard({
	bots,
	loading,
}: {
	bots: IAdminBotUsage[] | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const rows = useMemo<BarListRow[]>(
		() =>
			(bots ?? []).map((bot) => ({
				key: bot.botId,
				label: botName(t, bot),
				count: bot.turns,
				value: t("turnsCount", "{{turns}} turns", {
					turns: formatCount(bot.turns),
				}),
				detail: joinDetails([
					bot.ownerName &&
						t("byOwner", "by {{owner}}", { owner: bot.ownerName }),
					t("botReach", "{{people}} people · {{conversations}} conversations", {
						people: formatCount(bot.people),
						conversations: formatCount(bot.conversations),
					}),
					bot.failed > 0 &&
						t("botFailedShare", "{{share}} failed", {
							share: formatPercent(share(bot.failed, bot.turns)),
						}),
					bot.deleted && t("deletedBot", "deleted"),
				]),
			})),
		[bots, t],
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("mostUsedBots", "Most Used Bots")}
				</CardTitle>
				<CardDescription>
					{t(
						"mostUsedBotsDescription",
						"Bots with the most turns, and the people they reached.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<BarList
					rows={rows}
					loading={loading}
					mono={false}
					emptyMessage={t(
						"noBotTurnsInThisPeriod",
						"No bot turns in this period.",
					)}
				/>
			</CardContent>
		</Card>
	);
}

function BotSpendCard({
	bots,
	spendSince,
	loading,
}: {
	bots: IAdminBotUsage[] | undefined;
	spendSince: string | undefined;
	loading: boolean;
}) {
	const { t } = useTranslation("admin");
	const since = useMemo(
		() =>
			spendSince
				? new Date(spendSince).toLocaleDateString(undefined, {
						month: "short",
						day: "numeric",
					})
				: "",
		[spendSince],
	);
	const rows = useMemo<BarListRow[]>(
		() =>
			(bots ?? []).map((bot) => ({
				key: bot.botId,
				label: botName(t, bot),
				count: bot.aiCostMicros,
				value: formatCost(bot.aiCostMicros),
				detail: joinDetails([
					bot.ownerName &&
						t("byOwner", "by {{owner}}", { owner: bot.ownerName }),
					t("aiCallsCount", "{{calls}} AI calls", {
						calls: formatCount(bot.aiCalls),
					}),
					bot.runtimeMs > 0 &&
						t("botRuntime", "{{runtime}} runtime", {
							runtime: formatDuration(bot.runtimeMs),
						}),
					bot.deleted && t("deletedBot", "deleted"),
				]),
			})),
		[bots, t],
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("botSpendThisMonth", "Bot Spend This Month")}
				</CardTitle>
				<CardDescription>
					{since
						? t(
								"botSpendSince",
								"AI spend of the budget month since {{date}}.",
								{
									date: since,
								},
							)
						: t("botSpendBudgetMonth", "AI spend of the current budget month.")}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<BarList
					rows={rows}
					loading={loading}
					mono={false}
					emptyMessage={t("noBotSpendThisMonth", "No bot spend this month.")}
				/>
			</CardContent>
		</Card>
	);
}
