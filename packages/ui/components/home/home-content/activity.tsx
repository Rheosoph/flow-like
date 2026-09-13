"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Activity,
	AlertCircle,
	ArrowUpRight,
	CalendarDays,
	Check,
	CheckCircle2,
	Clock,
} from "lucide-react";
import Link from "next/link";
import { Fragment, useMemo } from "react";
import { toast } from "sonner";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { NotificationIcon } from "../../notifications/notification-icon";
import { Button } from "../../ui/button";
import {
	hasAttentionSeverity,
	homeActivityCoverage,
	homeActivityDays,
	homeActivityPeriod,
	homeActivitySourceLabel,
	homeDurationMs,
	homeUsageDollars,
	normalizeHomeActivity,
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
	HomeSourceNote,
	homeRowClass,
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
	const kind = type === "WORKFLOW" || type === "SYSTEM" ? type : undefined;
	const notifications = useQuery({
		queryKey: ["home", ...scope, "notifications", unread, type, limit],
		// The kind is applied by the source, so asking for one cannot come back
		// empty just because the newest rows are all of the other kind.
		queryFn: () => backend.userState.listNotifications(unread, kind, 0, limit),
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
	const rows = (notifications.data ?? []).slice(0, limit);
	if (!rows.length)
		return (
			<HomeEmpty icon={<CheckCircle2 className="size-7 text-emerald-500/70" />}>
				{attention
					? "No unread workflow notifications."
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
		<div className="flex min-w-0 flex-col gap-3">
			<div className="min-w-0 divide-y divide-border/50">
				{rows.map((notification) => {
					const href =
						safeHomeHref(notification.link ?? "") ?? "/notifications";
					return (
						<div
							key={notification.id}
							className={cn(
								homeRowClass,
								"items-start",
								!notification.read && "bg-primary/[0.04]",
							)}
						>
							<NotificationIcon
								icon={notification.icon}
								appId={notification.app_id}
								read={notification.read}
								className="mt-0.5 size-5"
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
				className="flex items-center justify-end gap-1.5 border-t border-border/50 pt-3 text-xs text-muted-foreground transition-colors hover:text-foreground"
			>
				Open inbox
				<ArrowUpRight className="size-3.5" />
			</Link>
		</div>
	);
}

/** Newest N records, for a widget that genuinely wants a list rather than counts. */
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

/**
 * Counts for a whole period. Every widget showing a total, a per-day bar, a
 * per-app split or a flagged list reads this rather than a page of records:
 * a page can only report its own size, which is what made a busy week render
 * as a single day.
 */
export function useHomeActivity(
	days: unknown,
	appId = "",
	enabled = true,
	editing = false,
) {
	const backend = useBackend();
	const scope = useHomeScope();
	const window = homeActivityDays(days);
	return useQuery({
		queryKey: ["home", ...scope, "activity", appId, window],
		queryFn: () => {
			if (!backend.usageState)
				throw new Error("Usage history is unavailable on this backend.");
			return backend.usageState.getExecutionActivity(
				window,
				appId || undefined,
			);
		},
		select: normalizeHomeActivity,
		enabled: enabled && Boolean(backend.usageState),
		staleTime: 30_000,
		refetchInterval: editing ? false : 60_000,
	});
}

export function HomeRunActivity({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const results = useHomeActivity(
		widget.config.days,
		textConfig(widget.config, "appId"),
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
	const activity = results.data;
	const maximum = Math.max(1, ...activity.buckets.map((day) => day.count));
	const formatDay = (day: string) =>
		new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
			month: "short",
			day: "numeric",
			timeZone: "UTC",
		});
	return (
		<figure className="flex min-w-0 flex-col gap-3">
			<figcaption className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 text-xs">
				<span className="text-muted-foreground">
					{homeActivityPeriod(activity.days)}
					{" · UTC"}
				</span>
				<span className="font-medium tabular-nums">
					{activity.total.toLocaleString()}{" "}
					{activity.total === 1 ? "record" : "records"}
				</span>
			</figcaption>
			<div className="flex min-w-0 flex-col">
				{activity.total ? (
					<>
						<div
							className="flex h-44 min-h-44 items-end gap-1.5 border-b border-border/60 bg-[linear-gradient(to_top,var(--border)_1px,transparent_1px)] bg-[size:100%_25%]"
							aria-hidden="true"
						>
							{activity.buckets.map((day) => (
								<div
									key={day.day}
									className="flex h-full min-w-0 flex-1 flex-col justify-end overflow-hidden rounded-t-sm"
									title={`${formatDay(day.day)}: ${day.count} records, ${day.attentionCount} Error/Fatal`}
								>
									<div
										className="bg-destructive/85"
										style={{
											height: `${(day.attentionCount / maximum) * 100}%`,
										}}
									/>
									<div
										className="bg-[var(--home-accent,var(--primary))] opacity-80"
										style={{
											height: `${((day.count - day.attentionCount) / maximum) * 100}%`,
										}}
									/>
									{!day.count && <div className="h-px bg-border" />}
								</div>
							))}
						</div>
						{activity.buckets.length > 0 && (
							<div
								className="mt-2 flex justify-between gap-2 text-[10px] text-muted-foreground"
								aria-hidden="true"
							>
								<span>{formatDay(activity.buckets[0].day)}</span>
								{activity.buckets.length > 1 && (
									<span>
										{formatDay(
											activity.buckets[activity.buckets.length - 1].day,
										)}
									</span>
								)}
							</div>
						)}
					</>
				) : (
					<HomeEmpty>No executions were recorded in this period.</HomeEmpty>
				)}
				{activity.total > 0 && (
					<div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-muted-foreground">
						<span className="flex items-center gap-1.5">
							<span className="size-2 rounded-sm bg-[var(--home-accent,var(--primary))] opacity-80" />
							Other recorded severities
						</span>
						<span className="flex items-center gap-1.5">
							<span className="size-2 rounded-sm bg-destructive/85" />
							Error / Fatal
						</span>
					</div>
				)}
			</div>
			<div className="sr-only">
				<table>
					<caption>
						Daily execution records for this period, grouped in UTC
					</caption>
					<thead>
						<tr>
							<th scope="col">Date</th>
							<th scope="col">Records</th>
							<th scope="col">Error or Fatal severity</th>
						</tr>
					</thead>
					<tbody>
						{activity.buckets.map((day) => (
							<tr key={day.day}>
								<th scope="row">{day.day}</th>
								<td>{day.count}</td>
								<td>{day.attentionCount}</td>
							</tr>
						))}
					</tbody>
				</table>
			</div>
			<HomeSourceNote label={homeActivitySourceLabel(activity)}>
				{homeActivityCoverage(activity)} Your account on this backend.
			</HomeSourceNote>
		</figure>
	);
}

export function HomeExecutionsByApp({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const library = useHomeLibrary();
	const results = useHomeActivity(
		widget.config.days,
		textConfig(widget.config, "appId"),
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
	const activity = results.data;
	const names = new Map(
		(library.data ?? []).map(([app, meta]) => [app.id, meta?.name ?? app.id]),
	);
	const apps = activity.apps.slice(0, numberConfig(widget.config, "limit", 5));
	const maximum = Math.max(1, ...apps.map((app) => app.count));
	return (
		<div className="flex min-w-0 flex-col gap-3">
			<div className="min-w-0 divide-y divide-border/50">
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
										className="h-full rounded-full bg-[var(--home-accent,var(--primary))] opacity-80"
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
								className={homeRowClass}
							>
								{content}
							</Link>
						) : (
							<div key="unassigned" className={homeRowClass}>
								{content}
							</div>
						);
					})
				) : (
					<HomeEmpty>No executions were recorded in this period.</HomeEmpty>
				)}
			</div>
			<HomeSourceNote label={homeActivitySourceLabel(activity)}>
				{homeActivityCoverage(activity)} Your account on this backend.
				{apps.length < activity.apps.length &&
					` Showing ${apps.length} of ${activity.apps.length} app groups.`}
			</HomeSourceNote>
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
		<div className="flex min-w-0 flex-col gap-3">
			<div className="grid min-w-0 grid-cols-2 gap-x-5 gap-y-5 py-1">
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
					<div key={stat.label} className="min-w-0">
						<p className="text-[11px] text-muted-foreground">{stat.label}</p>
						<p className="mt-1.5 break-words text-2xl font-semibold tabular-nums tracking-tight">
							{stat.value}
						</p>
					</div>
				))}
			</div>
			<HomeSourceNote label="Your account · all recorded usage · USD">
				Account totals reported by this backend across profiles, without a
				period filter. Recorded cost is in USD.
			</HomeSourceNote>
		</div>
	);
}

export function HomeNeedsAttention(props: HomeContentProps) {
	return (
		<div className="flex min-w-0 flex-col gap-3">
			<section
				className="flex min-w-0 flex-col gap-2"
				aria-label="Execution records needing review"
			>
				<p className="text-[11px] font-medium text-muted-foreground">
					Error / Fatal records
				</p>
				<div className="min-w-0">
					<HomeRecentRuns {...props} />
				</div>
			</section>
			<section
				className="flex min-w-0 flex-col gap-2 border-t border-border/50 pt-3"
				aria-label="Unread workflow notifications"
			>
				<p className="text-[11px] font-medium text-muted-foreground">
					Unread workflow notifications
				</p>
				<div className="min-w-0">
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
	const executions = useHomeActivity(
		widget.config.days,
		textConfig(widget.config, "appId"),
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
	const activity = executions.data;
	const stats = sample
		? [
				{
					label:
						metric === "errors"
							? "Error-severity executions"
							: "Average recorded duration",
					value: !activity
						? "No records"
						: metric === "errors"
							? activity.attentionTotal.toLocaleString()
							: homeDurationMs(activity.averageMicroseconds),
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
		<div className="flex min-w-0 flex-col gap-4 py-1">
			<div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,110px),1fr))] gap-5">
				{stats.map((stat) => (
					<div key={stat.label} className="min-w-0">
						<p className="text-xs text-muted-foreground">{stat.label}</p>
						<p className="mt-2 break-words text-3xl font-semibold tabular-nums tracking-tight">
							{stat.value}
						</p>
					</div>
				))}
			</div>
			<HomeSourceNote
				label={
					sample && activity
						? homeActivitySourceLabel(activity)
						: "Your account · all recorded usage"
				}
			>
				{sample && activity
					? homeActivityCoverage(activity)
					: "Your account's recorded usage across profiles on this backend."}
			</HomeSourceNote>
		</div>
	);
}

export function HomeRecentRuns({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const attention = widget.type === "needs-attention";
	const limit = numberConfig(widget.config, "limit", 8);
	const appId = textConfig(widget.config, "appId");
	// Two shapes, two sources. "Recent runs" wants the newest records, which a
	// page answers exactly. "Needs attention" wants the flagged ones, which a
	// page of newest records cannot answer: they may all sit past its end.
	const history = useHomeExecutions(appId, limit, !attention, editing);
	const flagged = useHomeActivity(
		widget.config.days,
		appId,
		attention,
		editing,
	);
	const results = attention ? flagged : history;
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
	const activity = attention ? (flagged.data ?? null) : null;
	const rows = attention
		? (activity?.attention.slice(0, limit) ?? [])
		: (history.data?.items ?? []);
	return (
		<div className="flex min-w-0 flex-col gap-3">
			<div className="min-w-0 divide-y divide-border/50">
				{!rows.length && (
					<HomeEmpty icon={<Activity className="size-7 opacity-50" />}>
						{attention
							? `No Error / Fatal records were recorded for ${homeActivityPeriod(homeActivityDays(widget.config.days)).toLowerCase()}.`
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
							className={homeRowClass}
						>
							{contents}
						</Link>
					) : (
						<div key={run.id} className={homeRowClass}>
							{contents}
						</div>
					);
				})}
			</div>
			<HomeSourceNote
				label={
					activity
						? homeActivitySourceLabel(activity)
						: "Your account · recorded log severity"
				}
			>
				{activity
					? `${homeActivityCoverage(activity)}${
							activity.attentionCapped
								? ` Showing the newest ${rows.length.toLocaleString()}.`
								: ""
						}`
					: "Your recorded executions. Badges show the recorded log severity."}
			</HomeSourceNote>
		</div>
	);
}

export function HomeSchedules({ widget }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const chosen = stringList(widget.config, "appIds");
	const names = new Map(
		(library.data ?? []).map(([app, metadata]) => [
			app.id,
			metadata?.name ?? app.id,
		]),
	);
	const limit = numberConfig(widget.config, "limit", 8);
	const schedules = useQuery({
		queryKey: ["home", ...scope, "schedules"],
		queryFn: () => backend.eventState.getUserSchedules(),
		staleTime: 60_000,
		refetchInterval: 60_000,
	});
	// The response covers every app whose events this account can read, which is
	// a wider set than the widget shows: it is narrowed to the profile library
	// so an app outside this profile cannot appear, and then to the widget's own
	// app picker. Both filters stay here because neither is a property of the
	// account.
	const visible = useMemo(() => {
		const inLibrary = new Set((library.data ?? []).map(([app]) => app.id));
		const rows = (schedules.data?.schedules ?? [])
			.filter((schedule) => inLibrary.has(schedule.app_id))
			.filter((schedule) => !chosen.length || chosen.includes(schedule.app_id))
			.flatMap((schedule) => {
				try {
					const next = nextHomeSchedule(
						(schedule.config ?? {}) as Record<string, unknown>,
					);
					return next
						? [
								{
									id: schedule.event_id,
									appId: schedule.app_id,
									title: schedule.name,
									next,
									timezone: schedule.config?.timezone || "UTC",
								},
							]
						: [];
				} catch {
					return [];
				}
			});
		return rows.sort((a, b) => a.next.getTime() - b.next.getTime());
	}, [schedules.data, library.data, chosen]);
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
	const rows = visible.slice(0, limit);
	return (
		<div className="flex min-w-0 flex-col gap-3">
			<div className="min-w-0 divide-y divide-border/50">
				{rows.length ? (
					rows.map((row) => (
						<Link
							key={`${row.appId}:${row.id}`}
							href={`/library/config/events?id=${encodeURIComponent(row.appId)}`}
							className={homeRowClass}
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
			<HomeSourceNote label="Active app schedules · your local time">
				Calculated from the active schedules of{" "}
				{(schedules.data?.apps_checked ?? 0).toLocaleString()} apps you can read
				events for. Times use your device timezone.
				{Boolean(schedules.data?.unreadable) &&
					" Some schedules could not be read."}
				{schedules.data?.truncated && " More schedules exist than were listed."}
			</HomeSourceNote>
		</div>
	);
}
