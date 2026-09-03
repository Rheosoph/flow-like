"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	ArchiveRestoreIcon,
	BanIcon,
	CheckCircle2Icon,
	CircleXIcon,
	CloudIcon,
	HardDriveIcon,
	HistoryIcon,
	InfoIcon,
	Loader2Icon,
	MoveRightIcon,
	ScrollIcon,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { formatDuration, formatRelativeTime } from "../../../lib/date";
import {
	type IEventVersionRunAggregate,
	aggregateNodeSeverity,
	aggregateRunsByEventVersion,
	diffTimelineEntries,
} from "../../../lib/event-history";
import type { INode } from "../../../lib/schema/flow/board";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type {
	IEventTimeline,
	IEventTimelineEntry,
	IEventTimelineRun,
	IRestorePlanResult,
} from "../../../state/backend-state/event-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Checkbox,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../ui";

const RUNS_PAGE_LIMIT = 200;

// Never invoked — `enabled` requires the real method; these only satisfy
// useInvoke's non-optional function parameter.
async function timelineUnavailable(
	_appId: string,
	_eventId: string,
): Promise<IEventTimeline> {
	throw new Error("Event timeline is not supported on this platform");
}
async function runsUnavailable(
	_appId: string,
	_eventId: string,
	_boardIds: string[],
	_options?: { limit?: number; offset?: number },
): Promise<IEventTimelineRun[]> {
	return [];
}
async function restoreUnavailable(
	_appId: string,
	_eventId: string,
	_version: [number, number, number],
	_options?: IRestoreCallOptions,
): Promise<IRestorePlanResult> {
	throw new Error("Event restore is not supported on this platform");
}

type IRestoreCallOptions = {
	dryRun?: boolean;
	versionType?: string;
	restoreRoute?: boolean;
	dropCanary?: boolean;
	acceptBlankSecrets?: boolean;
};

function RunSeverityIcon({ level }: Readonly<{ level: number }>) {
	if (level >= 4) return <BanIcon className="h-3 w-3 text-red-800" />;
	if (level === 3) return <CircleXIcon className="h-3 w-3 text-red-500" />;
	if (level === 2)
		return <AlertTriangleIcon className="h-3 w-3 text-yellow-500" />;
	return <CheckCircle2Icon className="h-3 w-3 text-green-500" />;
}

const microsToMs = (micros: number) => Math.floor(micros / 1000);

export function EventHistory({
	appId,
	eventId,
	nodes,
}: Readonly<{
	appId: string;
	eventId: string;
	/** Nodes of the event's current board, for friendly names where available. */
	nodes?: Record<string, INode>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();

	const timelineSupported =
		typeof backend.eventState.getEventTimeline === "function";
	const runsSupported = typeof backend.eventState.listEventRuns === "function";
	const restoreSupported =
		typeof backend.eventState.restoreEvent === "function";

	const timeline = useInvoke<IEventTimeline, [string, string]>(
		backend.eventState.getEventTimeline ?? timelineUnavailable,
		backend.eventState,
		[appId, eventId],
		Boolean(appId && eventId && timelineSupported),
	);
	const timelineData = timeline.data;
	const entries = timelineData?.entries ?? [];
	const boardIds = timelineData?.boards ?? [];

	const runsQuery = useInvoke<
		IEventTimelineRun[],
		[string, string, string[], { limit: number }]
	>(
		backend.eventState.listEventRuns ?? runsUnavailable,
		backend.eventState,
		[appId, eventId, boardIds, { limit: RUNS_PAGE_LIMIT }],
		Boolean(appId && eventId && runsSupported && boardIds.length > 0),
	);
	const runs = useMemo(() => runsQuery.data ?? [], [runsQuery.data]);

	// Rail selection filters the run-derived tabs; clicking again clears it.
	const [selectedKey, setSelectedKey] = useState<string | null>(null);
	const [restoreTarget, setRestoreTarget] =
		useState<IEventTimelineEntry | null>(null);

	const filteredRuns = useMemo(
		() =>
			selectedKey
				? runs.filter((run) => run.event_version === selectedKey)
				: runs,
		[runs, selectedKey],
	);
	const versionAggregates = useMemo(
		() => aggregateRunsByEventVersion(runs),
		[runs],
	);
	const nodeAggregates = useMemo(
		() => aggregateNodeSeverity(filteredRuns),
		[filteredRuns],
	);

	const nodeName = useCallback(
		(nodeId: string) => nodes?.[nodeId]?.friendly_name ?? nodeId,
		[nodes],
	);

	if (!timelineSupported) {
		return (
			<Card>
				<CardContent className="py-10 text-center text-sm text-muted-foreground">
					{t(
						"eventHistoryNotAvailableHere",
						"Version history isn't available on this platform yet.",
					)}
				</CardContent>
			</Card>
		);
	}

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<HistoryIcon className="h-5 w-5" />
					{t("eventHistory", "History")}
				</CardTitle>
				<CardDescription>
					{t(
						"eventHistoryDescription",
						"Every saved version of this event, and what its runs did.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{timeline.isLoading && (
					<div className="flex flex-col items-center justify-center gap-2 py-8">
						<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
						<p className="text-sm text-muted-foreground">
							{t("loadingHistory", "Loading history…")}
						</p>
					</div>
				)}
				{timeline.isError && (
					<div className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
						<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
						<p>
							{t(
								"failedToLoadEventHistory",
								"Could not load the version history: {{val}}",
								{ val: timeline.error?.message ?? "" },
							)}
						</p>
					</div>
				)}
				{timelineData && (
					<div className="grid grid-cols-1 gap-6 lg:grid-cols-[220px_minmax(0,1fr)]">
						<VersionRail
							timeline={timelineData}
							selectedKey={selectedKey}
							onSelect={(key) =>
								setSelectedKey((current) => (current === key ? null : key))
							}
							onRestore={restoreSupported ? setRestoreTarget : undefined}
						/>
						<Tabs defaultValue="overview" className="min-w-0">
							<TabsList>
								<TabsTrigger value="overview">
									{t("overview", "Overview")}
								</TabsTrigger>
								<TabsTrigger value="runs">{t("runs", "Runs")}</TabsTrigger>
								<TabsTrigger value="nodes">{t("nodes", "Nodes")}</TabsTrigger>
								<TabsTrigger value="diff">{t("diff", "Diff")}</TabsTrigger>
							</TabsList>

							<TabsContent value="overview">
								<HistoryOverview
									entries={entries}
									aggregates={versionAggregates}
									selectedKey={selectedKey}
									runsSupported={runsSupported}
									runsFailed={runsQuery.isError}
								/>
							</TabsContent>

							<TabsContent value="runs" className="space-y-3">
								<VersionFilterNotice
									selectedKey={selectedKey}
									onClear={() => setSelectedKey(null)}
								/>
								<HistoryRuns
									runs={filteredRuns}
									isLoading={runsQuery.isLoading}
									runsSupported={runsSupported}
									nodeName={nodeName}
								/>
							</TabsContent>

							<TabsContent value="nodes" className="space-y-3">
								<VersionFilterNotice
									selectedKey={selectedKey}
									onClear={() => setSelectedKey(null)}
								/>
								<HistoryNodes aggregates={nodeAggregates} nodeName={nodeName} />
							</TabsContent>

							<TabsContent value="diff">
								<HistoryDiff entries={entries} />
							</TabsContent>
						</Tabs>
					</div>
				)}
				{restoreTarget && (
					<RestoreDialog
						appId={appId}
						eventId={eventId}
						entry={restoreTarget}
						onClose={() => setRestoreTarget(null)}
					/>
				)}
			</CardContent>
		</Card>
	);
}

function VersionRail({
	timeline,
	selectedKey,
	onSelect,
	onRestore,
}: Readonly<{
	timeline: IEventTimeline;
	selectedKey: string | null;
	onSelect: (key: string) => void;
	onRestore?: (entry: IEventTimelineEntry) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="min-w-0">
			<p className="pb-1.5 text-[10px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">
				{t("versions", "Versions")}
			</p>
			<div className="flex max-h-105 flex-col gap-1 overflow-y-auto pr-1">
				{timeline.entries.map((entry) => {
					const targetMissing = !entry.board_resolves || !entry.node_resolves;
					const isSelected = selectedKey === entry.version_key;
					const restorable = onRestore && !entry.is_live;
					return (
						<div key={entry.version_key} className="relative">
							<button
								type="button"
								aria-pressed={isSelected}
								onClick={() => onSelect(entry.version_key)}
								className={cn(
									"flex w-full flex-col gap-1 rounded-md border px-2.5 py-2 text-left transition-colors",
									restorable && "pr-8",
									isSelected
										? "border-border bg-card shadow-sm ring-1 ring-border"
										: "border-transparent bg-muted/40 hover:bg-muted/60",
								)}
							>
								<span className="flex items-center gap-1.5">
									<span className="font-mono text-xs font-semibold">
										v{entry.version_key}
									</span>
									{entry.is_live && (
										<Badge className="h-4 px-1.5 text-[9.5px]">
											{t("live", "Live")}
										</Badge>
									)}
									{!entry.active && (
										<Badge
											variant="outline"
											className="h-4 px-1.5 text-[9.5px]"
										>
											{t("inactive", "Inactive")}
										</Badge>
									)}
								</span>
								<span className="truncate text-xs text-muted-foreground">
									{formatRelativeTime(entry.updated_at_ms, "narrow")}
								</span>
								{targetMissing && (
									<Badge
										variant="destructive"
										className="h-4 w-fit gap-1 px-1.5 text-[9.5px]"
									>
										<AlertTriangleIcon className="h-2.5 w-2.5" />
										{t("targetMissing", "Target missing")}
									</Badge>
								)}
							</button>
							{restorable && (
								<Tooltip>
									<TooltipTrigger asChild>
										{/* span keeps the tooltip alive on the disabled button */}
										<span className="absolute right-1 top-1">
											<Button
												variant="ghost"
												size="icon"
												className="size-6 text-muted-foreground hover:text-foreground"
												disabled={targetMissing}
												aria-label={t(
													"restoreThisVersion",
													"Restore this version",
												)}
												onClick={() => onRestore(entry)}
											>
												<ArchiveRestoreIcon className="h-3.5 w-3.5" />
											</Button>
										</span>
									</TooltipTrigger>
									<TooltipContent side="right">
										{targetMissing
											? t(
													"cannotRestoreTargetMissing",
													"This version's flow or node no longer exists, so it cannot be restored.",
												)
											: t("restoreThisVersion", "Restore this version")}
									</TooltipContent>
								</Tooltip>
							)}
						</div>
					);
				})}
			</div>
			{timeline.truncated && (
				<p className="pt-2 text-xs text-muted-foreground">
					{t(
						"eventHistoryTruncated",
						"Older versions exist but are not shown.",
					)}
				</p>
			)}
			{timeline.skipped > 0 && (
				<p className="pt-1 text-xs text-muted-foreground">
					{t(
						"eventHistorySkipped",
						"{{skipped}} archived versions could not be loaded.",
						{ skipped: timeline.skipped },
					)}
				</p>
			)}
		</div>
	);
}

function RestoreDialog({
	appId,
	eventId,
	entry,
	onClose,
}: Readonly<{
	appId: string;
	eventId: string;
	entry: IEventTimelineEntry;
	onClose: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [restoreRoute, setRestoreRoute] = useState(false);
	const [dropCanary, setDropCanary] = useState(false);
	const [acceptBlankSecrets, setAcceptBlankSecrets] = useState(false);
	const [isRestoring, setIsRestoring] = useState(false);

	const restoreSupported =
		typeof backend.eventState.restoreEvent === "function";
	const planQuery = useInvoke<
		IRestorePlanResult,
		[string, string, [number, number, number], IRestoreCallOptions]
	>(
		backend.eventState.restoreEvent ?? restoreUnavailable,
		backend.eventState,
		[
			appId,
			eventId,
			entry.version,
			{ dryRun: true, restoreRoute, dropCanary, acceptBlankSecrets },
		],
		restoreSupported,
	);
	const plan = planQuery.data?.plan;
	const blockingIssues = useMemo(
		() => plan?.issues.filter((issue) => issue.severity === "Blocking") ?? [],
		[plan],
	);
	const warningIssues = useMemo(
		() => plan?.issues.filter((issue) => issue.severity === "Warning") ?? [],
		[plan],
	);
	const hasBlankSecret =
		plan?.issues.some((issue) => issue.code === "SecretUnrecoverable") ?? false;

	const fieldLabels: Record<string, string> = {
		name: t("name", "Name"),
		description: t("description", "Description"),
		board_id: t("flow", "Flow"),
		board_version: t("flowVersion", "Flow Version"),
		node_id: t("node", "Node"),
		event_type: t("eventType", "Event type"),
		active: t("active", "Active"),
		priority: t("priority", "Priority"),
		default_page_id: t("page", "Page"),
		notes: t("notes", "Notes"),
		config: t("triggerConfiguration", "Trigger configuration"),
	};
	const fieldLabel = (field: string) =>
		field.startsWith("variables.")
			? `${t("variable", "Variable")} ${field.slice("variables.".length)}`
			: (fieldLabels[field] ?? field);

	const handleConfirm = useCallback(async () => {
		const restoreEvent = backend.eventState.restoreEvent;
		if (!restoreEvent) return;
		setIsRestoring(true);
		try {
			const result = await restoreEvent.call(
				backend.eventState,
				appId,
				eventId,
				entry.version,
				{ dryRun: false, restoreRoute, dropCanary, acceptBlankSecrets },
			);
			toast.success(
				t("eventRestoredAsVersion", "Restored as v{{version}}", {
					version: result.event?.event_version?.join(".") ?? "",
				}),
			);
			if (result.setup_status && result.setup_status !== "ok") {
				toast.warning(
					t("eventRestoreSetupStatus", "Endpoint re-setup: {{status}}", {
						status: result.setup_status,
					}),
				);
			}
			if (backend.eventState.getEventTimeline) {
				await invalidate(backend.eventState.getEventTimeline, [appId, eventId]);
			}
			await invalidate(backend.eventState.getEvents, [appId]);
			await invalidate(backend.eventState.getEvent, [appId, eventId]);
			// Prefix-invalidates every cached dry-run plan for this event.
			await invalidate(
				restoreEvent as unknown as (
					appId: string,
					eventId: string,
				) => Promise<IRestorePlanResult>,
				[appId, eventId],
			);
			onClose();
		} catch (error) {
			toast.error(
				t("failedToRestoreEvent", "Could not restore this version: {{val}}", {
					val: error instanceof Error ? error.message : String(error),
				}),
			);
		} finally {
			setIsRestoring(false);
		}
	}, [
		backend.eventState,
		appId,
		eventId,
		entry.version,
		restoreRoute,
		dropCanary,
		acceptBlankSecrets,
		invalidate,
		onClose,
		t,
	]);

	const optionCheckbox = (
		id: string,
		checked: boolean,
		onChange: (checked: boolean) => void,
		label: string,
		hint: string,
	) => (
		<label
			htmlFor={id}
			className="flex cursor-pointer items-start gap-2 text-sm"
		>
			<Checkbox
				id={id}
				checked={checked}
				onCheckedChange={(value) => onChange(value === true)}
				className="mt-0.5"
			/>
			<span>
				{label}
				<span className="block text-xs text-muted-foreground">{hint}</span>
			</span>
		</label>
	);

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="max-h-[85vh] sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<ArchiveRestoreIcon className="h-4 w-4" />
						{t("restoreVersion", "Restore v{{version}}", {
							version: entry.version_key,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"restoreVersionDescription",
							"Restoring writes a new version whose content matches this snapshot — the current version stays in the history.",
						)}
					</DialogDescription>
				</DialogHeader>
				<DialogBody className="space-y-4">
					{planQuery.isLoading && (
						<div className="flex flex-col items-center justify-center gap-2 py-8">
							<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
							<p className="text-sm text-muted-foreground">
								{t("planningRestore", "Checking what this restore would do…")}
							</p>
						</div>
					)}
					{planQuery.isError && (
						<div className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
							<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
							<p>
								{t(
									"failedToPlanRestore",
									"Could not plan the restore: {{val}}",
									{ val: planQuery.error?.message ?? "" },
								)}
							</p>
						</div>
					)}
					{plan && (
						<>
							{blockingIssues.length > 0 && (
								<div className="space-y-1.5 rounded-lg border border-destructive/40 bg-destructive/10 p-3">
									<p className="flex items-center gap-1.5 text-xs font-semibold text-destructive">
										<BanIcon className="h-3.5 w-3.5" />
										{t("restoreBlocked", "This restore is blocked")}
									</p>
									{blockingIssues.map((issue) => (
										<p
											key={`${issue.code}:${issue.subject ?? issue.message}`}
											className="text-xs"
										>
											{issue.message}
										</p>
									))}
								</div>
							)}
							{warningIssues.length > 0 && (
								<div className="space-y-1.5 rounded-lg border bg-muted/40 p-3">
									<p className="flex items-center gap-1.5 text-xs font-semibold">
										<AlertTriangleIcon className="h-3.5 w-3.5 text-yellow-500" />
										{t("restoreWarnings", "Worth knowing")}
									</p>
									{warningIssues.map((issue) => (
										<p
											key={`${issue.code}:${issue.subject ?? issue.message}`}
											className="text-xs text-muted-foreground"
										>
											{issue.message}
										</p>
									))}
								</div>
							)}
							<div>
								<p className="pb-1.5 text-[10px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">
									{t("restoreChanges", "What changes")}
								</p>
								{plan.diff.length === 0 ? (
									<p className="text-sm text-muted-foreground">
										{t(
											"restoreNoTrackedChanges",
											"The snapshot matches the live event in every compared field.",
										)}
									</p>
								) : (
									<div className="overflow-x-auto rounded-md border">
										<table className="w-full text-sm">
											<tbody>
												{plan.diff.map((change) => (
													<tr
														key={change.field}
														className="border-b last:border-b-0"
													>
														<td className="w-40 px-3 py-2 text-xs font-medium text-muted-foreground">
															{fieldLabel(change.field)}
														</td>
														<td className="px-3 py-2">
															<span className="break-all text-muted-foreground line-through">
																{change.from}
															</span>
														</td>
														<td className="w-6 px-1 py-2 text-muted-foreground">
															<MoveRightIcon className="h-3.5 w-3.5" />
														</td>
														<td className="px-3 py-2 break-all">{change.to}</td>
													</tr>
												))}
											</tbody>
										</table>
									</div>
								)}
							</div>
							<div>
								<p className="pb-1.5 text-[10px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">
									{t("restoreKeptFromLive", "Kept from the live event")}
								</p>
								<div className="flex flex-wrap gap-1">
									{plan.not_restored.map((item) => (
										<Badge
											key={item}
											variant="outline"
											className="font-mono text-[10px]"
										>
											{item}
										</Badge>
									))}
								</div>
							</div>
							<div className="space-y-2.5">
								{optionCheckbox(
									"restore-route",
									restoreRoute,
									setRestoreRoute,
									t("restoreRouteOption", "Also restore the snapshot's route"),
									t(
										"restoreRouteOptionHint",
										"By default the live route and default-route flag are kept.",
									),
								)}
								{optionCheckbox(
									"restore-drop-canary",
									dropCanary,
									setDropCanary,
									t("dropCanaryOption", "Drop the snapshot's canary"),
									t(
										"dropCanaryOptionHint",
										"Without this, the canary configuration stored in the snapshot is restored with it.",
									),
								)}
								{hasBlankSecret &&
									optionCheckbox(
										"restore-accept-blank-secrets",
										acceptBlankSecrets,
										setAcceptBlankSecrets,
										t(
											"acceptBlankSecretsOption",
											"Restore even though some secrets can't be recovered",
										),
										t(
											"acceptBlankSecretsOptionHint",
											"The listed secret variables stay blank until you re-enter their values.",
										),
									)}
							</div>
						</>
					)}
				</DialogBody>
				<DialogFooter>
					<Button variant="outline" onClick={onClose} disabled={isRestoring}>
						{t("cancel", "Cancel")}
					</Button>
					<Button
						onClick={handleConfirm}
						disabled={
							!plan ||
							planQuery.isLoading ||
							isRestoring ||
							blockingIssues.length > 0
						}
					>
						{isRestoring && <Loader2Icon className="h-4 w-4 animate-spin" />}
						{t("restore", "Restore")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function VersionFilterNotice({
	selectedKey,
	onClear,
}: Readonly<{ selectedKey: string | null; onClear: () => void }>) {
	const { t } = useTranslation("settings");
	if (!selectedKey) return null;
	return (
		<div className="flex items-center gap-2 text-xs text-muted-foreground">
			<Badge variant="secondary" className="font-mono">
				v{selectedKey}
			</Badge>
			<button
				type="button"
				className="underline-offset-2 hover:underline"
				onClick={onClear}
			>
				{t("showAllVersions", "Show all versions")}
			</button>
		</div>
	);
}

function HistoryOverview({
	entries,
	aggregates,
	selectedKey,
	runsSupported,
	runsFailed,
}: Readonly<{
	entries: IEventTimelineEntry[];
	aggregates: IEventVersionRunAggregate[];
	selectedKey: string | null;
	runsSupported: boolean;
	runsFailed: boolean;
}>) {
	const { t } = useTranslation("settings");

	const rows = useMemo(() => {
		const byKey = new Map(
			aggregates.map((aggregate) => [aggregate.versionKey, aggregate]),
		);
		const entryKeys = new Set(entries.map((entry) => entry.version_key));
		const result: Array<{
			key: string | null;
			entry?: IEventTimelineEntry;
			aggregate?: IEventVersionRunAggregate;
		}> = entries.map((entry) => ({
			key: entry.version_key,
			entry,
			aggregate: byKey.get(entry.version_key),
		}));
		// Runs whose version no longer has a timeline entry (pruned archives)
		// and unversioned runs still deserve a row.
		for (const aggregate of aggregates) {
			if (aggregate.versionKey === null || !entryKeys.has(aggregate.versionKey))
				result.push({ key: aggregate.versionKey, aggregate });
		}
		return result;
	}, [entries, aggregates]);

	return (
		<div className="space-y-3">
			{!runsSupported && (
				<p className="text-xs text-muted-foreground">
					{t(
						"eventRunHistoryNotAvailableHere",
						"Run history isn't available on this platform yet — showing versions only.",
					)}
				</p>
			)}
			{runsFailed && (
				<p className="text-xs text-destructive">
					{t(
						"failedToLoadEventRuns",
						"Run summaries could not be loaded; the version list is still complete.",
					)}
				</p>
			)}
			<div className="overflow-x-auto rounded-md border">
				<table className="w-full min-w-140 text-sm">
					<thead>
						<tr className="border-b bg-muted/40 text-left text-xs text-muted-foreground">
							<th className="px-3 py-2 font-medium">
								{t("version", "Version")}
							</th>
							<th className="px-3 py-2 text-right font-medium">
								{t("runs", "Runs")}
							</th>
							<th className="px-3 py-2 text-right font-medium">
								{t("ok", "OK")}
							</th>
							<th className="px-3 py-2 text-right font-medium">
								{t("warn", "Warn")}
							</th>
							<th className="px-3 py-2 text-right font-medium">
								{t("failed", "Failed")}
							</th>
							<th className="px-3 py-2 text-right font-medium">p50</th>
							<th className="px-3 py-2 text-right font-medium">p95</th>
							<th className="px-3 py-2 font-medium">
								{t("lastRun", "Last run")}
							</th>
						</tr>
					</thead>
					<tbody>
						{rows.map((row) => {
							const aggregate = row.aggregate;
							const targetMissing =
								row.entry &&
								(!row.entry.board_resolves || !row.entry.node_resolves);
							return (
								<tr
									key={row.key ?? "__unversioned__"}
									className={cn(
										"border-b last:border-b-0",
										selectedKey !== null &&
											selectedKey === row.key &&
											"bg-muted/50",
									)}
								>
									<td className="px-3 py-2">
										<span className="flex items-center gap-1.5">
											<span className="font-mono text-xs">
												{row.key === null
													? t("unversioned", "Unversioned")
													: `v${row.key}`}
											</span>
											{row.entry?.is_live && (
												<Badge className="h-4 px-1.5 text-[9.5px]">
													{t("live", "Live")}
												</Badge>
											)}
											{targetMissing && (
												<span title={t("targetMissing", "Target missing")}>
													<AlertTriangleIcon className="h-3 w-3 text-destructive" />
												</span>
											)}
										</span>
									</td>
									<td className="px-3 py-2 text-right tabular-nums">
										{aggregate?.total ?? 0}
									</td>
									<td className="px-3 py-2 text-right tabular-nums">
										{aggregate?.ok ?? 0}
									</td>
									<td className="px-3 py-2 text-right tabular-nums">
										{aggregate?.warn ?? 0}
									</td>
									<td className="px-3 py-2 text-right tabular-nums">
										{aggregate?.fail ?? 0}
									</td>
									<td className="px-3 py-2 text-right tabular-nums">
										{aggregate ? formatDuration(aggregate.p50DurationUs) : "—"}
									</td>
									<td className="px-3 py-2 text-right tabular-nums">
										{aggregate ? formatDuration(aggregate.p95DurationUs) : "—"}
									</td>
									<td className="px-3 py-2 text-xs text-muted-foreground">
										{aggregate
											? formatRelativeTime(
													microsToMs(aggregate.lastSeen),
													"narrow",
												)
											: "—"}
									</td>
								</tr>
							);
						})}
					</tbody>
				</table>
			</div>
		</div>
	);
}

function HistoryRuns({
	runs,
	isLoading,
	runsSupported,
	nodeName,
}: Readonly<{
	runs: IEventTimelineRun[];
	isLoading: boolean;
	runsSupported: boolean;
	nodeName: (nodeId: string) => string;
}>) {
	const { t } = useTranslation("settings");

	if (!runsSupported) {
		return (
			<p className="py-6 text-center text-sm text-muted-foreground">
				{t(
					"eventRunHistoryNotAvailableHere",
					"Run history isn't available on this platform yet — showing versions only.",
				)}
			</p>
		);
	}
	if (isLoading) {
		return (
			<div className="flex flex-col items-center justify-center gap-2 py-8">
				<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
				<p className="text-sm text-muted-foreground">
					{t("loadingRuns", "Loading runs...")}
				</p>
			</div>
		);
	}
	if (runs.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
				<ScrollIcon className="size-5 text-muted-foreground/60" />
				<p className="text-sm font-medium">{t("noRuns", "No runs")}</p>
				<p className="text-xs text-muted-foreground">
					{t(
						"noRunsRecordedForThisEventYet",
						"No runs have been recorded for this event yet.",
					)}
				</p>
			</div>
		);
	}

	// Row markup copied minimally from FlowRuns so runs read the same everywhere.
	return (
		<div className="flex max-h-105 flex-col gap-2 overflow-y-auto">
			{runs.map((run) => (
				<div
					key={run.run_id}
					className="flex flex-row items-center justify-between gap-2 rounded-md border p-2"
				>
					<div className="flex flex-col items-start justify-center gap-2">
						<div className="flex flex-row items-center gap-2">
							{run.is_remote ? (
								<span title={t("remoteExecution", "Remote execution")}>
									<CloudIcon className="h-3 w-3 text-blue-500" />
								</span>
							) : (
								<span title={t("localExecution", "Local execution")}>
									<HardDriveIcon className="h-3 w-3 text-muted-foreground" />
								</span>
							)}
							<small className="leading-none">{nodeName(run.node_id)}</small>
							<small className="font-mono text-muted-foreground">
								{run.event_version
									? `v${run.event_version}`
									: t("unversioned", "Unversioned")}
							</small>
							<small className="text-muted-foreground">{run.version}</small>
						</div>
						<small className="leading-none text-muted-foreground">
							{formatRelativeTime(microsToMs(run.start), "narrow")}
						</small>
					</div>
					<div className="flex flex-row items-center gap-2">
						<small className="text-muted-foreground">
							{formatDuration(Math.abs(run.end - run.start))}
						</small>
						<RunSeverityIcon level={run.log_level} />
					</div>
				</div>
			))}
		</div>
	);
}

function HistoryNodes({
	aggregates,
	nodeName,
}: Readonly<{
	aggregates: ReturnType<typeof aggregateNodeSeverity>;
	nodeName: (nodeId: string) => string;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-3">
			<div className="flex gap-3 rounded-lg border bg-muted/40 p-3">
				<InfoIcon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
				<p className="text-sm text-muted-foreground">
					{t(
						"nodeTimingsNeedDebugLevel",
						"Visit counts and severity are recorded at every log level. Per-node timings only exist when the flow logs at Debug level, so they are not shown here.",
					)}
				</p>
			</div>
			{aggregates.length === 0 ? (
				<p className="py-4 text-center text-sm text-muted-foreground">
					{t(
						"noNodeActivityRecorded",
						"No node activity recorded in the loaded runs.",
					)}
				</p>
			) : (
				<div className="flex max-h-90 flex-col gap-1.5 overflow-y-auto">
					{aggregates.map((node) => (
						<div
							key={node.nodeId}
							className="flex flex-row items-center justify-between gap-2 rounded-md border p-2 text-sm"
						>
							<div className="flex min-w-0 flex-row items-center gap-2">
								<RunSeverityIcon level={node.worstLevel} />
								<span className="truncate">{nodeName(node.nodeId)}</span>
							</div>
							<div className="flex shrink-0 flex-row items-center gap-3 text-xs text-muted-foreground">
								{node.warnRuns > 0 && (
									<span className="tabular-nums">
										{t("warnRunCount", "{{num}} warn", {
											num: node.warnRuns,
										})}
									</span>
								)}
								{node.failRuns > 0 && (
									<span className="tabular-nums text-destructive">
										{t("failedRunCount", "{{num}} failed", {
											num: node.failRuns,
										})}
									</span>
								)}
								<span className="tabular-nums">
									{t("nodeVisitCount", "{{num}} runs", { num: node.visits })}
								</span>
							</div>
						</div>
					))}
				</div>
			)}
		</div>
	);
}

function HistoryDiff({
	entries,
}: Readonly<{ entries: IEventTimelineEntry[] }>) {
	const { t } = useTranslation("settings");
	const [fromKey, setFromKey] = useState<string | undefined>(undefined);
	const [toKey, setToKey] = useState<string | undefined>(undefined);

	const effectiveFrom = fromKey ?? entries[1]?.version_key;
	const effectiveTo = toKey ?? entries[0]?.version_key;
	const fromEntry = entries.find((e) => e.version_key === effectiveFrom);
	const toEntry = entries.find((e) => e.version_key === effectiveTo);

	const diffs = useMemo(
		() => (fromEntry && toEntry ? diffTimelineEntries(fromEntry, toEntry) : []),
		[fromEntry, toEntry],
	);

	const fieldLabels: Record<string, string> = {
		name: t("name", "Name"),
		description: t("description", "Description"),
		event_type: t("eventType", "Event type"),
		active: t("active", "Active"),
		board: t("flow", "Flow"),
		board_version: t("flowVersion", "Flow Version"),
		node: t("node", "Node"),
		page: t("page", "Page"),
		route: t("routePath", "Route Path"),
		is_default: t("defaultRoute", "Default route"),
		execution_mode: t("execution", "Execution"),
		exposure: t("exposure", "Exposure"),
		variables: t("variables", "Variables"),
		secret_variables: t("secretVariables", "Secret variables"),
		notes: t("notes", "Notes"),
	};

	if (entries.length < 2) {
		return (
			<p className="py-6 text-center text-sm text-muted-foreground">
				{t(
					"needTwoVersionsToCompare",
					"Once this event has more than one version, you can compare them here.",
				)}
			</p>
		);
	}

	const versionPicker = (
		value: string | undefined,
		onChange: (key: string) => void,
		label: string,
	) => (
		<div className="flex min-w-0 items-center gap-2">
			<span className="shrink-0 text-xs text-muted-foreground">{label}</span>
			<Select value={value} onValueChange={onChange}>
				<SelectTrigger size="sm" className="w-36 font-mono text-xs">
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{entries.map((entry) => (
						<SelectItem key={entry.version_key} value={entry.version_key}>
							v{entry.version_key}
							{entry.is_live ? ` (${t("live", "Live")})` : ""}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);

	return (
		<div className="space-y-3">
			<div className="flex flex-wrap items-center gap-3">
				{versionPicker(effectiveFrom, setFromKey, t("compareFrom", "From"))}
				<MoveRightIcon className="h-4 w-4 text-muted-foreground" />
				{versionPicker(effectiveTo, setToKey, t("compareTo", "To"))}
			</div>
			{fromEntry && toEntry && diffs.length === 0 && (
				<p className="py-4 text-center text-sm text-muted-foreground">
					{t(
						"noTrackedDifferences",
						"These versions are identical in every tracked field.",
					)}
				</p>
			)}
			{diffs.length > 0 && (
				<div className="overflow-x-auto rounded-md border">
					<table className="w-full min-w-120 text-sm">
						<tbody>
							{diffs.map((diff) => (
								<tr key={diff.field} className="border-b last:border-b-0">
									<td className="w-40 px-3 py-2 text-xs font-medium text-muted-foreground">
										{fieldLabels[diff.field] ?? diff.field}
									</td>
									<td className="px-3 py-2">
										<span className="break-all text-muted-foreground line-through">
											{diff.from}
										</span>
									</td>
									<td className="w-6 px-1 py-2 text-muted-foreground">
										<MoveRightIcon className="h-3.5 w-3.5" />
									</td>
									<td className="px-3 py-2 break-all">{diff.to}</td>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			)}
			<p className="text-xs text-muted-foreground">
				{t(
					"triggerConfigNotCompared",
					"Type-specific trigger configuration is not part of the timeline and cannot be compared here.",
				)}
			</p>
		</div>
	);
}
