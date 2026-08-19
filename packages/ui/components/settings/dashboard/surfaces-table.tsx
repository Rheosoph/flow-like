"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRightIcon, GlobeIcon } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import type { IEvent } from "../../../lib";
import { formatRelativeTime } from "../../../lib/date";
import { formatEventTypeLabel } from "../../../lib/event-type-label";
import type { PageListItem } from "../../../state/backend-state/page-state";
import type { IRouteMapping } from "../../../state/backend-state/route-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { EmptyHint, SectionCard, StateDot } from "./dashboard-primitives";
import type { SurfaceRunHealth } from "./use-project-runs";

export interface ProjectSurface {
	id: string;
	name: string;
	kind: string;
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
}: Readonly<{ appId: string; surfaces: ProjectSurface[]; limit?: number }>) {
	const { t } = useTranslation("settings");
	const shown = limit ? surfaces.slice(0, limit) : surfaces;
	const manageHref = `/library/config/pages?id=${appId}`;

	return (
		<SectionCard
			title="Surfaces"
			icon={GlobeIcon}
			count={surfaces.length}
			contentClassName="p-0"
			action={
				<Link href={manageHref}>
					<Button variant="ghost" size="sm" className="gap-1 text-xs">
						Manage
						<ArrowRightIcon className="h-3 w-3" />
					</Button>
				</Link>
			}
		>
			{surfaces.length === 0 ? (
				<EmptyHint>
					{t('noTriggersYet', 'No triggers yet.')}{" "}
					<Link href={manageHref} className="text-primary hover:underline">
						{t('setUpAnEvent', 'Set up an event')}
					</Link>
				</EmptyHint>
			) : (
				<div className="overflow-x-auto">
					<table className="w-full text-sm">
						<thead>
							<tr className="border-b text-[11px] uppercase tracking-wider text-muted-foreground">
								<th className="px-4 py-2 text-left font-medium">Name</th>
								<th className="px-2 py-2 text-left font-medium">{t('kind', 'Kind')}</th>
								<th className="hidden px-2 py-2 text-left font-medium md:table-cell">
									{t('entry', 'Entry')}
								</th>
								<th className="hidden px-2 py-2 text-left font-medium lg:table-cell">
									{t('lastFired', 'Last fired')}
								</th>
								<th className="px-4 py-2 text-right font-medium">{t('runs24h', 'Runs 24h')}</th>
							</tr>
						</thead>
						<tbody>
							{shown.map((surface) => (
								<tr
									key={surface.id}
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
													{t('paused', 'Paused')}
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
											? formatRelativeTime(
													new Date(surface.lastAt).toISOString(),
													"narrow",
												)
											: "—"}
									</td>
									<td className="px-4 py-2 text-right text-xs tabular-nums">
										{surface.failed > 0 ? (
											<span className="text-destructive">{t('failedFailed', '{{failed}} failed', { failed: surface.failed })}</span>
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
