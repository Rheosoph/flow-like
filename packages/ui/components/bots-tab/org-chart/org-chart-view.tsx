"use client";

import { useTranslation } from "@flow-like/locales";
import { MaximizeIcon, MinusIcon, NetworkIcon, PlusIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useOrg } from "../../../hooks/use-bot-teams";
import { orgForest } from "../../../lib/bot-teams/org-model";
import type { IBot } from "../../../state/backend-state/bot-state";
import { Button } from "../../ui/button";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";
import { OrgChartProvider, useOrgChart } from "./org-chart-context";
import { OrgChartDialogs } from "./org-chart-dialogs";
import { DockedInspector, SheetInspector } from "./org-chart-inspector";
import { OrgOutline } from "./org-chart-outline";
import { AskBadge } from "./org-chart-parts";
import { Bench, OrgTree } from "./org-chart-tree";

export interface OrgChartViewProps {
	me: string | null;
	bots: IBot[] | undefined;
	onOpenChat: (botId: string) => void;
	onOpenConversation: (conversationId: string) => void;
}

const NARROW_PX = 760;
const ZOOM_MIN = 0.4;
const ZOOM_MAX = 1.2;
const ZOOM_STEP = 0.1;
const LEGEND = ["direct", "ok", "via", "blocked", "later"] as const;

const clamp = (value: number) =>
	Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(value * 100) / 100));

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

function Toolbar({
	showAsk,
	onShowAsk,
	zoom,
	onZoom,
	onFit,
	charted,
}: Readonly<{
	showAsk: boolean;
	onShowAsk: (value: boolean) => void;
	zoom: number;
	onZoom: (value: number) => void;
	onFit: () => void;
	charted: boolean;
}>) {
	const { t } = useTranslation("bots");
	const { setDialog } = useOrgChart();
	return (
		<div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b px-4 py-2">
			<h2 className="text-sm font-semibold">
				{t("teams.org.title", "Org chart")}
			</h2>
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
				{charted && (
					<div className="flex items-center gap-1">
						<Button
							variant="ghost"
							size="icon"
							className="size-7"
							aria-label={t("teams.org.zoomOut", "Zoom out")}
							disabled={zoom <= ZOOM_MIN}
							onClick={() => onZoom(zoom - ZOOM_STEP)}
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
							onClick={() => onZoom(zoom + ZOOM_STEP)}
						>
							<PlusIcon aria-hidden />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="size-7"
							aria-label={t("teams.org.fit", "Fit to view")}
							onClick={onFit}
						>
							<MaximizeIcon aria-hidden />
						</Button>
					</div>
				)}
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
	zoom,
	contentRef,
}: Readonly<{
	zoom: number;
	contentRef: React.RefObject<HTMLDivElement | null>;
}>) {
	const { org, select } = useOrgChart();
	const forest = orgForest(org);
	return (
		<div
			className="min-h-0 flex-1 overflow-auto bg-[radial-gradient(var(--border)_1px,transparent_1px)] [background-size:18px_18px]"
			onClick={(event) => event.target === event.currentTarget && select(null)}
			onKeyDown={undefined}
			role="presentation"
		>
			<div
				ref={contentRef}
				style={{ zoom }}
				className="flex w-max min-w-full flex-col gap-10 p-8"
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
}: Readonly<{
	available: boolean;
	hasTeams: boolean;
	showAsk: boolean;
	onShowAsk: (value: boolean) => void;
}>) {
	const { ref, width } = useWidth();
	const contentRef = useRef<HTMLDivElement>(null);
	const [zoom, setZoom] = useState(1);
	const narrow = width < NARROW_PX;
	const fit = useCallback(() => {
		const content = contentRef.current;
		if (!content) return;
		const natural = content.scrollWidth / zoom;
		setZoom(clamp((width - 380) / natural));
	}, [width, zoom]);
	return (
		<div ref={ref} className="flex h-full min-h-0 flex-col">
			{hasTeams ? (
				<>
					<Toolbar
						showAsk={showAsk}
						onShowAsk={onShowAsk}
						zoom={zoom}
						onZoom={(value) => setZoom(clamp(value))}
						onFit={fit}
						charted={!narrow}
					/>
					<div className="flex min-h-0 flex-1">
						{narrow ? (
							<div className="min-h-0 flex-1 overflow-y-auto">
								<OrgOutline />
							</div>
						) : (
							<>
								<Chart zoom={zoom} contentRef={contentRef} />
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
}: Readonly<OrgChartViewProps>) {
	const { org, available } = useOrg();
	const [showAsk, setShowAsk] = useState(false);
	return (
		<OrgChartProvider
			org={org}
			me={me}
			bots={bots ?? []}
			showAsk={showAsk}
			onOpenChat={onOpenChat}
			onOpenConversation={onOpenConversation}
		>
			<OrgChartBody
				available={available}
				hasTeams={org.teams.length > 0}
				showAsk={showAsk}
				onShowAsk={setShowAsk}
			/>
		</OrgChartProvider>
	);
}
