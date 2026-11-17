"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRightIcon, GlobeIcon } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import type { IEvent } from "../../../lib";
import { appDenProps } from "../../../lib/bot-den/app-targets";
import { formatRelativeTime } from "../../../lib/date";
import { formatEventTypeLabel } from "../../../lib/event-type-label";
import { RolePermissions } from "../../../lib/permission/role-permission";
import type { PageListItem } from "../../../state/backend-state/page-state";
import type { IRouteMapping } from "../../../state/backend-state/route-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { PermissionNotice } from "../permission";
import { EmptyHint, SectionCard, StateDot } from "./dashboard-primitives";
import type { SurfaceRunHealth } from "./use-project-runs";
import type { DashboardPermissions } from "./use-project-signals";

export interface ProjectSurface {
	id: string;
	name: string;
	kind: string;
	den: { kind: "event" | "page"; id: string };
	entry: string | null;
	active: boolean;
	runs: number;
	failed: number;
	lastAt: number | null;
}

/**
 * Every way into the app in one list. Today's dashboard shows Events and Pages
 * as two unrelated cards and never mentions routes at all, even though they are
 * all answers to the same question: how does this app get triggered?
 */
export function useProjectSurfaces(
	events: IEvent[] | undefined,
	pages: PageListItem[] | undefined,
	routes: IRouteMapping[] | undefined,
	byEvent: Map<string, SurfaceRunHealth>,
): ProjectSurface[] {
	return useMemo(() => {
		const routeByEvent = new Map<string, string>();
		for (const route of routes ?? []) {
			if (!routeByEvent.has(route.eventId)) {
				routeByEvent.set(route.eventId, route.path);
			}
		}
		const pageById = new Map<string, PageListItem>();
		for (const page of pages ?? []) pageById.set(page.pageId, page);

		const linkedPages = new Set<string>();
		const surfaces: ProjectSurface[] = (events ?? []).map((event) => {
			const health = byEvent.get(event.id);
			const routePath = routeByEvent.get(event.id) ?? null;
			const page = event.default_page_id
				? pageById.get(event.default_page_id)
				: undefined;
			if (page) linkedPages.add(page.pageId);

			return {
				id: event.id,
				name: event.name,
				kind: formatEventTypeLabel(event.event_type),
				den: { kind: "event", id: event.id },
				entry: routePath ?? page?.name ?? null,
				active: event.active,
				runs: health?.total ?? 0,
				failed: health?.failed ?? 0,
				lastAt: health?.lastAt ?? null,
			};
		});

		for (const page of pages ?? []) {
			if (linkedPages.has(page.pageId)) continue;
			surfaces.push({
				id: `page-${page.pageId}`,
				name: page.name,
				kind: "Page",
				den: { kind: "page", id: page.pageId },
				entry: null,
				active: true,
				runs: 0,
				failed: 0,
				lastAt: null,
			});
		}

		return surfaces.sort((a, b) => {
			if (a.active !== b.active) return a.active ? -1 : 1;
			return b.runs - a.runs;
		});
	}, [events, pages, routes, byEvent]);
}

export function SurfacesTable({
	appId,
	surfaces,
	limit,
	permissions,
}: Readonly<{
	appId: string;
	surfaces: ProjectSurface[];
	limit?: number;
	permissions: DashboardPermissions;
}>) {
	const { t } = useTranslation("settings");
	const shown = limit ? surfaces.slice(0, limit) : surfaces;
	const manageHref = `/library/config/pages?id=${appId}`;
	// Triggers and routes are `ListEvents`; the pages they open are `ReadBoards`.
	// A denial on either leaves the list incomplete, so it has to be named —
	// "no triggers yet" and "you cannot see the triggers" lead opposite ways.
	const missingReads = [
		...(permissions.canListEvents ? [] : [RolePermissions.ListEvents]),
		...(permissions.canReadBoards ? [] : [RolePermissions.ReadBoards]),
	];
	const partial = missingReads.length > 0;

	return (
		<SectionCard
			title={t("surfaces", "Surfaces")}
			icon={GlobeIcon}
			count={partial ? undefined : surfaces.length}
			contentClassName="p-0"
			action={
				permissions.canWriteEvents ? (
					<Link href={manageHref}>
						<Button variant="ghost" size="sm" className="gap-1 text-xs">
							{t("manage", "Manage")}
							<ArrowRightIcon className="h-3 w-3" />
						</Button>
					</Link>
				) : (
					<Tooltip>
						<TooltipTrigger asChild>
							<span>
								<Button
									variant="ghost"
									size="sm"
									className="gap-1 text-xs"
									disabled
								>
									{t("manage", "Manage")}
									<ArrowRightIcon className="h-3 w-3" />
								</Button>
							</span>
						</TooltipTrigger>
						<TooltipContent side="bottom">
							{t(
								"yourRoleCannotChangeThisProjectsTriggers",
								"Your role cannot change this project's triggers.",
							)}
						</TooltipContent>
					</Tooltip>
				)
			}
		>
			{partial && (
				<div className="p-3">
					<PermissionNotice
						title={t("someSurfacesAreHidden", "Some surfaces are hidden")}
						description={t(
							"yourRoleCannotSeeEveryWayIntoThisAppSoThisListIsIncomplete",
							"Your role cannot see every way into this app, so this list is incomplete.",
						)}
						missing={missingReads}
						requireAll
					/>
				</div>
			)}
			{surfaces.length === 0 ? (
				!partial && (
					<EmptyHint>
						{t("noTriggersYet", "No triggers yet.")}{" "}
						{permissions.canWriteEvents ? (
							<Link href={manageHref} className="text-primary hover:underline">
								{t("setUpAnEvent", "Set up an event")}
							</Link>
						) : null}
					</EmptyHint>
				)
			) : (
				<div className="overflow-x-auto">
					<table className="w-full text-sm">
						<thead>
							<tr className="border-b text-[11px] uppercase tracking-wider text-muted-foreground">
								<th className="px-4 py-2 text-left font-medium">Name</th>
								<th className="px-2 py-2 text-left font-medium">
									{t("kind", "Kind")}
								</th>
								<th className="hidden px-2 py-2 text-left font-medium md:table-cell">
									{t("entry", "Entry")}
								</th>
								<th className="hidden px-2 py-2 text-left font-medium lg:table-cell">
									{t("lastFired", "Last fired")}
								</th>
								<th className="px-4 py-2 text-right font-medium">
									{t("runs24h", "Runs 24h")}
								</th>
							</tr>
						</thead>
						<tbody>
							{shown.map((surface) => (
								<tr
									key={surface.id}
									{...appDenProps({
										kind: surface.den.kind,
										appId,
										id: surface.den.id,
										label: surface.name,
									})}
									className="border-b last:border-0 hover:bg-muted/50"
								>
									<td className="px-4 py-2">
										<span className="flex items-center gap-2">
											<StateDot
												tone={
													!surface.active
														? "idle"
														: surface.failed > 0
															? "critical"
															: "ok"
												}
											/>
											<span className="truncate font-medium">
												{surface.name}
											</span>
											{!surface.active && (
												<Badge variant="outline" className="text-[10px]">
													{t("paused", "Paused")}
												</Badge>
											)}
										</span>
									</td>
									<td className="px-2 py-2">
										<Badge variant="secondary" className="text-[10px]">
											{surface.kind}
										</Badge>
									</td>
									<td className="hidden px-2 py-2 text-xs text-muted-foreground md:table-cell">
										{surface.entry ?? "—"}
									</td>
									<td className="hidden px-2 py-2 text-xs text-muted-foreground lg:table-cell">
										{surface.lastAt
											? formatRelativeTime(surface.lastAt, "narrow")
											: "—"}
									</td>
									<td className="px-4 py-2 text-right text-xs tabular-nums">
										{surface.failed > 0 ? (
											<span className="text-destructive">
												{t("failedFailed", "{{failed}} failed", {
													failed: surface.failed,
												})}
											</span>
										) : null}{" "}
										<span className="text-muted-foreground">
											{surface.runs}
										</span>
									</td>
								</tr>
							))}
						</tbody>
					</table>
					{limit && surfaces.length > limit && (
						<p className="py-2 text-center text-xs text-muted-foreground">
							+{surfaces.length - limit} more
						</p>
					)}
				</div>
			)}
		</SectionCard>
	);
}
