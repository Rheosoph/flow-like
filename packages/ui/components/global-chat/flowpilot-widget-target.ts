export type FlowPilotWidgetMode = "create" | "edit";

export interface FlowPilotWidgetSurfaceTarget {
	kind: "page" | "widget";
	appId?: string;
	boardId?: string;
	pageId?: string;
	widgetId?: string;
}

export interface FlowPilotWidgetTargetRequest {
	mode?: string;
	appId?: string;
	boardId?: string;
	pageId?: string;
	pageName?: string;
	route?: string;
	surface?: FlowPilotWidgetSurfaceTarget | null;
}

/** Persisted page an edit targets when no matching builder is mounted. */
export interface FlowPilotWidgetPageTarget {
	pageId?: string;
	route?: string;
	pageName?: string;
	boardId?: string;
	/** The app scope came from the mounted builder rather than an explicit app_id. */
	appIdFromSurface: boolean;
}

/** Canonical form of a page route, so "dashboard", "/dashboard", and "Dashboard" all agree. */
export function slugifyRoute(value: string): string {
	const slug = value
		.trim()
		.toLowerCase()
		.replace(/^\/+/, "")
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "");
	return `/${slug || "page"}`;
}

export type FlowPilotWidgetTargetResolution =
	| {
			ok: true;
			mode: "create";
			appId: string;
			surface: null;
			pageTarget: null;
	  }
	| {
			ok: true;
			mode: "edit";
			appId: string;
			surface: FlowPilotWidgetSurfaceTarget;
			pageTarget: null;
	  }
	| {
			ok: true;
			mode: "edit";
			appId: string;
			surface: null;
			pageTarget: FlowPilotWidgetPageTarget;
	  }
	| {
			ok: false;
			code?: string;
			message: string;
	  };

const trimmed = (value: string | undefined) => value?.trim() ?? "";

/** NUL, so no id, route, or page name can forge a scope boundary. */
const SCOPE_SEPARATOR = String.fromCharCode(0);

/** Only an authoritative 404/local "not found" permits a create probe to continue. */
export function isFlowPilotPageNotFoundError(error: unknown) {
	if (
		error &&
		typeof error === "object" &&
		"status" in error &&
		(error as { status?: unknown }).status === 404
	) {
		return true;
	}
	return (
		error instanceof Error &&
		/^page not found(?::|$)/i.test(error.message.trim())
	);
}

/**
 * Resolves whether FlowPilot should edit the visible builder, edit a persisted page with no builder
 * mounted, or create a new page.
 *
 * Explicit persisted-page targeting always wins over ambient UI state. A named page the mounted
 * builder is not showing resolves to a detached edit rather than an error — an unrelated open
 * builder must never decide which page gets rewritten, nor block editing another one.
 */
export function resolveFlowPilotWidgetTarget(
	request: FlowPilotWidgetTargetRequest,
): FlowPilotWidgetTargetResolution {
	const requestedMode = trimmed(request.mode).toLowerCase();
	if (requestedMode && requestedMode !== "create" && requestedMode !== "edit") {
		return {
			ok: false,
			code: "FLOWPILOT_WIDGET_MODE_INVALID",
			message: `Unsupported flowpilot_widget mode '${request.mode}'. Use 'create' or 'edit'.`,
		};
	}

	const appId = trimmed(request.appId);
	const boardId = trimmed(request.boardId);
	const pageId = trimmed(request.pageId);
	const pageName = trimmed(request.pageName);
	const route = trimmed(request.route);
	const hasPersistedPageTarget = Boolean(
		appId || boardId || pageId || pageName || route,
	);
	// An unqualified request still defaults to create. Rewriting an existing page merely because the
	// caller named it is the one outcome this resolver must never produce silently; create refuses an
	// existing page id loudly instead.
	const mode: FlowPilotWidgetMode =
		requestedMode === "create" ||
		(!requestedMode && hasPersistedPageTarget) ||
		(!requestedMode && !request.surface)
			? "create"
			: "edit";

	if (mode === "create") {
		if (!appId) {
			return {
				ok: false,
				code: "FLOWPILOT_WIDGET_CREATE_APP_ID_REQUIRED",
				message:
					"Creating a page requires app_id. To edit an existing page pass mode='edit' with app_id and page_id (or route/page_name); mode='edit' with no target edits the currently open builder.",
			};
		}
		return { ok: true, mode, appId, surface: null, pageTarget: null };
	}

	const surface = request.surface ?? null;
	const namedPage = pageId || route || pageName;
	const scopeAppId = appId || surface?.appId || "";
	// The builder carries no route or name, so a request identified only that way can never be proven
	// to be the mounted page and is resolved against persisted pages instead.
	const surfaceMismatch = ((): string | null => {
		if (!surface) return null;
		if (appId && surface.appId && appId !== surface.appId)
			return `The open builder belongs to app '${surface.appId}', not requested app '${appId}'.`;
		if (pageId && (surface.kind !== "page" || pageId !== surface.pageId))
			return `The open builder is not page '${pageId}'.`;
		if (boardId && boardId !== surface.boardId)
			return `The open builder belongs to board '${surface.boardId ?? "unknown"}', not requested board '${boardId}'.`;
		if (!pageId && (route || pageName))
			return route
				? `The open builder cannot be matched to route '${route}'.`
				: `The open builder cannot be matched to page name '${pageName}'.`;
		return null;
	})();

	if (surface && !surfaceMismatch) {
		const targetAppId = surface.appId || appId;
		if (!targetAppId) {
			return {
				ok: false,
				code: "FLOWPILOT_WIDGET_SURFACE_APP_UNKNOWN",
				message:
					"The open widget/page builder has no app scope. Reopen it from an app, or pass app_id with page_id/route/page_name to edit a persisted page.",
			};
		}
		return { ok: true, mode, appId: targetAppId, surface, pageTarget: null };
	}

	if (namedPage) {
		if (!scopeAppId) {
			return {
				ok: false,
				code: "FLOWPILOT_WIDGET_EDIT_APP_ID_REQUIRED",
				message:
					"Editing a persisted page requires app_id alongside page_id, route, or page_name.",
			};
		}
		return {
			ok: true,
			mode,
			appId: scopeAppId,
			surface: null,
			pageTarget: {
				...(pageId ? { pageId } : {}),
				...(route ? { route } : {}),
				...(pageName ? { pageName } : {}),
				...(boardId ? { boardId } : {}),
				appIdFromSurface: !appId && Boolean(surface?.appId),
			},
		};
	}

	// Reusable-widget editing has no persisted-target form: without a mounted widget builder nothing
	// names which widget to rewrite.
	return {
		ok: false,
		code: "FLOWPILOT_WIDGET_EDIT_TARGET_REQUIRED",
		message: surfaceMismatch
			? `${surfaceMismatch} Pass app_id with page_id (or route/page_name) to edit a persisted page directly, or open the intended builder.`
			: "Editing UI requires either an open widget/page builder or an explicit persisted page: pass app_id with page_id, route, or page_name. Use mode='create' with app_id for a new page.",
	};
}

/**
 * Target-aware scope for crash/retry idempotency. Equal prose on two pages or boards must create
 * two artifacts, while an exact retry of one target should resolve to its first persisted page.
 */
export function flowPilotWidgetCreationScope(options: {
	appId: string;
	boardId?: string;
	pageId?: string;
	route?: string;
	pageName?: string;
}) {
	const pageTarget =
		trimmed(options.pageId) ||
		trimmed(options.route) ||
		trimmed(options.pageName) ||
		"generated-page";
	return [
		`app:${trimmed(options.appId)}`,
		`board:${trimmed(options.boardId) || "unresolved"}`,
		`page:${pageTarget}`,
	].join(SCOPE_SEPARATOR);
}
