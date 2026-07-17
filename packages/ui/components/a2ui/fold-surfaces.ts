import { applyA2UIMessage } from "./apply-a2ui-message";
import type { A2UIServerMessage, Surface, SurfaceComponent } from "./types";

/**
 * Pure reducer that folds one a2ui server message into a surface map — the map-level subset of
 * {@link useSurfaceManager}'s `handleServerMessage` (create on `beginRendering`, delete on
 * `deleteSurface`, everything else delegated to {@link applyA2UIMessage}), without the
 * optimistic-update bookkeeping the live manager tracks.
 *
 * Used to reconstruct the surfaces an app pushed when its run is driven headlessly (e.g. FlowPilot's
 * `call_app_chat`) where there is no mounted SurfaceManager to receive the pushes. Pure side-effect
 * messages (navigation, dialogs, query params, screen/page/global state) are ignored so replaying a
 * captured run can never navigate or mutate the host that displays it.
 */
export function foldA2UIServerMessage(
	surfaces: Map<string, Surface>,
	message: A2UIServerMessage,
): Map<string, Surface> {
	const next = new Map(surfaces);

	switch (message.type) {
		case "beginRendering": {
			const components: Record<string, SurfaceComponent> = {};
			for (const comp of message.components) components[comp.id] = comp;
			next.set(message.surfaceId, {
				id: message.surfaceId,
				rootComponentId: message.rootComponentId,
				components,
				dataModel: message.dataModel,
				catalogId: message.catalogId,
			});
			break;
		}
		case "deleteSurface": {
			next.delete(message.surfaceId);
			break;
		}
		case "upsertElement": {
			// Resolve the multi-surface fallback (first key) before delegating, matching the live
			// surface manager — applyA2UIMessage falls back to whichever surface it is handed.
			const { element_id } = message;
			const surfaceId = element_id.includes("/")
				? element_id.split("/", 2)[0]
				: Array.from(next.keys())[0];
			if (!surfaceId) break;
			const existing = next.get(surfaceId);
			if (!existing) break;
			next.set(surfaceId, applyA2UIMessage(existing, message));
			break;
		}
		case "surfaceUpdate":
		case "setCanvasSettings":
		case "dataModelUpdate":
		case "createElement":
		case "removeElement": {
			const existing = next.get(message.surfaceId);
			if (!existing) break;
			next.set(message.surfaceId, applyA2UIMessage(existing, message));
			break;
		}
	}

	return next;
}
