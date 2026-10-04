import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IPageState } from "../../state/backend-state/page-state";
import {
	reloadMicroWidgetsInPage,
	unavailableMicroWidgetsError,
} from "./micro-widget-project-reload";
import type { MicroWidgetReloadReport } from "./micro-widget-reload";

export interface ProjectMicroWidgetReloadResult {
	count: number;
	pages: number;
	boardIds: string[];
	report: MicroWidgetReloadReport;
	failures: { pageId: string; name: string; error: unknown }[];
}

/** Updates saved drafts across the app; the active builder owns its working copy. */
export async function reloadProjectMicroWidgets(
	pageState: Pick<
		IPageState,
		"getPagesAuthoritative" | "getPageAuthoritative" | "updatePage"
	>,
	appId: string,
	installed: readonly AppPackageWidget[],
	{
		skipPageId,
		shouldContinue,
	}: { skipPageId?: string; shouldContinue?: () => boolean } = {},
): Promise<ProjectMicroWidgetReloadResult> {
	const pages = await pageState.getPagesAuthoritative(appId);
	const result: ProjectMicroWidgetReloadResult = {
		count: 0,
		pages: 0,
		boardIds: [],
		report: {
			converted: [],
			defaulted: [],
			reset: [],
			removed: [],
			undeclaredEvents: [],
		},
		failures: [],
	};
	const visited = new Set<string>();
	const remainingPages = pages.filter((row) => {
		if (row.pageId === skipPageId || visited.has(row.pageId)) return false;
		visited.add(row.pageId);
		return true;
	});
	const stopIfEditorChanged = (index: number) => {
		if (!shouldContinue || shouldContinue()) return false;
		const error = new Error(
			"The project view changed during the update. Run Update all again.",
		);
		result.failures.push(
			...remainingPages.slice(index).map((row) => ({
				pageId: row.pageId,
				name: row.name || row.pageId,
				error,
			})),
		);
		return true;
	};
	const boardIds = new Set<string>();
	for (const [index, row] of remainingPages.entries()) {
		if (stopIfEditorChanged(index)) break;
		try {
			const page = await pageState.getPageAuthoritative(
				appId,
				row.pageId,
				row.boardId,
			);
			if (stopIfEditorChanged(index)) break;
			const reloaded = reloadMicroWidgetsInPage(page, installed);
			if (reloaded.count > 0) {
				const updatedPage = {
					...reloaded.page,
					boardId: reloaded.page.boardId || row.boardId,
					updatedAt: new Date().toISOString(),
				};
				await pageState.updatePage(appId, updatedPage);
				result.count += reloaded.count;
				result.pages += 1;
				if (updatedPage.boardId) boardIds.add(updatedPage.boardId);
				for (const key of Object.keys(
					result.report,
				) as (keyof MicroWidgetReloadReport)[]) {
					result.report[key] = [
						...new Set([...result.report[key], ...reloaded.report[key]]),
					];
				}
			}
			if (reloaded.unavailable.length) {
				throw unavailableMicroWidgetsError(reloaded.unavailable);
			}
		} catch (error) {
			result.failures.push({
				pageId: row.pageId,
				name: row.name || row.pageId,
				error,
			});
		}
	}
	result.boardIds = [...boardIds];
	return result;
}
