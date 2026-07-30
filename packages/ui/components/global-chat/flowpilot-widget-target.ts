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

export type FlowPilotWidgetTargetResolution =
	| {
			ok: true;
			mode: FlowPilotWidgetMode;
			appId: string;
			surface: FlowPilotWidgetSurfaceTarget | null;
	  }
	| {
			ok: false;
			message: string;
	  };

const trimmed = (value: string | undefined) => value?.trim() ?? "";

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
 * Resolves whether FlowPilot should edit the visible builder or create a persisted page.
 *
 * Explicit persisted-page targeting always wins over ambient UI state. This keeps an unrelated
 * mounted builder from hijacking a request for another app, board, or page. Callers that really
 * intend to edit the open surface can say mode="edit" (or omit every persisted-page target).
 */
export function resolveFlowPilotWidgetTarget(
	request: FlowPilotWidgetTargetRequest,
): FlowPilotWidgetTargetResolution {
	const requestedMode = trimmed(request.mode).toLowerCase();
	if (requestedMode && requestedMode !== "create" && requestedMode !== "edit") {
		return {
			ok: false,
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
				message:
					"Creating a page requires app_id. Pass mode='edit' with no persisted-page target to edit the currently open builder.",
			};
		}
		return { ok: true, mode, appId, surface: null };
	}

	const surface = request.surface;
	if (!surface) {
		return {
			ok: false,
			message:
				"Editing UI requires an open widget/page builder. Open the intended builder or use mode='create' with app_id.",
		};
	}
	if (appId && surface.appId && appId !== surface.appId) {
		return {
			ok: false,
			message: `The open builder belongs to app '${surface.appId}', not requested app '${appId}'. Use mode='create' for a new page or open the intended builder.`,
		};
	}
	if (pageId && (surface.kind !== "page" || pageId !== surface.pageId)) {
		return {
			ok: false,
			message: `The open builder is not page '${pageId}'. Use mode='create' for a new page or open that page before editing it.`,
		};
	}
	if (boardId && boardId !== surface.boardId) {
		return {
			ok: false,
			message: `The open builder belongs to board '${surface.boardId ?? "unknown"}', not requested board '${boardId}'. Use mode='create' for a new page or open the intended page.`,
		};
	}

	const targetAppId = surface.appId || appId;
	if (!targetAppId) {
		return {
			ok: false,
			message:
				"The open widget/page builder has no app scope. Reopen it from an app before using FlowPilot.",
		};
	}
	return { ok: true, mode, appId: targetAppId, surface };
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
	].join("\u0000");
}
