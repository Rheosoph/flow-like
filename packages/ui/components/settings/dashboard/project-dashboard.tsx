"use client";

import { useTranslation } from "@flow-like/locales";
import { GaugeIcon, RouteIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useMemo, useState } from "react";
import { useInvalidateInvoke, useInvoke } from "../../../hooks";
import { removeAppFromProfile } from "../../../lib/add-app-to-profile";
import { detectAppType } from "../../../lib/app-type";
import { asArray, isRecord } from "../../../lib/response-shape";
import { boardListing } from "../../../lib/schema/flow/board-summary";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui/tooltip";
import { AttentionQueue } from "./attention-queue";
import { LaunchPath } from "./launch-path";
import { MissionControl } from "./mission-control";
import { ProjectIdentityRow } from "./project-identity-row";
import { type InspectorSlots, SettingsInspector } from "./settings-inspector";
import { useProjectSurfaces } from "./surfaces-table";
import { type DashboardMode, useDashboardMode } from "./use-dashboard-mode";
import { useProjectDraft } from "./use-project-draft";
import { useProjectRuns } from "./use-project-runs";
import {
	type InspectorPanel,
	useAiActStatus,
	useDashboardPermissions,
	useListingChecklist,
	useProjectSignals,
} from "./use-project-signals";

export interface ProjectDashboardProps {
	appId: string;
	/**
	 * Deployment-level veto on editing. It can only take rights away — what the
	 * account may actually do is resolved from its role, so a host that leaves
	 * this at `true` still cannot hand a member controls the server refuses.
	 */
	canEdit?: boolean;
	/**
	 * Fired once the app has left the user's library — deleted outright, or
	 * quit by a member who does not own it — so the host can navigate away.
	 */
	onDeleted: () => void | Promise<void>;
	/**
	 * Host-provided sections. Desktop and web differ only in how forking and
	 * publication are wired, so those arrive as slots rather than being
	 * reimplemented per deployment.
	 */
	slots?: InspectorSlots;
	/** Extra actions for the identity row, e.g. a host-specific fork button. */
	identityActions?: ReactNode;
}

function ModeToggle({
	mode,
	onSelect,
}: Readonly<{ mode: DashboardMode; onSelect: (mode: DashboardMode) => void }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex items-center gap-1 rounded-full border bg-muted/50 p-0.5">
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant={mode === "launch" ? "default" : "ghost"}
						size="sm"
						className="h-7 rounded-full px-2.5 text-xs"
						onClick={() => onSelect("launch")}
					>
						<RouteIcon className="mr-1 h-3 w-3" />
						{t("launch", "Launch")}
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					{t(
						"stepbystepViewWhatToDoNext",
						"Step-by-step view — what to do next",
					)}
				</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant={mode === "control" ? "default" : "ghost"}
						size="sm"
						className="h-7 rounded-full px-2.5 text-xs"
						onClick={() => onSelect("control")}
					>
						<GaugeIcon className="mr-1 h-3 w-3" />
						{t("operate", "Operate")}
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					{t(
						"operationsViewHealthSurfacesAndActivity",
						"Operations view — health, surfaces and activity",
					)}
				</TooltipContent>
			</Tooltip>
		</div>
	);
}

function DashboardSkeleton() {
	return (
		<div className="space-y-4">
			<Skeleton className="h-16 w-full rounded-xl" />
			<div className="grid grid-cols-1 gap-3 min-[400px]:grid-cols-2 lg:grid-cols-4">
				{["a", "b", "c", "d"].map((key) => (
					<Skeleton key={key} className="h-24 rounded-xl" />
				))}
			</div>
			<Skeleton className="h-64 w-full rounded-xl" />
		</div>
	);
}

function DashboardLoadError({ onRetry }: Readonly<{ onRetry: () => void }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex flex-col items-center gap-3 rounded-xl border p-8 text-center">
			<p className="text-sm text-muted-foreground">
				{t("common:failedToLoadApp", "Failed to load app")}
			</p>
			<Button variant="outline" size="sm" onClick={onRetry}>
				{t("retry", "Retry")}
			</Button>
		</div>
	);
}

/**
 * One dashboard with two arrangements.
 *
 * A project that has never run successfully is shown the Launch Path, because
 * its open question is "what do I do next". Once it has run, the same page
 * becomes Mission Control, whose question is "is it healthy". The identity row,
 * attention queue, surfaces and settings inspector are shared by both, so this
 * is a single build rather than two dashboards.
 */
export function ProjectDashboard({
	appId,
	canEdit = true,
	onDeleted,
	slots,
	identityActions,
}: Readonly<ProjectDashboardProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const permissions = useDashboardPermissions(appId, canEdit);
	const enabled = appId.length > 0;
	// Nothing app-scoped is asked for until the role is known: firing a read the
	// role forbids answers 403, and every consumer here would render that as a
	// zero. `useDashboardPermissions` degrades open once resolution finishes, so
	// a local project with no permission model still reads everything.
	const enabledWithRole = enabled && !permissions.isLoading;

	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId],
		enabled,
	);
	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		enabled,
	);
	const boardSummaries = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		enabledWithRole && permissions.canReadBoards,
	);
	// The dashboard only ever needs names and node counts, so it reads summaries — served from
	// the database — instead of every board's full graph.
	const boards = useMemo(
		() => ({
			data: asArray(boardSummaries.data).map(boardListing),
			isLoading: boardSummaries.isLoading,
		}),
		[boardSummaries.data, boardSummaries.isLoading],
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		enabledWithRole && permissions.canListEvents,
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		enabledWithRole && permissions.canReadBoards,
	);
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		enabledWithRole && permissions.canListEvents,
	);
	const eventList = useMemo(() => asArray(events.data), [events.data]);
	const pageList = useMemo(() => asArray(pages.data), [pages.data]);
	const routeList = useMemo(() => asArray(routes.data), [routes.data]);

	const [inspectorOpen, setInspectorOpen] = useState(false);
	const [panel, setPanel] = useState<InspectorPanel>("identity");

	const runs = useProjectRuns(
		appId,
		boards.data,
		enabledWithRole && permissions.canReadBoards,
	);
	const aiAct = useAiActStatus(
		appId,
		app.data?.visibility,
		enabledWithRole && permissions.canReadCompliance,
	);
	const surfaces = useProjectSurfaces(
		eventList,
		pageList,
		routeList,
		runs.byEvent,
	);
	const listing = useListingChecklist(app.data, metadata.data);
	const suggestedType = useMemo(
		() => detectAppType(boards.data, eventList, pageList.length),
		[boards.data, eventList, pageList],
	);
	const boardNames = useMemo(() => {
		const map = new Map<string, string>();
		for (const board of boards.data ?? []) map.set(board.id, board.name);
		return map;
	}, [boards.data]);

	const signals = useProjectSignals({
		appId,
		app: app.data,
		events: eventList,
		runs,
		aiAct,
		listingDone: listing.done,
		listingTotal: listing.total,
		boardNames,
		permissions,
	});

	// A role that cannot read the run log can never satisfy "has succeeded", so
	// the auto rule would park every such account on the Launch Path — the
	// onboarding view — for a project that has been live for months. When the
	// history is unreadable, Operate is the honest default.
	const { mode, setPreference } = useDashboardMode(
		appId,
		runs.denied || runs.hasEverSucceeded,
		runs.denied || runs.ready,
	);

	const refreshApp = useCallback(async () => {
		await app.refetch();
		await metadata.refetch();
		await invalidate(backend.appState.getApps, []);
	}, [app, metadata, invalidate, backend.appState]);

	const draft = useProjectDraft(
		appId,
		app.data,
		metadata.data,
		refreshApp,
		permissions,
	);

	const openPanel = useCallback((next: InspectorPanel) => {
		setPanel(next);
		setInspectorOpen(true);
	}, []);

	const handleDelete = useCallback(async () => {
		await backend.appState.deleteApp(appId);
		await invalidate(backend.appState.getApps, []);
		await onDeleted();
	}, [appId, backend.appState, invalidate, onDeleted]);

	const handleLeave = useCallback(async () => {
		await backend.appState.leaveApp(appId);
		// The profile is what the library actually lists, and every per-app query
		// now answers 403 — clear both before the host navigates away, or the user
		// lands back on a listing that still holds the project.
		await removeAppFromProfile(backend, appId);
		await invalidate(backend.appState.getApps, []);
		await invalidate(backend.userState.getSettingsProfile, []);
		await invalidate(backend.appState.getApp, [appId]);
		await invalidate(backend.appState.getAppMeta, [appId]);
		await onDeleted();
	}, [appId, backend, invalidate, onDeleted]);

	if (
		!isRecord(app.data) ||
		!isRecord(metadata.data) ||
		permissions.isLoading
	) {
		// With no local copy and no hub, the reads settle as errors; do not spin forever.
		const loadFailed =
			(app.isError && !isRecord(app.data)) ||
			(metadata.isError && !isRecord(metadata.data));
		return (
			<div className="mx-auto w-full max-w-6xl px-1 py-4">
				{loadFailed ? (
					<DashboardLoadError
						onRetry={() => {
							void app.refetch();
							void metadata.refetch();
						}}
					/>
				) : (
					<DashboardSkeleton />
				)}
			</div>
		);
	}

	return (
		<TooltipProvider>
			<div className="mx-auto flex min-w-0 w-full max-w-6xl flex-col gap-4 px-1 pb-6">
				<ProjectIdentityRow
					app={app.data}
					metadata={metadata.data}
					permissions={permissions}
					onOpenPanel={openPanel}
					actions={
						<>
							{identityActions}
							<ModeToggle mode={mode} onSelect={setPreference} />
						</>
					}
				/>

				<AttentionQueue signals={signals} onOpenPanel={openPanel} />

				{mode === "control" ? (
					<MissionControl
						appId={appId}
						app={app.data}
						boards={boards.data ?? []}
						surfaces={surfaces}
						runs={runs}
						aiAct={aiAct}
						listing={listing.items}
						listingDone={listing.done}
						permissions={permissions}
						onOpenPanel={openPanel}
					/>
				) : (
					<LaunchPath
						appId={appId}
						app={app.data}
						boards={boards.data ?? []}
						surfaces={surfaces}
						runs={runs}
						aiAct={aiAct}
						listing={listing.items}
						listingDone={listing.done}
						signals={signals}
						permissions={permissions}
						onOpenPanel={openPanel}
					/>
				)}

				<SettingsInspector
					appId={appId}
					app={app.data}
					metadata={metadata.data}
					permissions={permissions}
					draft={draft}
					open={inspectorOpen}
					panel={panel}
					onOpenChange={setInspectorOpen}
					onPanelChange={setPanel}
					onDeleted={handleDelete}
					onLeft={handleLeave}
					onMediaChanged={refreshApp}
					suggestedType={suggestedType}
					slots={slots}
				/>
			</div>
		</TooltipProvider>
	);
}
