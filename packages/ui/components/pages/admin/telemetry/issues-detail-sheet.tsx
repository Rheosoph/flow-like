"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Check,
	ChevronDown,
	CircleCheck,
	Copy,
	EyeOff,
	RotateCcw,
	Sparkles,
} from "lucide-react";
import { useMemo, useState } from "react";
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts";
import { toast } from "sonner";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	type ChartConfig,
	ChartContainer,
	ChartTooltip,
	ChartTooltipContent,
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
} from "../../../ui";
import {
	IssueLevelBadge,
	IssueStatusBadge,
	frameLocation,
	safeStringify,
} from "./issues-shared";
import { BarList, EmptyState, formatBucketTick } from "./telemetry-shared";
import type {
	ITelemetryIssueBreadcrumb,
	ITelemetryIssueDetail,
	ITelemetryIssueFrame,
	ITelemetryIssuePoint,
	ITelemetryIssueStatus,
	ITelemetryReleasesResponse,
} from "./types";

const issueTrendChartConfig = {
	count: {
		label: "Events",
		color: "var(--chart-1)",
	},
} satisfies ChartConfig;

const NO_RELEASE = "NONE";

function IssueTrendChart({
	points,
}: {
	readonly points: ITelemetryIssuePoint[];
}) {
	if (points.length === 0) {
		return (
			<EmptyState
				message="No events for this issue in the selected window."
				className="h-40 text-xs"
			/>
		);
	}
	return (
		<ChartContainer config={issueTrendChartConfig} className="h-40 w-full">
			<AreaChart
				data={points}
				margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
			>
				<defs>
					<linearGradient id="issueTrendFill" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-count)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-count)"
							stopOpacity={0.03}
						/>
					</linearGradient>
				</defs>
				<CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.4} />
				<XAxis
					dataKey="ts"
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 10 }}
					tickFormatter={(v) => formatBucketTick(v as string, "hour")}
					minTickGap={32}
				/>
				<YAxis
					allowDecimals={false}
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 10 }}
					width={32}
				/>
				<ChartTooltip
					cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
					content={
						<ChartTooltipContent
							indicator="dot"
							labelFormatter={(value) =>
								formatBucketTick(value as string, "hour")
							}
						/>
					}
				/>
				<Area
					type="monotone"
					dataKey="count"
					stroke="var(--color-count)"
					fill="url(#issueTrendFill)"
					strokeWidth={2}
				/>
			</AreaChart>
		</ChartContainer>
	);
}

function StackFrameRow({
	frame,
	dimmed,
}: {
	readonly frame: ITelemetryIssueFrame;
	readonly dimmed: boolean;
}) {
	return (
		<li
			className={`flex flex-wrap items-baseline gap-x-2 px-3 py-1.5 font-mono text-[11px] ${dimmed ? "text-muted-foreground/70" : ""}`}
		>
			<span className={dimmed ? "" : "font-medium text-foreground"}>
				{frame.function ?? "<anonymous>"}
			</span>
			<span className="truncate text-muted-foreground">
				{frameLocation(frame.file, frame.lineno)}
				{frame.colno == null ? "" : `:${frame.colno}`}
			</span>
		</li>
	);
}

interface FrameGroup {
	readonly key: string;
	readonly inApp: boolean;
	readonly frames: ITelemetryIssueFrame[];
}

function groupFrames(frames: ITelemetryIssueFrame[]): FrameGroup[] {
	const groups: FrameGroup[] = [];
	frames.forEach((frame, index) => {
		const inApp = frame.in_app === true;
		const last = groups[groups.length - 1];
		if (last && last.inApp === inApp) {
			last.frames.push(frame);
			return;
		}
		groups.push({ key: `${index}-${inApp}`, inApp, frames: [frame] });
	});
	return groups;
}

function StackTrace({ frames }: { readonly frames: ITelemetryIssueFrame[] }) {
	const { t } = useTranslation("admin");
	if (frames.length === 0) {
		return <EmptyState message="No stack trace attached to this event." />;
	}
	const hasInApp = frames.some((frame) => frame.in_app === true);
	const groups = groupFrames(frames);

	return (
		<div className="overflow-hidden rounded-lg border bg-muted/30">
			{groups.map((group) =>
				group.inApp || !hasInApp ? (
					<ul key={group.key} className="divide-y divide-border/40">
						{group.frames.map((frame, index) => (
							<StackFrameRow
								key={`${group.key}-${index}-${frame.file ?? ""}`}
								frame={frame}
								dimmed={!hasInApp && frame.in_app === false}
							/>
						))}
					</ul>
				) : (
					<Collapsible key={group.key}>
						<CollapsibleTrigger className="group flex w-full items-center gap-1.5 border-y border-border/40 px-3 py-1.5 text-left text-[11px] text-muted-foreground hover:bg-muted/60">
							<ChevronDown className="h-3 w-3 transition-transform group-data-[state=open]:rotate-180" />{t('countSystemFrames', '{{count}} system frame', { count: group.frames.length })}
						</CollapsibleTrigger>
						<CollapsibleContent>
							<ul className="divide-y divide-border/40">
								{group.frames.map((frame, index) => (
									<StackFrameRow
										key={`${group.key}-${index}-${frame.file ?? ""}`}
										frame={frame}
										dimmed
									/>
								))}
							</ul>
						</CollapsibleContent>
					</Collapsible>
				),
			)}
		</div>
	);
}

function BreadcrumbTimeline({
	crumbs,
}: {
	readonly crumbs: ITelemetryIssueBreadcrumb[];
}) {
	if (crumbs.length === 0) {
		return <EmptyState message="No breadcrumbs attached to this event." />;
	}
	return (
		<ol className="space-y-1.5 border-l border-border pl-3">
			{crumbs.map((crumb, index) => (
				<li
					key={`${crumb.ts ?? index}-${crumb.category ?? ""}-${index}`}
					className="relative"
				>
					<span className="absolute -left-[17px] top-1.5 h-1.5 w-1.5 rounded-full bg-muted-foreground/60" />
					<div className="flex flex-wrap items-baseline gap-2">
						<Badge variant="outline" className="font-mono text-[10px]">
							{crumb.category ?? "event"}
						</Badge>
						<span className="text-xs text-foreground">
							{crumb.message ?? "—"}
						</span>
						{crumb.level && crumb.level !== "info" ? (
							<span className="text-[10px] uppercase text-muted-foreground">
								{crumb.level}
							</span>
						) : null}
						{crumb.ts ? (
							<RelativeTime
								value={crumb.ts}
								className="text-[10px] text-muted-foreground"
							/>
						) : null}
					</div>
				</li>
			))}
		</ol>
	);
}

function CopyableJson({
	label,
	value,
}: {
	readonly label: string;
	readonly value: unknown;
}) {
	const { t } = useTranslation("admin");
	const text = useMemo(() => safeStringify(value), [value]);
	return (
		<div className="space-y-1">
			<div className="flex items-center justify-between">
				<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
					{label}
				</div>
				<Button
					variant="ghost"
					size="sm"
					onClick={() => {
						navigator.clipboard.writeText(text).catch(() => null);
						toast.success(`${label} copied`);
					}}
				>
					<Copy className="mr-1 h-3 w-3" />
					{t('copy', 'Copy')}
				</Button>
			</div>
			<pre className="max-h-72 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[11px] leading-relaxed">
				{text}
			</pre>
		</div>
	);
}

interface TelemetryIssueDetailSheetProps {
	issueId: string | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	profile: IProfile | undefined;
}

export function TelemetryIssueDetailSheet({
	issueId,
	open,
	onOpenChange,
	profile,
}: Readonly<TelemetryIssueDetailSheetProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [resolveRelease, setResolveRelease] = useState<string>(NO_RELEASE);

	const detail = useQuery<ITelemetryIssueDetail>({
		queryKey: ["admin", "telemetry", "issues", "detail", issueId],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			if (!issueId) throw new Error("No issue id");
			return backend.apiState.get<ITelemetryIssueDetail>(
				profile,
				`admin/telemetry/issues/${encodeURIComponent(issueId)}`,
			);
		},
		enabled: Boolean(profile && issueId && open),
	});

	const issue = detail.data?.issue;

	const releases = useQuery<ITelemetryReleasesResponse>({
		queryKey: ["admin", "telemetry", "releases", issue?.source ?? "all"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const params = new URLSearchParams({ limit: "50" });
			if (issue?.source) params.set("source", issue.source);
			return backend.apiState.get<ITelemetryReleasesResponse>(
				profile,
				`admin/telemetry/releases?${params.toString()}`,
			);
		},
		enabled: Boolean(profile && issue && open),
	});

	const triage = useMutation({
		mutationFn: async (body: {
			status?: ITelemetryIssueStatus;
			resolved_in_release?: string | null;
		}) => {
			if (!profile) throw new Error("Profile not loaded");
			if (!issueId) throw new Error("No issue id");
			return backend.apiState.patch(
				profile,
				`admin/telemetry/issues/${encodeURIComponent(issueId)}`,
				body,
			);
		},
		onSuccess: async (_data, body) => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "telemetry", "issues"],
			});
			toast.success(
				body.resolved_in_release
					? t('resolvedInResolved_in_release', 'Resolved in {{resolved_in_release}}', { resolved_in_release: body.resolved_in_release })
					: t('issueMarkedVal', 'Issue marked {{val}}', { val: body.status ?? "updated" }),
			);
		},
		onError: (error) => {
			toast.error(
				error instanceof Error ? error.message : t('failedToUpdateTheIssue', 'Failed to update the issue'),
			);
		},
	});

	const latestEvent = detail.data?.latestEvent;
	const frames = latestEvent?.stacktrace ?? [];
	const crumbs = latestEvent?.breadcrumbs ?? [];
	const releaseOptions = useMemo(() => {
		const versions = (releases.data?.releases ?? []).map((r) => r.version);
		if (issue?.lastRelease && !versions.includes(issue.lastRelease)) {
			versions.unshift(issue.lastRelease);
		}
		return versions;
	}, [releases.data?.releases, issue?.lastRelease]);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-2xl">
				<SheetHeader>
					<SheetTitle className="flex flex-wrap items-center gap-2">
						{issue ? <IssueLevelBadge level={issue.level} /> : null}
						<span className="font-mono text-sm">{issue?.kind ?? "Issue"}</span>
						{issue ? <IssueStatusBadge status={issue.status} /> : null}
					</SheetTitle>
					<SheetDescription className="break-words text-xs">
						{issue?.title ?? issueId}
					</SheetDescription>
				</SheetHeader>

				<div className="space-y-4 px-4 pb-6">
					{detail.isLoading || !issue ? (
						<div className="space-y-3">
							<Skeleton className="h-20 w-full" />
							<Skeleton className="h-40 w-full" />
							<Skeleton className="h-40 w-full" />
						</div>
					) : (
						<>
							<div className="space-y-2 rounded-lg border bg-card/40 p-3">
								{issue.culprit ? (
									<code className="block truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]">
										{issue.culprit}
									</code>
								) : null}
								<div className="flex flex-wrap items-center gap-2">
									<Badge variant="outline" className="font-mono text-[10px]">
										{issue.source}
									</Badge>
									{issue.platform ? (
										<Badge variant="secondary" className="text-[10px]">
											{issue.platform}
										</Badge>
									) : null}
									{latestEvent?.symbolicated ? (
										<Badge variant="outline" className="gap-1 text-[10px]">
											<Sparkles className="h-3 w-3" />
											symbolicated
										</Badge>
									) : null}
								</div>
								<div className="grid grid-cols-2 gap-2 text-xs sm:grid-cols-4">
									<div>
										<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
											{t('events', 'Events')}
										</div>
										<div className="font-semibold tabular-nums">
											{issue.eventCount.toLocaleString()}
										</div>
									</div>
									<div>
										<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
											{t('installs', 'Installs')}
										</div>
										<div className="font-semibold tabular-nums">
											{issue.installCount.toLocaleString()}
										</div>
									</div>
									<div>
										<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
											{t('firstSeen2', 'First seen')}
										</div>
										<RelativeTime
											value={issue.firstSeen}
											className="text-muted-foreground"
										/>
									</div>
									<div>
										<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
											{t('lastSeen', 'Last seen')}
										</div>
										<RelativeTime
											value={issue.lastSeen}
											className="text-muted-foreground"
										/>
									</div>
								</div>
								<div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
									<span>
										{t('releases2', 'Releases:')}{" "}
										<span className="font-mono">
											{issue.firstRelease ?? "—"}
										</span>{" "}
										→{" "}
										<span className="font-mono">
											{issue.lastRelease ?? "—"}
										</span>
									</span>
									<span
										className="truncate font-mono"
										title={issue.fingerprint}
									>
										{issue.fingerprint.slice(0, 16)}
									</span>
								</div>
							</div>

							<div className="flex flex-wrap items-center gap-2">
								<Button
									size="sm"
									variant={
										issue.status === "resolved" ? "secondary" : "outline"
									}
									disabled={triage.isPending || issue.status === "resolved"}
									onClick={() =>
										triage.mutate({
											status: "resolved",
											resolved_in_release: null,
										})
									}
								>
									<CircleCheck className="mr-1 h-3.5 w-3.5" />
									{t('resolve', 'Resolve')}
								</Button>
								<Button
									size="sm"
									variant={issue.status === "ignored" ? "secondary" : "outline"}
									disabled={triage.isPending || issue.status === "ignored"}
									onClick={() => triage.mutate({ status: "ignored" })}
								>
									<EyeOff className="mr-1 h-3.5 w-3.5" />
									{t('ignore', 'Ignore')}
								</Button>
								<Button
									size="sm"
									variant="outline"
									disabled={triage.isPending || issue.status === "unresolved"}
									onClick={() =>
										triage.mutate({
											status: "unresolved",
											resolved_in_release: null,
										})
									}
								>
									<RotateCcw className="mr-1 h-3.5 w-3.5" />
									{t('unresolve', 'Unresolve')}
								</Button>
								<div className="flex items-center gap-1.5">
									<Select
										value={resolveRelease}
										onValueChange={setResolveRelease}
										disabled={releaseOptions.length === 0}
									>
										<SelectTrigger className="w-44" size="sm">
											<SelectValue placeholder={t('resolveInRelease', 'Resolve in release')} />
										</SelectTrigger>
										<SelectContent>
											<SelectItem value={NO_RELEASE}>
												{t('resolveInRelease2', 'Resolve in release…')}
											</SelectItem>
											{releaseOptions.map((version) => (
												<SelectItem key={version} value={version}>
													{version}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
									<Button
										size="sm"
										variant="outline"
										disabled={triage.isPending || resolveRelease === NO_RELEASE}
										onClick={() =>
											triage.mutate({
												status: "resolved",
												resolved_in_release: resolveRelease,
											})
										}
									>
										<Check className="h-3.5 w-3.5" />
									</Button>
								</div>
							</div>

							<div className="space-y-1">
								<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
									{t('eventsOverTime', 'Events over time')}
								</div>
								<IssueTrendChart points={detail.data?.timeseries ?? []} />
							</div>

							<div className="space-y-1">
								<div className="flex items-center justify-between">
									<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
										{t('stackTrace', 'Stack trace')}
									</div>
									{latestEvent ? (
										<span className="text-[11px] text-muted-foreground">
											{t('latestEvent', 'latest event ·')}{" "}
											<RelativeTime value={latestEvent.createdAt} />
										</span>
									) : null}
								</div>
								<StackTrace frames={frames} />
							</div>

							<div className="space-y-1">
								<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
									{t('breadcrumbs', 'Breadcrumbs')}
								</div>
								<BreadcrumbTimeline crumbs={crumbs} />
							</div>

							{latestEvent?.context ? (
								<CopyableJson label="Context" value={latestEvent.context} />
							) : (
								<EmptyState message="No context attached to this event." />
							)}

							<div className="grid gap-4 sm:grid-cols-2">
								<div className="space-y-1">
									<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
										{t('releases', 'Releases')}
									</div>
									<BarList
										rows={(detail.data?.releases ?? []).map((r) => ({
											key: r.release,
											label: r.release,
											count: r.count,
										}))}
										emptyMessage="No release attached to these events."
									/>
								</div>
								<div className="space-y-1">
									<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
										{t('platforms', 'Platforms')}
									</div>
									<BarList
										rows={(detail.data?.platforms ?? []).map((p) => ({
											key: p.platform,
											label: p.platform,
											count: p.count,
										}))}
										emptyMessage="No platform attached to these events."
									/>
								</div>
							</div>

							<Separator />

							<p className="text-xs text-muted-foreground">
								{t('crashReportsAreAnonymousTheInstallIdIsARandomIdentifierAndNoUserIdentityOrIpAddressIsEverStored', "Crash reports are anonymous: the install id is a random identifier and no user identity or IP address is ever stored.")}
							</p>
						</>
					)}
				</div>
			</SheetContent>
		</Sheet>
	);
}
