import { applyA2UIMessage } from "./apply-a2ui-message";
import type { A2UIServerMessage, Surface, SurfaceComponent } from "./types";

/**
 * Find the surface addressed by an element reference. A scoped reference can
 * name the surface itself or a widget instance hosted by that surface.
 */
export function resolveElementUpdateSurfaceId(
	surfaces: Map<string, Surface>,
	elementId: string,
): string | undefined {
	if (!elementId.includes("/")) return surfaces.keys().next().value;

	const prefix = elementId.slice(0, elementId.indexOf("/"));
	if (surfaces.has(prefix)) return prefix;

	for (const [surfaceId, surface] of surfaces) {
		for (const component of Object.values(surface.components)) {
			const data = component.component as unknown as Record<string, unknown>;
			if (
				(data.type === "widgetInstance" ||
					data.type === "microWidgetInstance") &&
				data.instanceId === prefix
			) {
				return surfaceId;
			}
		}
	}

	return undefined;
}

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
			const surfaceId = resolveElementUpdateSurfaceId(next, message.element_id);
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
