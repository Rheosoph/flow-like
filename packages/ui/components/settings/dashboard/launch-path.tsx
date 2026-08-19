"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRightIcon,
	CheckIcon,
	CopyIcon,
	EyeIcon,
	LockIcon,
	PlayCircleIcon,
	PlusIcon,
	SendIcon,
	SparklesIcon,
	WorkflowIcon,
} from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import type { IApp, IBoardListing } from "../../../lib";
import { IAppVisibility } from "../../../lib";
import { formatDuration, formatRelativeTime } from "../../../lib/date";
import { cn } from "../../../lib/utils";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import { Meter, SectionCard, StateDot } from "./dashboard-primitives";
import type { ProjectSurface } from "./surfaces-table";
import type { ProjectRunHealth } from "./use-project-runs";
import {
	type AiActStatus,
	type AttentionSignal,
	type InspectorPanel,
	type ListingChecklistItem,
	isOnlineVisibility,
} from "./use-project-signals";

type StageState = "done" | "current" | "blocked" | "todo";

function StageRow({
	state,
	title,
	summary,
	badge,
	last,
	children,
}: Readonly<{
	state: StageState;
	title: string;
	summary?: string;
	badge?: ReactNode;
	last?: boolean;
	children?: ReactNode;
}>) {
	return (
		<div className="grid grid-cols-[24px_minmax(0,1fr)] gap-3">
			<div className="flex flex-col items-center">
				<span
					className={cn(
						"mt-1.5 grid h-5 w-5 shrink-0 place-items-center rounded-full border-2",
						state === "done" && "border-emerald-500 bg-emerald-500 text-white",
						state === "current" &&
							"border-primary text-primary ring-4 ring-primary/20",
						(state === "blocked" || state === "todo") &&
							"border-muted-foreground/30 text-muted-foreground",
					)}
				>
					{state === "done" && <CheckIcon className="h-3 w-3" />}
					{state === "current" && (
						<span className="h-1.5 w-1.5 rounded-full bg-primary" />
					)}
					{state === "blocked" && <LockIcon className="h-2.5 w-2.5" />}
				</span>
				{!last && <span className="my-1 w-0.5 flex-1 bg-border" />}
			</div>

			<div className={cn("pb-5", state !== "current" && "opacity-95")}>
				<div className="flex flex-wrap items-center gap-2 py-1">
					<h3
						className={cn(
							"text-sm font-semibold",
							state === "todo" || state === "blocked"
								? "text-muted-foreground"
								: "text-foreground",
						)}
					>
						{title}
					</h3>
					{badge}
					{summary && (
						<span className="ml-auto text-xs text-muted-foreground">
							{summary}
						</span>
					)}
				</div>
				{children}
			</div>
		</div>
	);
}

function RailSignalRow({
	signal,
	onOpenPanel,
}: Readonly<{
	signal: AttentionSignal;
	onOpenPanel: (panel: InspectorPanel) => void;
}>) {
	const { t } = useTranslation("settings");
	const className =
		"flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-muted/60";
	const tone =
		signal.tone === "critical"
			? "critical"
			: signal.tone === "warning"
				? "warn"
				: "idle";
	const body = (
		<>
			<StateDot tone={tone} />
			<span className="min-w-0 flex-1 truncate">{signal.label}</span>
			{signal.stage && (
				<span className="shrink-0 text-[10px] text-muted-foreground">
					{t("stageStage", "Stage {{stage}}", { stage: signal.stage })}
				</span>
			)}
		</>
	);

	if (signal.href) {
		return (
			<Link href={signal.href} className={className}>
				{body}
			</Link>
		);
	}

	return (
		<button
			type="button"
			className={className}
			onClick={() => signal.panel && onOpenPanel(signal.panel)}
		>
			{body}
		</button>
	);
}

function Blocker({
	children,
	action,
}: Readonly<{ children: ReactNode; action?: ReactNode }>) {
	return (
		<div className="flex flex-wrap items-center gap-3 rounded-md border border-dashed border-blue-500/40 bg-blue-500/5 px-3 py-2.5 text-xs">
			<LockIcon className="h-3.5 w-3.5 shrink-0 text-blue-500" />
			<span className="min-w-0 flex-1">{children}</span>
			{action}
		</div>
	);
}

/**
 * The lifecycle dashboard. A young project's real question is "what next", so
 * the page is ordered by dependency: each stage shows only its own controls,
 * finished stages collapse to a summary, and a locked stage states the blocker
 * next to the control that clears it.
 */
export function LaunchPath({
	appId,
	app,
	boards,
	surfaces,
	runs,
	aiAct,
	listing,
	listingDone,
	signals,
	onOpenPanel,
}: Readonly<{
	appId: string;
	app: IApp;
	boards: IBoardListing[];
	surfaces: ProjectSurface[];
	runs: ProjectRunHealth;
	aiAct: AiActStatus;
	listing: ListingChecklistItem[];
	listingDone: number;
	signals: AttentionSignal[];
	onOpenPanel: (panel: InspectorPanel) => void;
}>) {
	const { t } = useTranslation("settings");
	const boardsWithContent = boards.filter((board) => board.nodeCount > 0);
	const hasLogic = boardsWithContent.length > 0;
	const activeSurfaces = surfaces.filter((surface) => surface.active);
	const hasTriggers = activeSurfaces.length > 0;
	const hasRun = runs.hasEverRun;
	const listed = isOnlineVisibility(app.visibility);
	const canInvite = listed;

	const stageStates: StageState[] = [
		hasLogic ? "done" : "current",
		!hasLogic ? "todo" : hasTriggers ? "done" : "current",
		!hasTriggers ? "todo" : hasRun ? "done" : "current",
		canInvite ? "done" : hasRun ? "blocked" : "todo",
		listingDone === listing.length ? "done" : "todo",
		listed && aiAct.hasAssessment ? "done" : "todo",
	];
	const completed = stageStates.filter((state) => state === "done").length;
	const currentIndex = stageStates.findIndex((state) => state === "current");

	const nodeTotal = boards.reduce((sum, board) => sum + board.nodeCount, 0);

	return (
		<div className="grid grid-cols-1 gap-5 lg:grid-cols-[minmax(0,1.9fr)_minmax(0,1fr)]">
			<div>
				<div className="mb-4 flex items-center gap-3">
					<span className="text-xs uppercase tracking-wider text-muted-foreground">
						{t("launchProgress", "Launch progress")}
					</span>
					<span className="text-xs text-muted-foreground">
						{currentIndex >= 0
							? t("stageValOfLength", "stage {{val}} of {{length}}", {
									val: currentIndex + 1,
									length: stageStates.length,
								})
							: t(
									"completedOfLengthComplete",
									"{{completed}} of {{length}} complete",
									{ completed, length: stageStates.length },
								)}
					</span>
					<div className="ml-auto w-40">
						<Meter value={completed} total={stageStates.length} />
					</div>
				</div>

				{/* 1 — build */}
				<StageRow
					state={stageStates[0]}
					title={t("buildTheLogic", "Build the logic")}
					badge={
						stageStates[0] === "done" ? (
							<Badge variant="secondary" className="text-[10px]">
								{t("done", "Done")}
							</Badge>
						) : (
							<Badge className="text-[10px]">
								{t("startHere", "Start here")}
							</Badge>
						)
					}
					summary={
						hasLogic
							? t(
									"lengthFlowvalNodetotalNodes",
									"{{length}} flow{{val}} · {{nodeTotal}} nodes",
									{
										length: boardsWithContent.length,
										val: boardsWithContent.length === 1 ? "" : "s",
										nodeTotal,
									},
								)
							: undefined
					}
				>
					{hasLogic ? (
						<Card className="gap-0 p-1.5">
							{boards.slice(0, 4).map((board) => (
								<Link
									key={board.id}
									href={`/flow?id=${board.id}&app=${appId}`}
									className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
								>
									<WorkflowIcon className="h-3.5 w-3.5 text-muted-foreground" />
									<span className="truncate">{board.name}</span>
									<span className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
										{t("countNodes", {
											defaultValue_one: "{{count}} Node",
											defaultValue_other: "{{count}} Nodes",
											count: board.nodeCount,
										})}
										{board.updatedAt && (
											<span className="hidden md:inline">
												{formatRelativeTime(board.updatedAt, "narrow")}
											</span>
										)}
									</span>
								</Link>
							))}
						</Card>
					) : (
						<Card className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center">
							<p className="max-w-prose text-xs text-muted-foreground">
								{`A flow is the logic your app runs. Start from a blank canvas, a template, or describe what you want and let FlowPilot draft it.`}
							</p>
							<div className="flex shrink-0 flex-wrap gap-2 sm:ml-auto">
								<Link href={`/library/config/flows?id=${appId}`}>
									<Button size="sm">
										<PlusIcon className="mr-1.5 h-3 w-3" />
										{t("newFlow", "New flow")}
									</Button>
								</Link>
								<Link href={`/library/config/templates?id=${appId}`}>
									<Button variant="outline" size="sm">
										<CopyIcon className="mr-1.5 h-3 w-3" />
										{t("fromTemplate", "From template")}
									</Button>
								</Link>
							</div>
						</Card>
					)}
				</StageRow>

				{/* 2 — triggers */}
				<StageRow
					state={stageStates[1]}
					title={t("connectTheTriggers", "Connect the triggers")}
					badge={
						stageStates[1] === "done" ? (
							<Badge variant="secondary" className="text-[10px]">
								{t("done", "Done")}
							</Badge>
						) : stageStates[1] === "current" ? (
							<Badge className="text-[10px]">
								{t("youAreHere", "You are here")}
							</Badge>
						) : undefined
					}
					summary={
						hasTriggers
							? t("lengthActiveOfLength2", "{{length}} active of {{length2}}", {
									length: activeSurfaces.length,
									length2: surfaces.length,
								})
							: hasLogic
								? undefined
								: t("needsAtLeastOneFlow", "Needs at least one flow")
					}
				>
					{surfaces.length > 0 ? (
						<div className="flex flex-wrap gap-2">
							{surfaces.slice(0, 6).map((surface) => (
								<span
									key={surface.id}
									className="flex items-center gap-2 rounded-full border bg-card px-3 py-1 text-xs"
								>
									<StateDot tone={surface.active ? "ok" : "idle"} />
									<span className="truncate">{surface.name}</span>
									<Badge variant="secondary" className="text-[10px]">
										{surface.kind}
									</Badge>
								</span>
							))}
						</div>
					) : hasLogic ? (
						<Card className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center">
							<p className="max-w-prose text-xs text-muted-foreground">
								{t(
									"aTriggerDecidesWhenYourFlowsRunAnIncomingMessageAScheduleAPageSomeoneOpensOrAnApiCall",
									"A trigger decides when your flows run — an incoming message, a schedule, a page someone opens, or an API call.",
								)}
							</p>
							<Link
								href={`/library/config/pages?id=${appId}`}
								className="shrink-0 sm:ml-auto"
							>
								<Button size="sm">
									<SparklesIcon className="mr-1.5 h-3 w-3" />
									{t("setUpEvents", "Set up events")}
								</Button>
							</Link>
						</Card>
					) : null}
				</StageRow>

				{/* 3 — try it */}
				<StageRow
					state={stageStates[2]}
					title={t("tryItEndtoend", "Try it end-to-end")}
					badge={
						stageStates[2] === "done" ? (
							<Badge variant="secondary" className="text-[10px]">
								{t("done", "Done")}
							</Badge>
						) : stageStates[2] === "current" ? (
							<Badge className="text-[10px]">
								{t("youAreHere", "You are here")}
							</Badge>
						) : undefined
					}
					summary={
						hasRun
							? t(
									"windowrunsRunsIn24hval",
									"{{windowRuns}} runs in 24h{{val}}",
									{
										windowRuns: runs.windowRuns,
										val:
											runs.windowFailed > 0
												? ` · ${runs.windowFailed} failed`
												: "",
									},
								)
							: hasTriggers
								? undefined
								: t("needsATrigger", "Needs a trigger")
					}
				>
					{hasTriggers && (
						<Card className="space-y-3 p-4">
							{hasRun ? (
								<div className="space-y-1.5">
									{[...runs.byBoard.entries()]
										.slice(0, 4)
										.map(([id, health]) => {
											const board = boards.find((entry) => entry.id === id);
											return (
												<div
													key={id}
													className="flex items-center gap-2 text-xs"
												>
													<StateDot
														tone={health.failed > 0 ? "critical" : "ok"}
													/>
													<span className="truncate">
														{board?.name ?? t("deletedFlow", "Deleted flow")}
													</span>
													<span className="ml-auto text-muted-foreground">
														{t("countRuns", {
															defaultValue_one: "{{count}} run",
															defaultValue_other: "{{count}} runs",
															count: health.total,
														})}
														{health.failed > 0 && (
															<span className="text-destructive">
																{` · `}
																{t("failedFailed", "{{failed}} failed", {
																	failed: health.failed,
																})}
															</span>
														)}
													</span>
												</div>
											);
										})}
									{runs.p95Micros !== null && (
										<p className="pt-1 text-[11px] text-muted-foreground">
											{t("p95Duration", "p95 duration")}{" "}
											{formatDuration(runs.p95Micros)}
										</p>
									)}
								</div>
							) : (
								<p className="text-xs text-muted-foreground">
									{t(
										"nothingHasRunYetTriggerTheAppOnceToConfirmTheWholeChainWorksBeforeYouShareIt",
										"Nothing has run yet. Trigger the app once to confirm the whole chain works before you share it.",
									)}
								</p>
							)}
							<div className="flex flex-wrap gap-2">
								<Link href={`/use?id=${appId}`}>
									<Button size="sm" variant={hasRun ? "outline" : "default"}>
										<PlayCircleIcon className="mr-1.5 h-3 w-3" />
										{t("runIt", "Run it")}
									</Button>
								</Link>
								<Link href={`/use?id=${appId}`}>
									<Button variant="outline" size="sm">
										<EyeIcon className="mr-1.5 h-3 w-3" />
										{t("previewAsUser", "Preview as user")}
									</Button>
								</Link>
							</div>
						</Card>
					)}
				</StageRow>

				{/* 4 — invite */}
				<StageRow
					state={stageStates[3]}
					title={t("invitePeopleIn", "Invite people in")}
					badge={
						stageStates[3] === "blocked" ? (
							<Badge variant="outline" className="text-[10px]">
								{t("blocked", "Blocked")}
							</Badge>
						) : stageStates[3] === "done" ? (
							<Badge variant="secondary" className="text-[10px]">
								{t("unlocked", "Unlocked")}
							</Badge>
						) : undefined
					}
					summary={t("teamRolesAndShareLinks", "Team, Roles and share links")}
				>
					{canInvite ? (
						<div className="flex flex-wrap gap-2">
							<Link href={`/library/config/team?id=${appId}`}>
								<Button variant="outline" size="sm">
									{t("manageTeam", "Manage team")}
								</Button>
							</Link>
							<Link href={`/library/config/roles?id=${appId}`}>
								<Button variant="outline" size="sm">
									{t("defineRoles", "Define roles")}
								</Button>
							</Link>
						</div>
					) : (
						<Blocker
							action={
								<Button size="sm" onClick={() => onOpenPanel("access")}>
									Change visibility
								</Button>
							}
						>
							{app.visibility === IAppVisibility.Offline
								? t(
										"anOfflineProjectLivesOnlyOnThisDeviceBringItOnlineToInviteCollaboratorsAndAssignRoles",
										"An offline project lives only on this device. Bring it online to invite collaborators and assign roles.",
									)
								: t(
										"aPrivateProjectIsSyncedToYourAccountOnlySwitchToPrototypeToInviteCollaboratorsAssignRolesAndShareALink",
										"A private project is synced to your account only. Switch to Prototype to invite collaborators, assign roles and share a link.",
									)}
						</Blocker>
					)}
				</StageRow>

				{/* 5 — listing */}
				<StageRow
					state={stageStates[4]}
					title={t("writeTheListing", "Write the listing")}
					badge={
						<Badge
							variant={listingDone === listing.length ? "secondary" : "outline"}
							className="text-[10px]"
						>
							{t("listingdoneOfLength", "{{listingDone}} of {{length}}", {
								listingDone,
								length: listing.length,
							})}
						</Badge>
					}
					summary={t("whatPeopleSeeInTheStore", "What people see in the store")}
				>
					<Card className="space-y-3 p-4">
						<Meter
							value={listingDone}
							total={listing.length}
							tone={listingDone === listing.length ? "ok" : "primary"}
						/>
						<div className="grid grid-cols-1 gap-x-4 gap-y-1 sm:grid-cols-2 lg:grid-cols-3">
							{listing.map((item) => (
								<span
									key={item.id}
									className={cn(
										"flex items-center gap-1.5 text-xs",
										item.done ? "text-muted-foreground" : "text-foreground",
									)}
								>
									{item.done ? (
										<CheckIcon className="h-3 w-3 text-emerald-500" />
									) : (
										<PlusIcon className="h-3 w-3 text-muted-foreground" />
									)}
									{item.label}
								</span>
							))}
						</div>
						<Button
							variant="outline"
							size="sm"
							onClick={() => onOpenPanel("listing")}
						>
							{t("editListing", "Edit listing")}
							<ArrowRightIcon className="ml-1.5 h-3 w-3" />
						</Button>
					</Card>
				</StageRow>

				{/* 6 — publish */}
				<StageRow
					state={stageStates[5]}
					title={t("publishGrow", "Publish & grow")}
					last
					badge={
						<Badge variant="outline" className="text-[10px]">
							{listed
								? t("readyToSubmit", "Ready to submit")
								: t("notStarted", "Not started")}
						</Badge>
					}
					summary={t("reviewTakes13Days", "Review takes 1–3 days")}
				>
					<Card className="flex flex-wrap items-center gap-3 p-4 text-xs">
						{aiAct.available ? (
							<span className="flex items-center gap-2">
								<SendIcon className="h-3.5 w-3.5 text-muted-foreground" />
								{t("euAiAct", "EU AI Act")}
								{aiAct.hasAssessment ? (
									<Badge variant="secondary" className="text-[10px]">
										{aiAct.riskCategory ?? "Assessed"}
										{aiAct.conformityScore !== null &&
											` · ${aiAct.conformityScore}/100`}
									</Badge>
								) : (
									<Badge variant="outline" className="text-[10px]">
										{t("notSubmitted", "Not submitted")}
									</Badge>
								)}
							</span>
						) : (
							<span className="text-muted-foreground">
								{t(
									"bringTheProjectOnlineToStartThePublicationProcess",
									"Bring the project online to start the publication process.",
								)}
							</span>
						)}
						<Button
							variant="outline"
							size="sm"
							className="ml-auto"
							onClick={() => onOpenPanel("compliance")}
						>
							{t("openPublication", "Open publication")}
						</Button>
					</Card>
				</StageRow>
			</div>

			{/* right rail */}
			<div className="space-y-4">
				<SectionCard
					title={t("needsYou", "Needs you")}
					count={signals.length}
					contentClassName="p-2"
				>
					{signals.length === 0 ? (
						<p className="px-2 py-3 text-xs text-muted-foreground">
							{t("nothingIsBlockedRightNow", "Nothing is blocked right now.")}
						</p>
					) : (
						<div className="space-y-0.5">
							{signals.map((signal) => (
								<RailSignalRow
									key={signal.id}
									signal={signal}
									onOpenPanel={onOpenPanel}
								/>
							))}
						</div>
					)}
				</SectionCard>

				<SectionCard title={t("atAGlance", "At a glance")}>
					<div className="space-y-2 text-xs">
						<div className="flex items-center gap-2">
							<span className="text-muted-foreground">
								{t("runs24h2", "Runs · 24h")}
							</span>
							<span className="ml-auto font-medium tabular-nums">
								{runs.windowRuns.toLocaleString()}
							</span>
						</div>
						<div className="flex items-center gap-2">
							<span className="text-muted-foreground">
								{t("success", "Success")}
							</span>
							<span className="ml-auto font-medium tabular-nums">
								{runs.successRate === null
									? "—"
									: `${runs.successRate.toFixed(1)}%`}
							</span>
						</div>
						<div className="flex items-center gap-2">
							<span className="text-muted-foreground">
								{t("lastRun", "Last run")}
							</span>
							<span className="ml-auto font-medium">
								{runs.lastRunAt
									? formatRelativeTime(
											new Date(runs.lastRunAt).toISOString(),
											"narrow",
										)
									: "never"}
							</span>
						</div>
						<div className="flex items-center gap-2">
							<span className="text-muted-foreground">
								{t("status", "Status")}
							</span>
							<span className="ml-auto">
								<Badge variant="secondary" className="text-[10px]">
									{app.status}
								</Badge>
							</span>
						</div>
					</div>
				</SectionCard>

				{app.changelog && (
					<SectionCard title={t("latestRelease", "Latest release")}>
						<div className="space-y-1 text-xs">
							<Badge variant="outline" className="text-[10px]">
								v{app.version ?? "unversioned"}
							</Badge>
							<p className="whitespace-pre-line text-muted-foreground">
								{app.changelog}
							</p>
						</div>
					</SectionCard>
				)}

				<Button
					variant="outline"
					size="sm"
					className="w-full text-destructive"
					onClick={() => onOpenPanel("advanced")}
				>
					{t("advancedDangerZone", "Advanced & danger zone")}
				</Button>
			</div>
		</div>
	);
}
