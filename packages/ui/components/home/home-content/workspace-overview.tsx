"use client";

import { useQuery } from "@tanstack/react-query";
import {
	Activity,
	ArrowRight,
	ArrowUpRight,
	Boxes,
	CircleAlert,
	Compass,
	Info,
	Layers3,
	Loader2,
	RefreshCw,
	Sparkles,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import type { ReactNode } from "react";
import { useAuth } from "react-oidc-context";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { useGlobalChatStore } from "../../../state/global-chat/global-chat-store";
import {
	homeActivityCoverage,
	homeActivityDays,
} from "../home-activity-statistics";
import { useHomeLibrary } from "./collections";
import type { HomeContentProps } from "./config";
import { HomeSourceNote, useHomeScope } from "./shared";
import {
	workspaceProfileAppCount,
	workspacePulseHistory,
	workspacePulseMetrics,
	workspacePulseState,
} from "./workspace-overview-model";

const actionClass =
	"group flex min-w-0 items-center gap-3 rounded-xl border border-border/60 bg-background/30 px-3 py-3 text-left transition-colors hover:border-primary/30 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function HomeWorkspacePulse({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const router = useRouter();
	const auth = useAuth();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const profile = useQuery({
		queryKey: ["home", ...scope, "profile-apps"],
		queryFn: () => backend.userState.getProfile(),
		staleTime: 60_000,
	});
	const setDraft = useGlobalChatStore((state) => state.setDraft);
	const enabled = auth.isAuthenticated && Boolean(backend.usageState);
	const executions = useQuery({
		queryKey: ["home", ...scope, "executions", "", 100],
		queryFn: () => {
			if (!backend.usageState) throw new Error("Usage history is unavailable.");
			return backend.usageState.getExecutionHistory(0, 100);
		},
		enabled,
		staleTime: 30_000,
		refetchInterval: editing ? false : 60_000,
		retry: false,
	});
	const statistics = workspacePulseHistory(
		enabled ? executions.data : undefined,
		widget.config.days,
	);
	const state = workspacePulseState({
		authenticated: auth.isAuthenticated,
		supported: Boolean(backend.usageState),
		loading: executions.isLoading,
		error: executions.isError,
		volume: statistics?.volume,
	});
	const appCount = workspaceProfileAppCount(
		library.data?.map(([app]) => app.id),
		profile.data
			? (profile.data.apps ?? []).map((app) => app.app_id)
			: undefined,
	);
	const names = new Map(
		(library.data ?? []).map(([app, metadata]) => [
			app.id,
			metadata?.name ?? "App execution",
		]),
	);
	const days = homeActivityDays(widget.config.days);
	const period = days === 1 ? "Today" : `Last ${days} days`;
	const showAttention = widget.config.showAttention !== false;
	const maximum = Math.max(
		1,
		...(statistics?.buckets.map((bucket) => bucket.count) ?? []),
	);
	const formatDay = (day: string) =>
		new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
			month: "short",
			day: "numeric",
			timeZone: "UTC",
		});
	if (widget.config.mode === "attention") {
		return (
			<WorkspacePulseAttention
				title={widget.title || "Needs attention"}
				statistics={workspacePulseMetrics(
					statistics,
					enabled,
					executions.isError,
				)}
				state={state}
				local={!backend.usageState}
				names={names}
				onRetry={() => void executions.refetch()}
			/>
		);
	}
	if (widget.config.mode === "strip") {
		return (
			<WorkspacePulseStrip
				title={widget.title || "Your profile at a glance"}
				profileName={backend.profile?.name || "Your profile"}
				appCount={appCount}
				appLoading={library.isLoading || profile.isLoading}
				appError={library.isError || profile.isError}
				statistics={workspacePulseMetrics(
					statistics,
					enabled,
					executions.isError,
				)}
				state={state}
				local={!backend.usageState}
				showAttention={showAttention}
				onRetry={() => void executions.refetch()}
			/>
		);
	}
	return (
		<div
			className="flex min-w-0 flex-col gap-4 p-5"
			data-workspace-pulse={state}
		>
			<div className="flex min-w-0 items-start justify-between gap-3">
				<div className="min-w-0">
					<p className="mb-1.5 truncate text-[10px] font-medium uppercase leading-4 tracking-[0.16em] text-muted-foreground">
						{backend.profile?.name || "Your profile"} · overview
					</p>
					<h2 className="text-xl font-semibold leading-7 tracking-tight">
						{widget.title || "Your workspace"}
					</h2>
				</div>
				<span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-violet-500/10 text-[var(--home-surface-accent)]">
					<Activity className="size-4" aria-hidden="true" />
				</span>
			</div>
			{state === "activity" && statistics ? (
				<>
					<div
						className={cn(
							"grid gap-3",
							showAttention ? "grid-cols-3" : "grid-cols-2",
						)}
					>
						<PulseStat
							value={appCount?.toLocaleString()}
							label="Apps in library"
							href="/library"
						/>
						<PulseStat
							value={statistics.volume.toLocaleString()}
							label={
								statistics.partial ? "Sampled records" : "Execution records"
							}
						/>
						{showAttention && (
							<PulseStat
								value={statistics.attention.length.toLocaleString()}
								label="Flagged records"
								accent={statistics.attention.length > 0}
							/>
						)}
					</div>
					<figure className="min-w-0">
						<figcaption className="mb-3 flex flex-wrap items-center justify-between gap-2 text-[10px] text-muted-foreground">
							<span>{period} · your account · UTC</span>
							<span className="flex items-center gap-1.5">
								<span className="size-1.5 rounded-full bg-[var(--home-accent)]" />
								Records
								<span className="ml-1 size-1.5 rounded-full bg-orange-500" />
								Error / Fatal
							</span>
						</figcaption>
						<div
							className="flex h-24 items-end gap-1.5 border-b border-border/60 bg-[linear-gradient(to_top,var(--border)_1px,transparent_1px)] bg-[size:100%_50%]"
							role="img"
							aria-label={`${statistics.volume} execution records, including ${statistics.attention.length} Error or Fatal records. ${period}, UTC. ${statistics.partial ? "Counts are from a limited sample." : "All available records checked."}`}
						>
							{statistics.buckets.map((bucket) => (
								<div
									key={bucket.day}
									className="flex h-full min-w-0 flex-1 flex-col justify-end"
									title={`${formatDay(bucket.day)}: ${bucket.count} records, ${bucket.attentionCount} Error / Fatal`}
								>
									<div
										className="w-full max-w-10 self-center rounded-t-sm bg-orange-500"
										style={{
											height: `${(bucket.attentionCount / maximum) * 100}%`,
										}}
									/>
									<div
										className="w-full max-w-10 self-center bg-[var(--home-accent)]/85"
										style={{
											height: `${((bucket.count - bucket.attentionCount) / maximum) * 100}%`,
											borderRadius: bucket.attentionCount
												? undefined
												: "3px 3px 0 0",
										}}
									/>
								</div>
							))}
						</div>
						<div className="mt-2 flex justify-between text-[10px] tabular-nums text-muted-foreground">
							<span>{formatDay(statistics.buckets[0].day)}</span>
							{days > 1 && (
								<span>
									{formatDay(
										statistics.buckets[statistics.buckets.length - 1].day,
									)}
								</span>
							)}
						</div>
					</figure>
					{showAttention && statistics.attention.length > 0 && (
						<div className="rounded-xl border border-orange-500/15 bg-orange-500/[0.04] p-3">
							<div className="mb-2 flex items-center gap-2 text-xs font-medium">
								<CircleAlert className="size-3.5 text-orange-500" />
								Worth a look
							</div>
							{statistics.attention.slice(0, 2).map((record) => (
								<Link
									key={record.id}
									href={
										record.app_id
											? `/library/config/analytics?id=${encodeURIComponent(record.app_id)}`
											: "/library"
									}
									className="group flex min-w-0 items-center justify-between gap-3 rounded-lg py-1.5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
								>
									<div className="min-w-0">
										<p className="truncate text-xs font-medium leading-4">
											{record.app_id
												? (names.get(record.app_id) ?? "App execution")
												: "Execution"}
										</p>
										<p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
											{record.status} ·{" "}
											{new Date(record.created_at).toLocaleDateString(
												undefined,
												{ month: "short", day: "numeric" },
											)}
										</p>
									</div>
									<ArrowUpRight className="size-3.5 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground" />
								</Link>
							))}
						</div>
					)}
					<HomeSourceNote
						label={`${statistics.partial ? "Sample: " : "Checked "}${statistics.scanned.toLocaleString()} of ${statistics.total.toLocaleString()} account records`}
					>
						{homeActivityCoverage(statistics)} Flagged counts use recorded Error
						/ Fatal severity. These are execution records, not a workflow
						success rate. The library count belongs to this profile.
					</HomeSourceNote>
				</>
			) : (
				<>
					<div className="flex items-center gap-4 rounded-xl bg-violet-500/[0.06] p-4">
						<div className="flex size-12 shrink-0 items-center justify-center rounded-xl border border-violet-400/20 bg-background/50 text-[var(--home-surface-accent)]">
							<Layers3 className="size-6" aria-hidden="true" />
						</div>
						<div className="min-w-0">
							<p className="text-[clamp(1.4rem,4vw,1.8rem)] font-semibold leading-tight tracking-tight">
								{appCount === undefined
									? "Make room for an idea"
									: appCount === 0
										? "Start with one useful app"
										: `${appCount.toLocaleString()} ${appCount === 1 ? "app" : "apps"}, ready to open`}
							</p>
							<p className="mt-1 text-xs leading-relaxed text-muted-foreground">
								{appCount
									? "Your library is a good place to pick up."
									: "Find a starting point, then make it yours."}
							</p>
						</div>
					</div>
					<div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,180px),1fr))] gap-2.5">
						<Link
							href={appCount ? "/library" : "/store/explore/apps"}
							className={actionClass}
						>
							<Compass className="size-4 shrink-0 text-[var(--home-surface-accent)]" />
							<span className="min-w-0 flex-1">
								<span className="block text-xs font-medium">
									{appCount ? "Open your library" : "Find an app"}
								</span>
								<span className="mt-0.5 block text-[11px] text-muted-foreground">
									{appCount
										? "Apps and projects in this profile"
										: "Explore what you can build on"}
								</span>
							</span>
							<ArrowRight className="size-3.5 shrink-0 text-muted-foreground" />
						</Link>
						<button
							type="button"
							disabled={editing}
							onClick={() => {
								setDraft({
									prompt: "Help me turn a task I do often into a useful app.",
								});
								router.push("/chat");
							}}
							className={cn(actionClass, "disabled:opacity-50")}
						>
							<Sparkles className="size-4 shrink-0 text-orange-500" />
							<span className="min-w-0 flex-1">
								<span className="block text-xs font-medium">
									Build with FlowPilot
								</span>
								<span className="mt-0.5 block text-[11px] text-muted-foreground">
									Bring an idea or a recurring task
								</span>
							</span>
							<ArrowRight className="size-3.5 shrink-0 text-muted-foreground" />
						</button>
					</div>
					<div className="flex items-center justify-between gap-3 text-[10px] leading-relaxed text-muted-foreground">
						<p className="flex min-w-0 items-center gap-1.5 leading-4">
							{state === "loading" ? (
								<Loader2 className="size-3 shrink-0 animate-spin" />
							) : (
								<Boxes className="size-3 shrink-0" />
							)}
							{state === "loading"
								? "Checking recent activity…"
								: state === "unavailable"
									? "Recent activity is temporarily unavailable."
									: !enabled
										? "Open this profile’s library to manage your apps."
										: `No execution records found in the recent sample for ${days === 1 ? "today" : `the last ${days} days`}.`}
						</p>
						{state === "unavailable" && (
							<button
								type="button"
								onClick={() => void executions.refetch()}
								className="flex shrink-0 items-center gap-1 rounded text-xs hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								<RefreshCw className="size-3" />
								Retry
							</button>
						)}
					</div>
				</>
			)}
		</div>
	);
}

function WorkspacePulseAttention({
	title,
	statistics,
	state,
	local,
	names,
	onRetry,
}: {
	title: string;
	statistics: ReturnType<typeof workspacePulseHistory>;
	state: ReturnType<typeof workspacePulseState>;
	local: boolean;
	names: Map<string, string>;
	onRetry: () => void;
}) {
	return (
		<section
			className="flex min-w-0 flex-col gap-4 p-5"
			aria-label={title}
			data-workspace-pulse={state}
			data-workspace-mode="attention"
		>
			<div className="flex items-center justify-between gap-3">
				<h2 className="flex min-w-0 items-center gap-2 text-base font-semibold leading-6 tracking-tight">
					<span
						className={cn(
							"size-2 shrink-0 rounded-full",
							statistics?.attention.length
								? "bg-orange-500"
								: "bg-[var(--home-accent)]",
						)}
					/>
					{title}
				</h2>
				{statistics && statistics.attention.length > 0 && (
					<span className="rounded-full bg-orange-500/10 px-2 py-0.5 text-[11px] font-medium tabular-nums text-orange-600 dark:text-orange-400">
						{statistics.attention.length.toLocaleString()}
					</span>
				)}
			</div>
			{statistics && statistics.attention.length > 0 ? (
				<div className="space-y-2">
					{statistics.attention.slice(0, 3).map((record) => (
						<Link
							key={record.id}
							aria-label={`Open app activity for ${record.app_id ? (names.get(record.app_id) ?? "app execution") : "execution"}. ${record.status}, ${new Date(record.created_at).toLocaleString()}`}
							href={
								record.app_id
									? `/library/config/analytics?id=${encodeURIComponent(record.app_id)}`
									: "/library"
							}
							className="group flex min-w-0 items-start gap-3 rounded-xl border border-border/60 bg-background/35 px-3 py-2 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						>
							<CircleAlert
								className="mt-0.5 size-3.5 shrink-0 text-orange-500"
								aria-hidden="true"
							/>
							<div className="min-w-0 flex-1">
								<div className="flex min-w-0 items-center justify-between gap-2">
									<p className="truncate text-xs font-medium leading-4">
										{record.app_id
											? (names.get(record.app_id) ?? "App execution")
											: "Execution"}
									</p>
									<ArrowUpRight
										className="size-3 shrink-0 text-muted-foreground transition-colors group-hover:text-orange-500"
										aria-hidden="true"
									/>
								</div>
								<p className="mt-1 text-[11px] leading-4 text-muted-foreground">
									{record.status} ·{" "}
									{new Date(record.created_at).toLocaleString(undefined, {
										month: "short",
										day: "numeric",
										hour: "2-digit",
										minute: "2-digit",
									})}
								</p>
							</div>
						</Link>
					))}
				</div>
			) : (
				<div className="flex min-w-0 items-start gap-3 py-1">
					<span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-violet-500/[0.07] text-[var(--home-surface-accent)]">
						{state === "loading" ? (
							<Loader2 className="size-4 animate-spin" aria-hidden="true" />
						) : (
							<Layers3 className="size-4" aria-hidden="true" />
						)}
					</span>
					<div className="min-w-0 flex-1">
						<p className="text-xs font-medium leading-5">
							{statistics
								? "No flagged records in this sample"
								: state === "loading"
									? "Checking recent records…"
									: state === "unavailable"
										? "Activity is temporarily unavailable"
										: local
											? "Your local workspace is ready"
											: "Account activity appears here"}
						</p>
						<p className="mt-1 text-[11px] leading-5 text-muted-foreground">
							{statistics
								? "No Error or Fatal severity was recorded in the checked period."
								: state === "loading"
									? "Flags will appear after your account history loads."
									: state === "unavailable"
										? "Try loading your account records again."
										: local
											? "Open your library to continue. Account execution history is unavailable in this workspace."
											: "Sign in to see execution records with Error or Fatal severity."}
						</p>
						{state === "unavailable" ? (
							<button
								type="button"
								onClick={onRetry}
								className="mt-3 flex items-center gap-1 text-xs font-medium text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								<RefreshCw className="size-3" aria-hidden="true" />
								Retry activity
							</button>
						) : (
							<Link
								href="/library"
								className="mt-3 inline-flex items-center gap-1 rounded text-xs font-medium text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								Open library{" "}
								<ArrowRight className="size-3" aria-hidden="true" />
							</Link>
						)}
					</div>
				</div>
			)}
			{statistics && (
				<HomeSourceNote
					label={`${statistics.days === 1 ? "Today" : `Last ${statistics.days} days`} · ${statistics.scanned.toLocaleString()} of ${statistics.total.toLocaleString()} account records checked`}
				>
					{homeActivityCoverage(statistics)} Flags use recorded Error / Fatal
					severity. Multiple records can belong to the same execution.
				</HomeSourceNote>
			)}
		</section>
	);
}

function WorkspacePulseStrip({
	title,
	profileName,
	appCount,
	appLoading,
	appError,
	statistics,
	state,
	local,
	showAttention,
	onRetry,
}: {
	title: string;
	profileName: string;
	appCount: number | undefined;
	appLoading: boolean;
	appError: boolean;
	statistics: ReturnType<typeof workspacePulseHistory>;
	state: ReturnType<typeof workspacePulseState>;
	local: boolean;
	showAttention: boolean;
	onRetry: () => void;
}) {
	const cellClass =
		"flex min-w-0 flex-col justify-between gap-2 bg-[var(--home-surface-background)] px-4 py-3.5 @[800px]/workspace:px-5";
	const labelClass =
		"flex min-w-0 items-center gap-1.5 text-[10px] font-medium uppercase leading-4 tracking-[0.1em] text-muted-foreground";
	const metricCellClass = cn(cellClass, "justify-start");
	const metricLabelClass = cn(labelClass, "min-h-8 @[800px]/workspace:min-h-4");
	const captionClass = "text-[11px] leading-4 text-muted-foreground";
	const period =
		statistics?.days === 1 ? "Today" : `Last ${statistics?.days ?? 7} days`;
	const maximum = Math.max(
		1,
		...(statistics?.buckets.map((bucket) => bucket.count) ?? []),
	);
	const libraryCaption =
		appCount !== undefined
			? "In this profile"
			: appError
				? "Library unavailable"
				: appLoading
					? "Checking your library…"
					: "Open your library";
	return (
		<section
			className="@container/workspace min-w-0"
			aria-label={title}
			data-workspace-pulse={state}
			data-workspace-mode="strip"
		>
			<div
				className={cn(
					"grid min-w-0 grid-cols-1 gap-px overflow-hidden rounded-2xl border border-border/60 bg-border/60 @[280px]/workspace:grid-cols-2",
					statistics && !showAttention
						? "@[800px]/workspace:grid-cols-3"
						: "@[800px]/workspace:grid-cols-4",
				)}
			>
				<Link
					href="/library"
					className={cn(
						metricCellClass,
						"group transition-colors hover:bg-muted/70 focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
					)}
				>
					<span className={metricLabelClass}>
						<Layers3
							className="size-3.5 shrink-0 text-[var(--home-surface-accent)]"
							aria-hidden="true"
						/>
						Your apps
						<ArrowUpRight
							className="ml-auto size-3.5 shrink-0 opacity-50 transition-opacity group-hover:opacity-100"
							aria-hidden="true"
						/>
					</span>
					<StripValue>
						{appCount?.toLocaleString() ?? (
							<span
								aria-label={
									appLoading ? "Loading app count" : "App count unavailable"
								}
							>
								···
							</span>
						)}
					</StripValue>
					<span className={captionClass}>{libraryCaption}</span>
				</Link>
				{statistics ? (
					<>
						<div className={metricCellClass}>
							<span className={metricLabelClass}>
								<Activity
									className="size-3.5 shrink-0 text-[var(--home-surface-accent)]"
									aria-hidden="true"
								/>
								{statistics.partial ? "Sampled records" : "Execution records"}
							</span>
							<StripValue>{statistics.volume.toLocaleString()}</StripValue>
							<span className={captionClass}>{period} · your account</span>
						</div>
						{showAttention && (
							<div className={metricCellClass}>
								<span className={metricLabelClass}>
									<CircleAlert
										className={cn(
											"size-3.5 shrink-0",
											statistics.attention.length > 0 && "text-orange-500",
										)}
										aria-hidden="true"
									/>
									Flagged records
								</span>
								<StripValue accent={statistics.attention.length > 0}>
									{statistics.attention.length.toLocaleString()}
								</StripValue>
								<span className={captionClass}>Error / Fatal severity</span>
							</div>
						)}
						<figure
							className={cn(
								cellClass,
								!showAttention &&
									"@[280px]/workspace:col-span-2 @[800px]/workspace:col-span-1",
							)}
						>
							<figcaption className={labelClass}>{period} · UTC</figcaption>
							{statistics.volume > 0 ? (
								<div
									className="flex h-9 items-end gap-1"
									role="img"
									aria-label={`${statistics.volume} execution records, including ${statistics.attention.length} Error or Fatal records. ${homeActivityCoverage(statistics)}`}
								>
									{statistics.buckets.map((bucket) => (
										<div
											key={bucket.day}
											className="flex h-full min-w-0 flex-1 flex-col justify-end"
											title={`${bucket.day}: ${bucket.count} records, ${bucket.attentionCount} Error / Fatal`}
										>
											<div
												className="w-full max-w-7 self-center rounded-t-sm bg-orange-500"
												style={{
													height: `${(bucket.attentionCount / maximum) * 100}%`,
												}}
											/>
											<div
												className={cn(
													"w-full max-w-7 self-center bg-[var(--home-accent)]/85",
													!bucket.attentionCount && "rounded-t-sm",
												)}
												style={{
													height: `${((bucket.count - bucket.attentionCount) / maximum) * 100}%`,
												}}
											/>
										</div>
									))}
								</div>
							) : (
								<div className="flex min-h-9 items-center gap-2 text-xs font-medium leading-4">
									<span className="size-1.5 shrink-0 rounded-full bg-[var(--home-accent)]" />
									No recent records{statistics.partial ? " in sample" : ""}
								</div>
							)}
							<details className="group/coverage text-[10px] leading-4 text-muted-foreground">
								<summary className="flex cursor-pointer list-none items-center justify-between gap-1 rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
									<span>
										{statistics.scanned.toLocaleString()} of{" "}
										{statistics.total.toLocaleString()} records checked
									</span>
									<Info className="size-3 shrink-0" aria-hidden="true" />
								</summary>
								<p className="mt-2 text-[11px] leading-4">
									{homeActivityCoverage(statistics)} Flagged records use Error /
									Fatal severity. The app count belongs to this profile.
								</p>
							</details>
						</figure>
					</>
				) : (
					<>
						<div className={cellClass}>
							<span className={labelClass}>
								<Boxes className="size-3.5 shrink-0" aria-hidden="true" />
								Workspace
							</span>
							<span
								className="truncate text-lg font-semibold leading-7 tracking-tight"
								title={profileName}
							>
								{profileName}
							</span>
							<span className={captionClass}>
								{local
									? "Local workspace"
									: appCount === 0
										? "Ready to explore"
										: "Your personal starting point"}
							</span>
						</div>
						<div className={cn(cellClass, "@[280px]/workspace:col-span-2")}>
							<span className={labelClass}>
								<Activity
									className="size-3.5 shrink-0 text-[var(--home-surface-accent)]"
									aria-hidden="true"
								/>
								Recent activity
							</span>
							<div className="flex min-w-0 items-center justify-between gap-3">
								<p className="flex min-w-0 items-center gap-2 text-sm font-medium leading-5">
									{state === "loading" && (
										<Loader2
											className="size-3.5 shrink-0 animate-spin"
											aria-hidden="true"
										/>
									)}
									{state === "loading"
										? "Checking account history…"
										: state === "unavailable"
											? "Activity is temporarily unavailable"
											: local
												? "Continue in your library"
												: "Sign in to see account activity"}
								</p>
								{state === "unavailable" && (
									<button
										type="button"
										onClick={onRetry}
										className="flex shrink-0 items-center gap-1 rounded text-xs text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
									>
										<RefreshCw className="size-3" aria-hidden="true" />
										Retry
									</button>
								)}
							</div>
							<span className={captionClass}>
								{state === "loading"
									? "Account records are separate from your profile library."
									: state === "unavailable"
										? "Try again to load your account’s execution records."
										: local
											? "Account execution history is unavailable in this workspace."
											: "Execution records and flagged activity appear here."}
							</span>
						</div>
					</>
				)}
			</div>
		</section>
	);
}

function StripValue({
	children,
	accent,
}: { children: ReactNode; accent?: boolean }) {
	return (
		<span
			className={cn(
				"text-[1.75rem] font-semibold leading-9 tracking-tight tabular-nums",
				accent && "text-orange-500",
			)}
		>
			{children}
		</span>
	);
}

function PulseStat({
	value,
	label,
	href,
	accent,
}: { value?: string; label: string; href?: string; accent?: boolean }) {
	const content = (
		<>
			<span
				className={cn(
					"block text-[clamp(1.6rem,4vw,2.1rem)] font-semibold leading-tight tracking-tight tabular-nums",
					accent && "text-orange-500",
				)}
			>
				{value ?? "···"}
			</span>
			<span className="mt-1 block text-[10px] leading-relaxed text-muted-foreground">
				{label}
			</span>
		</>
	);
	return href ? (
		<Link
			href={href}
			className="min-w-0 rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
		>
			{content}
		</Link>
	) : (
		<div className="min-w-0">{content}</div>
	);
}
