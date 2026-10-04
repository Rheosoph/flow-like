import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IPageState } from "../../state/backend-state/page-state";
import {
	isUnavailableMicroWidget,
	unavailableMicroWidgetsError,
} from "./micro-widget-project-reload";
import { findMicroWidgetUpdate } from "./micro-widget-reload";
import type { SurfaceComponent } from "./types";

export interface ProjectMicroWidgetScanResult {
	byPackage: Map<string, { widgets: number; pages: number }>;
	failures: { pageId: string; name: string; error: unknown }[];
}

/** Counts outdated widgets across saved drafts, including newer releases supplied by the caller. */
export async function scanProjectMicroWidgets(
	pageState: Pick<IPageState, "getPagesAuthoritative" | "getPageAuthoritative">,
	appId: string,
	installed: readonly AppPackageWidget[],
	latestVersions?: ReadonlyMap<string, string>,
): Promise<ProjectMicroWidgetScanResult> {
	const pages = await pageState.getPagesAuthoritative(appId);
	const result: ProjectMicroWidgetScanResult = {
		byPackage: new Map(),
		failures: [],
	};
	const visited = new Set<string>();
	for (const row of pages) {
		if (visited.has(row.pageId)) continue;
		visited.add(row.pageId);
		try {
			const page = await pageState.getPageAuthoritative(
				appId,
				row.pageId,
				row.boardId,
			);
			const counts = new Map<string, number>();
			const unavailable = new Set<string>();
			const countComponent = (surface: SurfaceComponent) => {
				const placed = surface.component;
				if (placed.type !== "microWidgetInstance") return;
				const current = installed.find(
					(entry) =>
						entry.packageId === placed.packageId &&
						entry.widget.id === placed.widgetId,
				);
				if (!current) {
					if (isUnavailableMicroWidget(installed, placed)) {
						unavailable.add(`${placed.packageId}/${placed.widgetId}`);
					}
					return;
				}
				const targetVersion = latestVersions?.get(placed.packageId);
				const outdated =
					targetVersion && targetVersion !== current.packageVersion
						? placed.packageVersion !== targetVersion
						: findMicroWidgetUpdate([current], placed) !== null;
				if (!outdated) return;
				counts.set(placed.packageId, (counts.get(placed.packageId) ?? 0) + 1);
			};
			for (const component of page.components ?? []) countComponent(component);
			for (const entry of page.content ?? []) {
				if ("Component" in entry) countComponent(entry.Component);
			}
			for (const widget of Object.values(page.widgetRefs ?? {})) {
				for (const component of widget.components ?? [])
					countComponent(component);
			}
			for (const [packageId, widgets] of counts) {
				const previous = result.byPackage.get(packageId);
				result.byPackage.set(packageId, {
					widgets: (previous?.widgets ?? 0) + widgets,
					pages: (previous?.pages ?? 0) + 1,
				});
			}
			if (unavailable.size) {
				throw unavailableMicroWidgetsError([...unavailable]);
			}
		} catch (error) {
			result.failures.push({
				pageId: row.pageId,
				name: row.name || row.pageId,
				error,
			});
		}
	}
	return result;
}
