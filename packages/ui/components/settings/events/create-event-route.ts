import { normalizeRoutePath } from "../../../lib/route-path";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IAppRouteState } from "../../../state/backend-state/route-state";

/**
 * A page or UI event owns a path. The backend may hand a new event another id,
 * so the route points at the saved event. A failed route never fails the
 * creation: the event exists and the path can be set from its row.
 */
export async function saveCreatedEventRoute(
	routeState: Pick<IAppRouteState, "setRoute">,
	appId: string,
	saved: IEvent,
	path: string | null | undefined,
	uiEventTypes: ReadonlySet<string>,
): Promise<boolean> {
	if (!uiEventTypes.has(saved.event_type) && !saved.default_page_id)
		return false;
	try {
		await routeState.setRoute(appId, normalizeRoutePath(path), saved.id);
		return true;
	} catch (error) {
		console.error("Failed to create route for UI event:", error);
		return false;
	}
}
