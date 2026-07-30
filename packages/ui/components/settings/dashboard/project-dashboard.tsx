"use client";

import { GaugeIcon, RouteIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useMemo, useState } from "react";
import { useInvalidateInvoke, useInvoke } from "../../../hooks";
import { detectAppType } from "../../../lib/app-type";
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
	useListingChecklist,
	useProjectSignals,
} from "./use-project-signals";

export interface ProjectDashboardProps {
	appId: string;
	canEdit?: boolean;
	/** Fired after the app is deleted so the host can navigate away. */
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
						Launch
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					Step-by-step view — what to do next
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
						Operate
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					Operations view — health, surfaces and activity
				</TooltipContent>
			</Tooltip>
		</div>
	);
}

function DashboardSkeleton() {
	return (
		<div className="space-y-4">
			<Skeleton className="h-16 w-full rounded-xl" />
			<div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
				{["a", "b", "c", "d"].map((key) => (
					<Skeleton key={key} className="h-24 rounded-xl" />
				))}
			</div>
			<Skeleton className="h-64 w-full rounded-xl" />
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
	const enabled = appId.length > 0;

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
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[appId],
		enabled,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		enabled,
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		enabled,
	);
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		enabled,
	);

	const [inspectorOpen, setInspectorOpen] = useState(false);
	const [panel, setPanel] = useState<InspectorPanel>("identity");

	const runs = useProjectRuns(appId, boards.data);
	const aiAct = useAiActStatus(appId, app.data?.visibility);
	const surfaces = useProjectSurfaces(
		events.data,
		pages.data,
		routes.data,
		runs.byEvent,
	);
	const listing = useListingChecklist(app.data, metadata.data);
	const suggestedType = useMemo(
		() => detectAppType(boards.data, events.data, pages.data?.length ?? 0),
		[boards.data, events.data, pages.data],
	);
	const boardNames = useMemo(() => {
		const map = new Map<string, string>();
		for (const board of boards.data ?? []) map.set(board.id, board.name);
		return map;
	}, [boards.data]);

	const signals = useProjectSignals({
		appId,
		app: app.data,
		events: events.data,
		runs,
		aiAct,
		listingDone: listing.done,
		listingTotal: listing.total,
		boardNames,
	});

	const { mode, setPreference } = useDashboardMode(
		appId,
		runs.hasEverSucceeded,
		runs.ready,
	);

	const refreshApp = useCallback(async () => {
		await app.refetch();
		await metadata.refetch();
		await invalidate(backend.appState.getApps, []);
	}, [app, metadata, invalidate, backend.appState]);

	const draft = useProjectDraft(appId, app.data, metadata.data, refreshApp);

	const openPanel = useCallback((next: InspectorPanel) => {
		setPanel(next);
		setInspectorOpen(true);
	}, []);

	const handleDelete = useCallback(async () => {
		await backend.appState.deleteApp(appId);
		await invalidate(backend.appState.getApps, []);
		await onDeleted();
	}, [appId, backend.appState, invalidate, onDeleted]);

	if (!app.data || !metadata.data) {
		return (
			<div className="mx-auto w-full max-w-6xl px-1 py-4">
				<DashboardSkeleton />
			</div>
		);
	}

	return (
		<TooltipProvider>
			<div className="mx-auto flex w-full max-w-6xl flex-col gap-4 px-1 pb-6">
				<ProjectIdentityRow
					app={app.data}
					metadata={metadata.data}
					canEdit={canEdit}
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
						onOpenPanel={openPanel}
					/>
				)}

				<SettingsInspector
					appId={appId}
					app={app.data}
					metadata={metadata.data}
					canEdit={canEdit}
					draft={draft}
					open={inspectorOpen}
					panel={panel}
					onOpenChange={setInspectorOpen}
					onPanelChange={setPanel}
					onDeleted={handleDelete}
					onMediaChanged={refreshApp}
					suggestedType={suggestedType}
					slots={slots}
				/>
			</div>
		</TooltipProvider>
	);
}
