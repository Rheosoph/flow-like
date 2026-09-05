"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Activity,
	AlertCircle,
	ArrowUpRight,
	Bell,
	CalendarDays,
	Check,
	CheckCircle2,
	Clock,
} from "lucide-react";
import Link from "next/link";
import { Fragment } from "react";
import { toast } from "sonner";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import {
	hasAttentionSeverity,
	homeActivityCoverage,
	homeActivityDays,
	homeUsageDollars,
	summarizeHomeExecutions,
} from "../home-activity-statistics";
import { useHomeLibrary } from "./collections";
import {
	type HomeContentProps,
	numberConfig,
	safeHomeHref,
	stringList,
	textConfig,
} from "./config";
import { nextHomeSchedule } from "./schedules";
import {
	HomeEmpty,
	HomeQueryState,
	homeItemClass,
	useHomeScope,
} from "./shared";

export function HomeNotifications({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const queryClient = useQueryClient();
	const attention = widget.type === "needs-attention";
	const unread = attention || widget.config.unread === true;
	const type = textConfig(
		widget.config,
		"notificationType",
		attention ? "WORKFLOW" : "all",
	);
	const limit = numberConfig(widget.config, "limit", 8);
	const notifications = useQuery({
		queryKey: ["home", ...scope, "notifications", unread, type, limit],
		queryFn: () =>
			backend.userState.listNotifications(
				unread,
				0,
				type === "all" ? limit : 100,
			),
		refetchInterval: editing ? false : 60_000,
	});
	if (notifications.isLoading || notifications.isError)
		return (
			<HomeQueryState
				loading={notifications.isLoading}
				error={notifications.isError}
				retry={() => void notifications.refetch()}
			/>
		);
	const rows = (notifications.data ?? [])
		.filter(
			(notification) =>
				type === "all" || notification.notification_type === type,
		)
		.slice(0, limit);
	if (!rows.length)
		return (
			<HomeEmpty icon={<CheckCircle2 className="size-7 text-emerald-500/70" />}>
				{attention
					? "No unread workflow notifications in the latest 100 notifications."
					: "No notifications match this view."}
			</HomeEmpty>
		);
	const markRead = async (id: string) => {
		try {
			await backend.userState.markNotificationRead(id);
			await Promise.all([
				queryClient.invalidateQueries({
					queryKey: ["home", ...scope, "notifications"],
				}),
				queryClient.invalidateQueries({
					queryKey: [backend.userState.getNotifications.name],
				}),
			]);
		} catch {
			toast.error("This notification could not be marked as read.");
		}
	};
	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
				{rows.map((notification) => {
					const href =
						safeHomeHref(notification.link ?? "") ?? "/notifications";
					return (
						<div
							key={notification.id}
							className={cn(
								homeItemClass,
								"items-start",
								!notification.read && "border-primary/25",
							)}
						>
							<Bell
								className={cn(
									"mt-0.5 size-4 shrink-0",
									notification.read ? "text-muted-foreground" : "text-primary",
								)}
							/>
							<Link
								href={href}
								className="min-w-0 flex-1 rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
							>
								<p className="text-sm font-medium">{notification.title}</p>
								{notification.description && (
									<p className="mt-1 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
										{notification.description}
									</p>
								)}
								<p className="mt-2 text-[10px] text-muted-foreground">
									{new Date(notification.created_at).toLocaleString()}
								</p>
							</Link>
							{!notification.read && (
								<Button
									variant="ghost"
									size="icon"
									className="size-7 shrink-0"
									aria-label={`Mark ${notification.title} as read`}
									disabled={editing}
									onClick={() => void markRead(notification.id)}
								>
									<Check className="size-3.5" />
								</Button>
							)}
						</div>
					);
				})}
			</div>
			<Link
				href="/notifications"
				className="flex shrink-0 items-center justify-end gap-1.5 border-t px-4 py-2.5 text-xs text-muted-foreground hover:text-foreground"
			>
				Open inbox
				<ArrowUpRight className="size-3.5" />
			</Link>
		</div>
	);
}

function useHomeExecutions(
	appId: string,
	limit: number,
	enabled = true,
	editing = false,
) {
	const backend = useBackend();
	const scope = useHomeScope();
	return useQuery({
		queryKey: ["home", ...scope, "executions", appId, limit],
		queryFn: () => {
			if (!backend.usageState)
				throw new Error("Usage history is unavailable on this backend.");
			return backend.usageState.getExecutionHistory(
				0,
				limit,
				appId || undefined,
			);
		},
		enabled: enabled && Boolean(backend.usageState),
		staleTime: 30_000,
		refetchInterval: editing ? false : 60_000,
	});
}

export function HomeRunActivity({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const results = useHomeExecutions(
		textConfig(widget.config, "appId"),
		100,
		true,
		editing,
	);
	if (!backend.usageState)
		return (
			<HomeEmpty>Usage history is not available on this backend.</HomeEmpty>
		);
	if (results.isLoading || results.isError || !results.data)
		return (
			<HomeQueryState
				loading={results.isLoading}
				error={results.isError}
				retry={() => void results.refetch()}
			/>
		);
	const statistics = summarizeHomeExecutions(results.data, widget.config.days);
	const maximum = Math.max(1, ...statistics.buckets.map((day) => day.count));
	const formatDay = (day: string) =>
		new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
			month: "short",
			day: "numeric",
			timeZone: "UTC",
		});
	return (
		<figure className="flex h-full min-h-0 flex-col">
			<figcaption className="flex items-center justify-between gap-3 px-4 pt-3 text-xs">
				<span className="text-muted-foreground">
					{statistics.days === 1 ? "Today" : `Last ${statistics.days} days`}
					{" · UTC"}
				</span>
				<span className="font-medium tabular-nums">
					{statistics.rows.length.toLocaleString()} sampled records
				</span>
			</figcaption>
			<div className="flex min-h-0 flex-1 flex-col px-4 pb-3 pt-4">
				{statistics.rows.length ? (
					<>
						<div
							className="flex min-h-16 flex-1 items-end gap-1"
							aria-hidden="true"
						>
							{statistics.buckets.map((day) => (
								<div
									key={day.day}
									className="flex h-full min-w-0 flex-1 flex-col justify-end overflow-hidden rounded-t"
									title={`${formatDay(day.day)}: ${day.count} records, ${day.attentionCount} Error/Fatal`}
								>
									<div
										className="bg-destructive/85"
										style={{
											height: `${(day.attentionCount / maximum) * 100}%`,
										}}
									/>
									<div
										className="bg-primary/70"
										style={{
											height: `${((day.count - day.attentionCount) / maximum) * 100}%`,
										}}
									/>
									{!day.count && <div className="h-px bg-border" />}
								</div>
							))}
						</div>
						<div
							className="mt-2 flex justify-between gap-2 text-[10px] text-muted-foreground"
							aria-hidden="true"
						>
							<span>{formatDay(statistics.buckets[0].day)}</span>
							{statistics.days > 1 && (
								<span>
									{formatDay(statistics.buckets[statistics.days - 1].day)}
								</span>
							)}
						</div>
					</>
				) : (
					<HomeEmpty>No sampled executions fall in this period.</HomeEmpty>
				)}
				<div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-muted-foreground">
					<span className="flex items-center gap-1.5">
						<span className="size-2 rounded-sm bg-primary/70" />
						Other recorded severities
					</span>
					<span className="flex items-center gap-1.5">
						<span className="size-2 rounded-sm bg-destructive/85" />
						Error / Fatal
					</span>
				</div>
			</div>
			<div className="sr-only">
				<table>
					<caption>
						Daily execution records in the retrieved sample, grouped in UTC
					</caption>
					<thead>
						<tr>
							<th scope="col">Date</th>
							<th scope="col">Records</th>
							<th scope="col">Error or Fatal severity</th>
						</tr>
					</thead>
					<tbody>
						{statistics.buckets.map((day) => (
							<tr key={day.day}>
								<th scope="row">{day.day}</th>
								<td>{day.count}</td>
								<td>{day.attentionCount}</td>
							</tr>
						))}
					</tbody>
				</table>
			</div>
			<p className="shrink-0 border-t px-4 py-2 text-[10px] leading-relaxed text-muted-foreground">
				{homeActivityCoverage(statistics)} Your account on this backend.
			</p>
		</figure>
	);
}

export function HomeExecutionsByApp({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const library = useHomeLibrary();
	const results = useHomeExecutions(
		textConfig(widget.config, "appId"),
		100,
		true,
		editing,
	);
	if (!backend.usageState)
		return (
			<HomeEmpty>Usage history is not available on this backend.</HomeEmpty>
		);
	if (results.isLoading || results.isError || !results.data)
		return (
			<HomeQueryState
				loading={results.isLoading}
				error={results.isError}
				retry={() => void results.refetch()}
			/>
		);
	const statistics = summarizeHomeExecutions(results.data, widget.config.days);
	const names = new Map(
		(library.data ?? []).map(([app, meta]) => [app.id, meta?.name ?? app.id]),
	);
	const apps = statistics.apps.slice(
		0,
		numberConfig(widget.config, "limit", 5),
	);
	const maximum = Math.max(1, ...apps.map((app) => app.count));
	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
				{apps.length ? (
					apps.map((app) => {
						const content = (
							<div key={app.appId ?? "unassigned"} className="min-w-0 flex-1">
								<div className="flex items-center justify-between gap-3 text-xs">
									<span className="truncate font-medium">
										{app.appId
											? (names.get(app.appId) ?? app.appId)
											: "Unassigned executions"}
									</span>
									<span className="shrink-0 tabular-nums">
										{app.count.toLocaleString()} records
									</span>
								</div>
								<div
									className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
									aria-hidden="true"
								>
									<div
										className="h-full rounded-full bg-primary/70"
										style={{ width: `${(app.count / maximum) * 100}%` }}
									/>
								</div>
								{app.attentionCount > 0 && (
									<p className="mt-1.5 text-[10px] text-destructive">
										{app.attentionCount.toLocaleString()} with Error / Fatal
										severity
									</p>
								)}
							</div>
						);
						return app.appId ? (
							<Link
								key={`app:${app.appId}`}
								href={`/library/config/analytics?id=${encodeURIComponent(app.appId)}`}
								className={homeItemClass}
							>
								{content}
							</Link>
						) : (
							<div key="unassigned" className={homeItemClass}>
								{content}
							</div>
						);
					})
				) : (
					<HomeEmpty>No sampled executions fall in this period.</HomeEmpty>
				)}
			</div>
			<p className="shrink-0 border-t px-4 py-2 text-[10px] leading-relaxed text-muted-foreground">
				{homeActivityCoverage(statistics)} Your account on this backend.
				{apps.length < statistics.apps.length &&
					` Showing ${apps.length} of ${statistics.apps.length} app groups.`}
			</p>
		</div>
	);
}

export function HomeAiUsage({ editing }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const summary = useQuery({
		queryKey: ["home", ...scope, "usage-summary"],
		queryFn: () => {
			if (!backend.usageState)
				throw new Error("Usage history is unavailable on this backend.");
			return backend.usageState.getUsageSummary();
		},
		enabled: Boolean(backend.usageState),
		staleTime: 30_000,
		refetchInterval: editing ? false : 60_000,
	});
	if (!backend.usageState)
		return (
			<HomeEmpty>Usage history is not available on this backend.</HomeEmpty>
		);
	if (summary.isLoading || summary.isError || !summary.data)
		return (
			<HomeQueryState
				loading={summary.isLoading}
				error={summary.isError}
				retry={() => void summary.refetch()}
			/>
		);
	const usage = summary.data;
	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="grid min-h-0 flex-1 grid-cols-2 content-center gap-x-5 gap-y-4 overflow-auto p-4">
				{[
					{
						label: "AI requests",
						value: usage.total_llm_invocations.toLocaleString(),
					},
					{
						label: "Embedding requests",
						value: usage.total_embedding_invocations.toLocaleString(),
					},
					{
						label: "Recorded AI cost",
						value: homeUsageDollars(usage.total_llm_price),
					},
					{
						label: "Recorded embedding cost",
						value: homeUsageDollars(usage.total_embedding_price),
					},
				].map((stat) => (
					<div key={stat.label}>
						<p className="text-[11px] text-muted-foreground">{stat.label}</p>
						<p className="mt-1.5 text-xl font-semibold tabular-nums tracking-tight">
							{stat.value}
						</p>
					</div>
				))}
			</div>
			<p className="shrink-0 border-t px-4 py-2 text-[10px] leading-relaxed text-muted-foreground">
				Account totals reported by this backend across profiles, without a
				period filter. Recorded cost is in USD.
			</p>
		</div>
	);
}

export function HomeNeedsAttention(props: HomeContentProps) {
	return (
		<div className="flex h-full min-h-0 flex-col">
			<section
				className="flex min-h-0 flex-1 flex-col"
				aria-label="Execution records needing review"
			>
				<p className="shrink-0 px-4 pt-3 text-xs font-medium">
					Error / Fatal records
				</p>
				<div className="min-h-0 flex-1">
					<HomeRecentRuns {...props} />
				</div>
			</section>
			<section
				className="flex min-h-0 flex-1 flex-col border-t"
				aria-label="Unread workflow notifications"
			>
				<p className="shrink-0 px-4 pt-3 text-xs font-medium">
					Unread workflow notifications
				</p>
				<div className="min-h-0 flex-1">
					<HomeNotifications {...props} />
				</div>
			</section>
		</div>
	);
}

export function HomeRunStats({ widget }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const metric = textConfig(widget.config, "metric", "overview");
	const sample = metric === "errors" || metric === "duration";
	const executions = useHomeExecutions(
		textConfig(widget.config, "appId"),
		100,
		sample,
	);
	const summary = useQuery({
		queryKey: ["home", ...scope, "usage-summary"],
		queryFn: () => {
			if (!backend.usageState)
				throw new Error("Usage history is unavailable on this backend.");
			return backend.usageState.getUsageSummary();
		},
		enabled: !sample && Boolean(backend.usageState),
		staleTime: 30_000,
	});
	const state = sample ? executions : summary;
	if (!backend.usageState)
		return (
			<HomeEmpty>Usage history is not available on this backend.</HomeEmpty>
		);
	if (state.isLoading || state.isError)
		return (
			<HomeQueryState
				loading={state.isLoading}
				error={state.isError}
				retry={() => void state.refetch()}
			/>
		);
	const rows = executions.data?.items ?? [];
	const stats = sample
		? [
				{
					label:
						metric === "errors"
							? "Error-severity executions"
							: "Average recorded duration",
					value: rows.length
						? metric === "errors"
							? rows
									.filter((row) =>
										["error", "fatal"].includes(row.status.toLowerCase()),
									)
									.length.toLocaleString()
							: `${(rows.reduce((sum, row) => sum + row.microseconds, 0) / rows.length / 1000).toLocaleString(undefined, { maximumFractionDigits: 0 })} ms`
						: "No records",
				},
			]
		: [
				{
					label: "Recorded executions",
					value: (summary.data?.total_executions ?? 0).toLocaleString(),
					id: "executions",
				},
				{
					label: "AI requests",
					value: (summary.data?.total_llm_invocations ?? 0).toLocaleString(),
					id: "ai",
				},
				{
					label: "Embedding requests",
					value: (
						summary.data?.total_embedding_invocations ?? 0
					).toLocaleString(),
					id: "embeddings",
				},
			].filter((item) => metric === "overview" || item.id === metric);
	return (
		<div className="flex h-full flex-col justify-center p-4">
			<div className="grid grid-cols-[repeat(auto-fit,minmax(110px,1fr))] gap-5">
				{stats.map((stat) => (
					<div key={stat.label}>
						<p className="text-xs text-muted-foreground">{stat.label}</p>
						<p className="mt-2 text-3xl font-semibold tabular-nums tracking-tight">
							{stat.value}
						</p>
					</div>
				))}
			</div>
			<p className="mt-4 text-[10px] text-muted-foreground">
				{sample
					? `Your latest ${rows.length} recorded executions${executions.data && executions.data.total > rows.length ? ` of ${executions.data.total.toLocaleString()}` : ""}.`
					: "Your account's recorded usage across profiles on this backend."}
			</p>
		</div>
	);
}

export function HomeRecentRuns({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const attention = widget.type === "needs-attention";
	const limit = numberConfig(widget.config, "limit", 8);
	const appId = textConfig(widget.config, "appId");
	const results = useHomeExecutions(
		appId,
		attention ? 100 : limit,
		true,
		editing,
	);
	const library = useHomeLibrary();
	const names = new Map(
		(library.data ?? []).map(([app, meta]) => [app.id, meta?.name ?? app.id]),
	);
	if (!backend.usageState)
		return (
			<HomeEmpty>Usage history is not available on this backend.</HomeEmpty>
		);
	if (results.isLoading || results.isError)
		return (
			<HomeQueryState
				loading={results.isLoading}
				error={results.isError}
				retry={() => void results.refetch()}
			/>
		);
	const statistics = results.data
		? summarizeHomeExecutions(results.data, widget.config.days)
		: null;
	const rows = attention
		? (statistics?.rows
				.filter((row) => hasAttentionSeverity(row.status))
				.slice(0, limit) ?? [])
		: (results.data?.items ?? []);
	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
				{!rows.length && (
					<HomeEmpty icon={<Activity className="size-7 opacity-50" />}>
						{attention
							? `No Error / Fatal records in the sample for ${homeActivityDays(widget.config.days) === 1 ? "today" : `the last ${homeActivityDays(widget.config.days)} days`}.`
							: "No execution records are available for your account yet."}
					</HomeEmpty>
				)}
				{rows.map((run) => {
					const error = hasAttentionSeverity(run.status);
					const Icon = error ? AlertCircle : Activity;
					const contents = (
						<Fragment key={run.id}>
							<Icon
								className={cn(
									"size-4 shrink-0",
									error ? "text-destructive" : "text-primary",
								)}
							/>
							<div className="min-w-0 flex-1">
								<p className="truncate text-sm font-medium">
									{run.app_id
										? (names.get(run.app_id) ?? "App execution")
										: "Execution"}
								</p>
								<p className="mt-1 text-[11px] text-muted-foreground">
									{new Date(run.created_at).toLocaleString()}
								</p>
							</div>
							<div className="shrink-0 text-right">
								<span
									className={cn(
										"rounded-full bg-muted px-2 py-0.5 text-[10px]",
										error && "bg-destructive/10 text-destructive",
									)}
								>
									{run.status}
								</span>
								<p className="mt-1 text-[10px] tabular-nums text-muted-foreground">
									{(run.microseconds / 1000).toLocaleString(undefined, {
										maximumFractionDigits: 0,
									})}{" "}
									ms
								</p>
							</div>
						</Fragment>
					);
					return run.app_id ? (
						<Link
							key={run.id}
							href={`/library/config/analytics?id=${encodeURIComponent(run.app_id)}`}
							className={homeItemClass}
						>
							{contents}
						</Link>
					) : (
						<div key={run.id} className={homeItemClass}>
							{contents}
						</div>
					);
				})}
			</div>
			<p className="shrink-0 border-t px-4 py-2 text-[10px] text-muted-foreground">
				{attention && statistics
					? homeActivityCoverage(statistics)
					: "Your recorded executions. Badges show the recorded log severity."}
			</p>
		</div>
	);
}

export function HomeSchedules({ widget }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const chosen = stringList(widget.config, "appIds");
	const appIds = (library.data ?? [])
		.filter(([app]) => !chosen.length || chosen.includes(app.id))
		.map(([app]) => app.id);
	const names = new Map(
		(library.data ?? []).map(([app, metadata]) => [
			app.id,
			metadata?.name ?? app.id,
		]),
	);
	const schedules = useQuery({
		queryKey: ["home", ...scope, "schedules", appIds],
		enabled: Boolean(library.data),
		queryFn: async () => {
			const rows: {
				id: string;
				appId: string;
				title: string;
				next: Date;
				timezone: string;
			}[] = [];
			let unavailable = 0;
			for (let start = 0; start < appIds.length; start += 5) {
				const batch = await Promise.allSettled(
					appIds.slice(start, start + 5).map(async (appId) => ({
						appId,
						events: await backend.eventState.getEvents(appId),
					})),
				);
				for (const result of batch) {
					if (result.status === "rejected") {
						unavailable++;
						continue;
					}
					for (const event of result.value.events) {
						if (!event.active || event.event_type !== "cron") continue;
						try {
							const config = parseUint8ArrayToJson(event.config) ?? {};
							const next = nextHomeSchedule(config);
							if (next)
								rows.push({
									id: event.id,
									appId: result.value.appId,
									title: event.name,
									next,
									timezone: config.timezone || "UTC",
								});
						} catch {
							unavailable++;
						}
					}
				}
			}
			return {
				rows: rows.sort((a, b) => a.next.getTime() - b.next.getTime()),
				unavailable,
			};
		},
		staleTime: 60_000,
		refetchInterval: 60_000,
	});
	if (
		library.isLoading ||
		library.isError ||
		schedules.isLoading ||
		schedules.isError
	)
		return (
			<HomeQueryState
				loading={library.isLoading || schedules.isLoading}
				error={library.isError || schedules.isError}
				retry={() => {
					void library.refetch();
					void schedules.refetch();
				}}
			/>
		);
	const rows =
		schedules.data?.rows.slice(0, numberConfig(widget.config, "limit", 8)) ??
		[];
	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
				{rows.length ? (
					rows.map((row) => (
						<Link
							key={`${row.appId}:${row.id}`}
							href={`/library/config/events?id=${encodeURIComponent(row.appId)}`}
							className={homeItemClass}
						>
							<CalendarDays className="size-4 shrink-0 text-primary" />
							<div className="min-w-0 flex-1">
								<p className="truncate text-sm font-medium">{row.title}</p>
								<p className="mt-1 truncate text-xs text-muted-foreground">
									{names.get(row.appId)}
								</p>
							</div>
							<div className="shrink-0 text-right text-xs">
								<p>
									{row.next.toLocaleDateString(undefined, {
										month: "short",
										day: "numeric",
									})}
								</p>
								<p className="mt-1 flex items-center justify-end gap-1 text-muted-foreground">
									<Clock className="size-3" />
									{row.next.toLocaleTimeString(undefined, {
										hour: "2-digit",
										minute: "2-digit",
									})}
								</p>
							</div>
						</Link>
					))
				) : (
					<HomeEmpty>No upcoming schedules were found in these apps.</HomeEmpty>
				)}
			</div>
			<p className="shrink-0 border-t px-4 py-2 text-[10px] text-muted-foreground">
				Calculated from active app schedules. Times use your device timezone.
				{Boolean(schedules.data?.unavailable) &&
					" Some schedules could not be loaded."}
			</p>
		</div>
	);
}
