"use client";

import { useTranslation } from "@flow-like/locales";
import { MaximizeIcon, MinusIcon, NetworkIcon, PlusIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useOrg } from "../../../hooks/use-bot-teams";
import { orgForest } from "../../../lib/bot-teams/org-model";
import type { IBot } from "../../../state/backend-state/bot-state";
import { Button } from "../../ui/button";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";
import { AppFilterPicker } from "./app-card";
import type { AppFilter } from "./org-chart-apps";
import {
	type AppDestination,
	OrgChartProvider,
	useOrgChart,
} from "./org-chart-context";
import { OrgChartDialogs } from "./org-chart-dialogs";
import { DockedInspector, SheetInspector } from "./org-chart-inspector";
import { OrgOutline } from "./org-chart-outline";
import { AskBadge } from "./org-chart-parts";
import { Bench, OrgTree } from "./org-chart-tree";
import { ZOOM_MAX, ZOOM_MIN } from "./org-chart-viewport";
import {
	type ChartZoom,
	useChartViewport,
	useChartZoom,
} from "./use-chart-viewport";

export interface OrgChartViewProps {
	me: string | null;
	bots: IBot[] | undefined;
	onOpenChat: (botId: string) => void;
	onOpenConversation: (conversationId: string) => void;
	onOpenApp: (appId: string, where: AppDestination) => void;
}

interface AppToggles {
	showApps: boolean;
	onShowApps: (value: boolean) => void;
	appFilter: AppFilter;
	onAppFilter: (value: AppFilter) => void;
}

const NARROW_PX = 760;
const LEGEND = ["direct", "ok", "via", "blocked", "later"] as const;

function useWidth() {
	const ref = useRef<HTMLDivElement>(null);
	const [width, setWidth] = useState(1024);
	useEffect(() => {
		const element = ref.current;
		if (!element) return;
		setWidth(element.clientWidth || 1024);
		if (typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(([entry]) =>
			setWidth(entry.contentRect.width),
		);
		observer.observe(element);
		return () => observer.disconnect();
	}, []);
	return { ref, width };
}

interface ZoomControlsProps {
	zoom: ChartZoom;
	zoomIn: () => void;
	zoomOut: () => void;
	fit: () => void;
}

function ZoomControls({
	zoom: store,
	zoomIn,
	zoomOut,
	fit,
}: Readonly<ZoomControlsProps>) {
	const { t } = useTranslation("bots");
	const zoom = useChartZoom(store);
	return (
		<div className="flex items-center gap-1">
			<Button
				variant="ghost"
				size="icon"
				className="size-7"
				aria-label={t("teams.org.zoomOut", "Zoom out")}
				disabled={zoom <= ZOOM_MIN}
				onClick={zoomOut}
			>
				<MinusIcon aria-hidden />
			</Button>
			<span
				className="w-10 text-center text-xs tabular-nums"
				aria-live="polite"
			>
				{Math.round(zoom * 100)}%
			</span>
			<Button
				variant="ghost"
				size="icon"
				className="size-7"
				aria-label={t("teams.org.zoomIn", "Zoom in")}
				disabled={zoom >= ZOOM_MAX}
				onClick={zoomIn}
			>
				<PlusIcon aria-hidden />
			</Button>
			<Button
				variant="ghost"
				size="icon"
				className="size-7"
				aria-label={t("teams.org.fit", "Fit to view")}
				onClick={fit}
			>
				<MaximizeIcon aria-hidden />
			</Button>
		</div>
	);
}

function Toolbar({
	showAsk,
	onShowAsk,
	showIo,
	onShowIo,
	apps,
	controls,
}: Readonly<{
	showAsk: boolean;
	onShowAsk: (value: boolean) => void;
	showIo: boolean;
	onShowIo: (value: boolean) => void;
	apps: AppToggles;
	controls: ZoomControlsProps | null;
}>) {
	const { t } = useTranslation("bots");
	const { setDialog } = useOrgChart();
	return (
		<div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b px-4 py-2">
			<h2 className="text-sm font-semibold">
				{t("teams.org.title", "Org chart")}
			</h2>
			<div className="flex items-center gap-2">
				<Switch id="org-ins-outs" checked={showIo} onCheckedChange={onShowIo} />
				<Label htmlFor="org-ins-outs" className="text-xs">
					{t("teams.org.io.toggle", "Ins & outs")}
				</Label>
			</div>
			<div className="flex items-center gap-2">
				<Switch
					id="org-apps"
					checked={apps.showApps}
					onCheckedChange={apps.onShowApps}
				/>
				<Label htmlFor="org-apps" className="text-xs">
					{t("teams.org.apps.toggle", "Apps")}
				</Label>
			</div>
			{apps.showApps && (
				<AppFilterPicker
					value={apps.appFilter}
					onChange={apps.onAppFilter}
					look="segmented"
				/>
			)}
			<div className="flex items-center gap-2">
				<Switch
					id="org-who-can-ask"
					checked={showAsk}
					onCheckedChange={onShowAsk}
				/>
				<Label htmlFor="org-who-can-ask" className="text-xs">
					{t("teams.org.whoCanAskToggle", "Who can ask")}
				</Label>
			</div>
			{showAsk && (
				<ul
					aria-label={t("teams.org.legend", "Legend")}
					className="flex flex-wrap items-center gap-1.5"
				>
					{LEGEND.map((status) => (
						<li key={status}>
							<AskBadge status={status} />
						</li>
					))}
				</ul>
			)}
			<div className="ml-auto flex items-center gap-2">
				{controls && <ZoomControls {...controls} />}
				<Button
					size="sm"
					onClick={() => setDialog({ kind: "newTeam", parentId: null })}
				>
					<PlusIcon aria-hidden />
					{t("teams.org.newTeam", "New team")}
				</Button>
			</div>
		</div>
	);
}

function Chart({
	viewportRef,
	contentRef,
}: Readonly<{
	viewportRef: (element: HTMLDivElement | null) => void;
	contentRef: (element: HTMLDivElement | null) => void;
}>) {
	const { org, select } = useOrgChart();
	const forest = orgForest(org);
	return (
		<div
			ref={viewportRef}
			className="relative min-h-0 flex-1 cursor-grab touch-none overflow-clip bg-[radial-gradient(var(--border)_1px,transparent_1px)] bg-size-[18px_18px] select-none data-[panning=true]:cursor-grabbing data-[panning=true]:[&_*]:cursor-grabbing [&_section]:cursor-default"
			onClick={(event) => {
				if (!(event.target as Element).closest("section, button")) select(null);
			}}
			onKeyDown={undefined}
			role="presentation"
		>
			<div
				ref={contentRef}
				className="absolute top-0 left-0 flex w-max origin-top-left flex-col gap-10 p-8"
			>
				<div className="flex items-start justify-center gap-12">
					{forest.map((node) => (
						<OrgTree key={node.team.id} node={node} />
					))}
				</div>
				<Bench />
			</div>
		</div>
	);
}

function EmptyOrg({ available }: Readonly<{ available: boolean }>) {
	const { t } = useTranslation("bots");
	const { setDialog } = useOrgChart();
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
			<NetworkIcon className="size-8 text-muted-foreground" aria-hidden />
			<h3 className="text-base font-semibold">
				{t("teams.org.emptyTitle", "No teams yet")}
			</h3>
			<p className="max-w-md text-sm text-muted-foreground">
				{available
					? t(
							"teams.org.emptyBody",
							"Teams group bots and people, like Security or Finance, so bots know who they work with. Each team has a lead bot and a room, and teams can sit inside other teams.",
						)
					: t(
							"teams.org.needsAccount",
							"Teams need a Flow-Like account. Sign in to organize your bots into teams.",
						)}
			</p>
			{available && (
				<Button onClick={() => setDialog({ kind: "newTeam", parentId: null })}>
					<PlusIcon aria-hidden />
					{t("teams.org.createFirst", "Create your first team")}
				</Button>
			)}
		</div>
	);
}

function OrgChartBody({
	available,
	hasTeams,
	showAsk,
	onShowAsk,
	showIo,
	onShowIo,
	apps,
}: Readonly<{
	available: boolean;
	hasTeams: boolean;
	showAsk: boolean;
	onShowAsk: (value: boolean) => void;
	showIo: boolean;
	onShowIo: (value: boolean) => void;
	apps: AppToggles;
}>) {
	const { ref, width } = useWidth();
	const { dragBotId } = useOrgChart();
	const { viewportRef, contentRef, ...controls } = useChartViewport(
		dragBotId !== null,
	);
	const narrow = width < NARROW_PX;
	return (
		<div ref={ref} className="flex h-full min-h-0 flex-col">
			{hasTeams ? (
				<>
					<Toolbar
						showAsk={showAsk}
						onShowAsk={onShowAsk}
						showIo={showIo}
						onShowIo={onShowIo}
						apps={apps}
						controls={narrow ? null : controls}
					/>
					<div className="flex min-h-0 flex-1">
						{narrow ? (
							<div className="min-h-0 flex-1 overflow-y-auto">
								<OrgOutline />
							</div>
						) : (
							<>
								<Chart viewportRef={viewportRef} contentRef={contentRef} />
								<DockedInspector />
							</>
						)}
					</div>
					{narrow && <SheetInspector />}
				</>
			) : (
				<EmptyOrg available={available} />
			)}
			<OrgChartDialogs />
		</div>
	);
}

export function OrgChartView({
	me,
	bots,
	onOpenChat,
	onOpenConversation,
	onOpenApp,
}: Readonly<OrgChartViewProps>) {
	const { org, available } = useOrg();
	const [showAsk, setShowAsk] = useState(false);
	const [showIo, setShowIo] = useState(true);
	const [showApps, setShowApps] = useState(true);
	const [appFilter, setAppFilter] = useState<AppFilter>("all");
	return (
		<OrgChartProvider
			org={org}
			me={me}
			bots={bots ?? []}
			showAsk={showAsk}
			showIo={showIo}
			showApps={showApps}
			appFilter={appFilter}
			onOpenChat={onOpenChat}
			onOpenConversation={onOpenConversation}
			onOpenApp={onOpenApp}
		>
			<OrgChartBody
				available={available}
				hasTeams={org.teams.length > 0}
				showAsk={showAsk}
				onShowAsk={setShowAsk}
				showIo={showIo}
				onShowIo={setShowIo}
				apps={{
					showApps,
					onShowApps: setShowApps,
					appFilter,
					onAppFilter: setAppFilter,
				}}
			/>
		</OrgChartProvider>
	);
}
