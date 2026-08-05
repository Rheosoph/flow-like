import type { IPage, IPageState } from "../../state/backend-state/page-state";
import type { CanvasSettings, SurfaceComponent } from "../a2ui/types";
import {
	type FlowPilotWidgetPageTarget,
	isFlowPilotPageNotFoundError,
	slugifyRoute,
} from "./flowpilot-widget-target";

/** Route matching has to read each page, so a very large app must name its page outright. */
const MAX_SCANNED_PAGES = 50;

/** Mirrors WidgetBuilder's ROOT_ID — the page renderer looks up "root" verbatim. */
const ROOT_ID = "root";

export type DetachedPageLookup =
	| { ok: true; page: IPage }
	| { ok: false; code: string; message: string };

const normalizedName = (name: string | undefined) =>
	(name ?? "").trim().toLowerCase();

function matchesPageTarget(page: IPage, target: FlowPilotWidgetPageTarget) {
	if (target.route)
		return slugifyRoute(page.route ?? "") === slugifyRoute(target.route);
	return normalizedName(page.name) === normalizedName(target.pageName);
}

/**
 * The strongest identifier locates the page; every weaker one supplied alongside it is an assertion
 * that has to hold. A page whose board differs, or that has no board at all, is refused rather than
 * written — both backends reject a boardless page anyway.
 */
function verifyPageTarget(
	page: IPage,
	target: FlowPilotWidgetPageTarget,
): DetachedPageLookup {
	if (
		target.pageId &&
		target.route &&
		slugifyRoute(page.route ?? "") !== slugifyRoute(target.route)
	) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_TARGET_CONFLICT",
			message: `Page '${page.id}' has route '${page.route ?? "none"}', not the requested '${target.route}'. Nothing was changed — pass matching identifiers or only the page_id.`,
		};
	}
	if (
		target.pageId &&
		target.pageName &&
		normalizedName(page.name) !== normalizedName(target.pageName)
	) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_TARGET_CONFLICT",
			message: `Page '${page.id}' is named '${page.name}', not the requested '${target.pageName}'. Nothing was changed — pass matching identifiers or only the page_id.`,
		};
	}
	if (target.boardId && page.boardId && page.boardId !== target.boardId) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_BOARD_MISMATCH",
			message: `Page '${page.id}' belongs to board '${page.boardId}', not requested board '${target.boardId}'. Nothing was changed.`,
		};
	}
	if (!page.boardId) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_UNOWNED",
			message: `Page '${page.id}' has no board_id, so it cannot be saved. Repair its ownership before editing it.`,
		};
	}
	return { ok: true, page };
}

/**
 * Finds the persisted page a builder-less edit targets. An id resolves directly; a route or name has
 * to be matched against the app's pages because `PageListItem` carries neither. More than one match
 * is an error rather than a guess — silently rewriting the wrong page is the failure this path must
 * never produce.
 */
export async function findPersistedPage(
	pageState: IPageState,
	appId: string,
	target: FlowPilotWidgetPageTarget,
): Promise<DetachedPageLookup> {
	if (target.pageId) {
		try {
			// Deliberately no board argument: an exact-board lookup would turn an ownership
			// mismatch into a misleading "not found".
			return verifyPageTarget(
				await pageState.getPage(appId, target.pageId),
				target,
			);
		} catch (error) {
			// Only an authoritative miss means "no such page" — a transport or auth failure must
			// never be reported to the model as a missing page.
			if (!isFlowPilotPageNotFoundError(error)) throw error;
			return {
				ok: false,
				code: "FLOWPILOT_WIDGET_PAGE_NOT_FOUND",
				message: `Page '${target.pageId}' does not exist in app '${appId}'. List the app's pages with ui_inspect, or use mode='create' to add one.`,
			};
		}
	}

	const listed = (await pageState.getPages(appId, target.boardId)).filter(
		(item) =>
			!target.boardId || !item.boardId || item.boardId === target.boardId,
	);
	const described = target.route
		? `route '${target.route}'`
		: `page name '${target.pageName}'`;
	if (listed.length > MAX_SCANNED_PAGES) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_SCAN_TOO_LARGE",
			message: `App '${appId}' has ${listed.length} pages — too many to match by ${described}. Pass the exact page_id (ui_inspect lists them).`,
		};
	}

	const candidates: IPage[] = [];
	for (const item of listed) {
		try {
			const page = await pageState.getPage(
				appId,
				item.pageId,
				item.boardId ?? target.boardId,
			);
			if (matchesPageTarget(page, target)) candidates.push(page);
		} catch {
			// An unreadable page cannot be the target; keep scanning the rest.
		}
	}

	if (candidates.length === 0) {
		const known = listed
			.slice(0, 10)
			.map((item) => `${item.name} (${item.pageId})`)
			.join(", ");
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_NOT_FOUND",
			message: `No page in app '${appId}' matches ${described}.${known ? ` Its pages are: ${known}.` : ""} Pass an existing page_id, or use mode='create' to add one.`,
		};
	}
	if (candidates.length > 1) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_AMBIGUOUS",
			message: `${candidates.length} pages in app '${appId}' match ${described} (${candidates.map((page) => page.id).join(", ")}). Pass the exact page_id; nothing was changed.`,
		};
	}
	return verifyPageTarget(candidates[0], target);
}

export type DetachedWriteGuard =
	| { ok: true }
	| { ok: false; code: string; message: string };

/**
 * Runs inside the board lock, right before the write. `updatePage` replaces the whole page and no
 * layer in the stack does a compare-and-set, so a page that moved on since the snapshot must fail
 * loudly instead of being overwritten with a tree generated against the old one.
 */
export function assertDetachedWriteSafe(
	snapshot: Pick<IPage, "id" | "boardId" | "updatedAt">,
	fresh: Pick<IPage, "id" | "boardId" | "updatedAt">,
): DetachedWriteGuard {
	if (fresh.boardId !== snapshot.boardId) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_BOARD_CHANGED",
			message: `Page '${snapshot.id}' moved from board '${snapshot.boardId ?? "none"}' to '${fresh.boardId ?? "none"}' while the UI was being generated. Nothing was written.`,
		};
	}
	if (fresh.updatedAt !== snapshot.updatedAt) {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_CHANGED",
			message: `Page '${snapshot.id}' was saved by someone else while the UI was being generated. Nothing was written — run the same edit again and it will read the current page.`,
		};
	}
	return { ok: true };
}

function explicitChildList(component: SurfaceComponent | undefined): string[] {
	const children = (
		component?.component as unknown as Record<string, unknown> | undefined
	)?.children as Record<string, unknown> | undefined;
	if (!Array.isArray(children?.explicitList)) return [];
	return (children.explicitList as unknown[]).filter(
		(id): id is string => typeof id === "string",
	);
}

/** Mirrors WidgetBuilder's createRootComponent(). */
function createRootComponent(): SurfaceComponent {
	return {
		id: ROOT_ID,
		style: { className: "flex-1 h-full overflow-auto" },
		component: {
			type: "column",
			gap: "8px",
			children: { explicitList: [] },
		},
	} as unknown as SurfaceComponent;
}

/**
 * Applies generated components to a persisted page's component list exactly as the open builder
 * would: seed a root when the page has none, upsert the incoming components by id, then link every
 * new top-level component into the root's child list. Components the copilot did not mention are
 * left untouched — this is a merge, never a wholesale replacement.
 *
 * Staying behaviourally identical to `WidgetBuilder.handleApplyComponents` is the point: a FlowPilot
 * edit must land the same way whether or not the builder happens to be open.
 */
export function applyComponentsToPageTree(
	pageComponents: SurfaceComponent[],
	incoming: SurfaceComponent[],
): SurfaceComponent[] {
	if (incoming.length === 0) return pageComponents;

	const merged =
		pageComponents.length > 0 &&
		pageComponents.some((component) => component.id === ROOT_ID)
			? [...pageComponents]
			: [createRootComponent(), ...pageComponents];
	// The builder reads the root before upserting and writes it back last, so an incoming "root"
	// keeps its own props only while nothing new is linked into it.
	const rootBeforeApply = merged.find((component) => component.id === ROOT_ID);

	const referencedChildIds = new Set<string>();
	for (const component of incoming) {
		for (const childId of explicitChildList(component))
			referencedChildIds.add(childId);
	}
	const topLevelIds = incoming
		.filter(
			(component) =>
				!referencedChildIds.has(component.id) && component.id !== ROOT_ID,
		)
		.map((component) => component.id);

	const indexById = new Map(
		merged.map((component, index) => [component.id, index] as const),
	);
	for (const component of incoming) {
		const index = indexById.get(component.id);
		if (index === undefined) {
			indexById.set(component.id, merged.length);
			merged.push(component);
			continue;
		}
		merged[index] = { ...merged[index], ...component };
	}

	const rootIndex = indexById.get(ROOT_ID);
	if (topLevelIds.length > 0 && rootBeforeApply && rootIndex !== undefined) {
		const nextChildren = explicitChildList(rootBeforeApply);
		for (const id of topLevelIds) {
			if (!nextChildren.includes(id)) nextChildren.push(id);
		}
		merged[rootIndex] = {
			...merged[rootIndex],
			component: {
				...(rootBeforeApply.component as unknown as Record<string, unknown>),
				children: { explicitList: nextChildren },
			},
		} as unknown as SurfaceComponent;
	}

	return merged;
}

/**
 * The page a detached edit persists. Everything the page already owns is carried through — only the
 * component tree, canvas settings, and `updatedAt` change, matching the builder's own save.
 */
export function pageWithAppliedComponents(
	page: IPage,
	components: SurfaceComponent[],
	canvasSettings: CanvasSettings | undefined,
	timestamp: string,
): IPage {
	const next: IPage = {
		...page,
		components: applyComponentsToPageTree(page.components ?? [], components),
		updatedAt: timestamp,
	};
	// Canvas settings merge into the page's existing ones, as BuilderContext.setCanvasSettings does.
	if (canvasSettings)
		next.canvasSettings = { ...(page.canvasSettings ?? {}), ...canvasSettings };
	return next;
}
