import type { A2UIComponent, Hotspot } from "./types";

/**
 * A user interaction exposed by an A2UI component in the builder's Actions
 * tab. `legacyFallback` keeps the compatibility decision next to the event:
 * when no entry exists in the new `eventHandlers` map, renderers may fall back
 * to the component's historical action path.
 */
export interface ComponentEventDefinition {
	id: string;
	label: string;
	description: string;
	legacyFallback: boolean;
}

type ComponentType = A2UIComponent["type"];

const event = (
	id: string,
	label: string,
	description: string,
	legacyFallback = true,
): ComponentEventDefinition => ({
	id,
	label,
	description,
	legacyFallback,
});

/** Static interaction contracts implemented by the built-in renderers. */
export const COMPONENT_EVENT_MANIFEST = {
	boundingBoxOverlay: [
		event("boxClick", "Box clicked", "A bounding box was clicked."),
	],
	button: [event("click", "Click", "The button was clicked.")],
	feedback: [event("submit", "Submit", "The user submitted their feedback.")],
	textField: [
		event(
			"change",
			"Value committed",
			"The field value was committed on blur or Enter.",
		),
	],
	select: [event("change", "Change", "The selected value changed.")],
	slider: [
		event(
			"change",
			"Value committed",
			"The slider value was committed at the end of the interaction.",
		),
	],
	checkbox: [event("change", "Change", "The checked state changed.")],
	switch: [event("change", "Change", "The switch state changed.")],
	radioGroup: [event("change", "Change", "The selected radio value changed.")],
	dateTimeInput: [event("change", "Change", "The date or time value changed.")],
	fileInput: [event("change", "Change", "The selected files changed.")],
	imageInput: [event("change", "Change", "The selected images changed.")],
	voiceInput: [
		event("change", "Change", "The recording or transcript changed."),
	],
	link: [event("navigate", "Navigate", "The link was activated.")],
	imageLabeler: [event("change", "Change", "The set of image labels changed.")],
	imageHotspot: [
		event("hotspotClick", "Hotspot clicked", "An image hotspot was clicked."),
	],
	card: [event("click", "Click", "The card was clicked.")],
	modal: [event("close", "Close", "The modal was closed.")],
	tabs: [event("change", "Change", "The active tab changed.")],
	accordion: [
		event("change", "Change", "The expanded accordion items changed."),
	],
	drawer: [event("close", "Close", "The drawer was closed.")],
	popover: [event("close", "Close", "The popover was closed.")],
	choiceMenu: [
		event("choiceSelect", "Choice selected", "A choice was selected."),
	],
	inventoryGrid: [
		event("itemClick", "Item clicked", "An inventory item was clicked."),
	],
	miniMap: [
		event("markerClick", "Marker clicked", "A mini-map marker was clicked."),
	],
	geoMap: [
		event("markerClick", "Marker clicked", "A map marker was clicked."),
		event("markerDragEnd", "Marker moved", "A draggable marker was moved."),
		event("routeClick", "Route clicked", "A map route was clicked."),
		event("locate", "Location found", "The locate control found a location."),
		// Viewport changes have always emitted an A2UI message, but unlike the
		// events above they did not execute component.actions[0]. Falling back here
		// would make old maps unexpectedly run a workflow on every pan/zoom frame.
		event(
			"viewportChange",
			"Viewport changed",
			"The user panned, zoomed, rotated, or tilted the map.",
			false,
		),
	],
	graph: [
		event("nodeClick", "Node selected", "A graph node was selected."),
		event("edgeClick", "Edge selected", "A graph edge was selected."),
	],
	ontologyGraph: [
		event("nodeClick", "Node selected", "An ontology object was selected."),
		event("edgeClick", "Edge selected", "An ontology relation was selected."),
	],
	calendar: [
		event("open", "Event opened", "A calendar event was opened."),
		event("create", "Event created", "A calendar event was created."),
		event("update", "Event updated", "A calendar event was updated."),
		event("move", "Event moved", "A calendar event was moved."),
		event("resize", "Event resized", "A calendar event was resized."),
		event("delete", "Event deleted", "A calendar event was deleted."),
	],
	gantt: [
		event("open", "Task opened", "A task was opened."),
		event("create", "Task created", "A task was created."),
		event("update", "Task updated", "A task was updated."),
		event("move", "Task moved", "A task was moved."),
		event("resize", "Task resized", "A task was resized."),
		event("delete", "Task deleted", "A task was deleted."),
		event("link", "Tasks linked", "A dependency was created between tasks."),
		event("reorder", "Task reordered", "A task was reordered in the list."),
	],
} as const satisfies Partial<
	Record<ComponentType, readonly ComponentEventDefinition[]>
>;

function staticEventsFor(
	type: ComponentType,
): readonly ComponentEventDefinition[] {
	return (
		(
			COMPONENT_EVENT_MANIFEST as Partial<
				Record<ComponentType, readonly ComponentEventDefinition[]>
			>
		)[type] ?? []
	);
}

function readLiteralHotspots(value: unknown): Hotspot[] {
	if (!value || typeof value !== "object") return [];

	if ("literalJson" in value && typeof value.literalJson === "string") {
		try {
			const parsed: unknown = JSON.parse(value.literalJson);
			return Array.isArray(parsed) ? (parsed as Hotspot[]) : [];
		} catch {
			return [];
		}
	}

	// Some hand-authored surfaces use literalOptions for arbitrary JSON arrays.
	// Accept it here without treating path/default values as literal contracts.
	if ("literalOptions" in value && Array.isArray(value.literalOptions)) {
		return value.literalOptions as unknown as Hotspot[];
	}

	return [];
}

function literalHotspotEvents(
	component: Extract<A2UIComponent, { type: "imageHotspot" }>,
): ComponentEventDefinition[] {
	const definitions: ComponentEventDefinition[] = [];
	const seen = new Set<string>();

	for (const hotspot of readLiteralHotspots(component.hotspots)) {
		if (!hotspot || typeof hotspot !== "object") continue;
		const action =
			typeof hotspot.action === "string" ? hotspot.action.trim() : "";
		if (!action || seen.has(action) || action === "hotspotClick") continue;
		seen.add(action);

		const hotspotName =
			typeof hotspot.label === "string" && hotspot.label.trim()
				? hotspot.label.trim()
				: typeof hotspot.id === "string" && hotspot.id.trim()
					? hotspot.id.trim()
					: null;
		definitions.push(
			event(
				action,
				action,
				hotspotName
					? `Emitted by the “${hotspotName}” hotspot.`
					: "Emitted by an image hotspot.",
				// Literal hotspot actions historically emitted their own A2UI event;
				// the generic component action is already covered by hotspotClick.
				false,
			),
		);
	}

	return definitions;
}

function microWidgetEvents(
	component: Extract<A2UIComponent, { type: "microWidgetInstance" }>,
): ComponentEventDefinition[] {
	return Object.entries(component.contract?.events ?? {}).map(([id, spec]) =>
		event(id, id, spec.description ?? `The widget emitted “${id}”.`),
	);
}

/**
 * Returns the event contract for a concrete component. Built-in events come
 * from the static manifest; package widget events and literal hotspot action
 * names are appended dynamically from the component data.
 */
export function getComponentEventDefinitions(
	component: A2UIComponent,
): ComponentEventDefinition[] {
	const definitions = [...staticEventsFor(component.type)];

	if (component.type === "microWidgetInstance") {
		definitions.push(...microWidgetEvents(component));
	} else if (component.type === "imageHotspot") {
		definitions.push(...literalHotspotEvents(component));
	}

	const seen = new Set<string>();
	return definitions.filter((definition) => {
		if (seen.has(definition.id)) return false;
		seen.add(definition.id);
		return true;
	});
}
