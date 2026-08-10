import type { A2UIComponent, Hotspot } from "./types";

/**
 * A user interaction exposed by an A2UI component in the builder's Actions
 * tab. The two fallback flags keep the compatibility decision next to the
 * event: when no entry exists in the `eventHandlers` map, renderers may fall
 * back to the `*` handler and to the component's historical action path.
 */
export interface ComponentEventDefinition {
	id: string;
	label: string;
	description: string;
	legacyFallback: boolean;
	wildcardFallback: boolean;
}

type ComponentType = A2UIComponent["type"];

const event = (
	id: string,
	label: string,
	description: string,
	legacyFallback = true,
	wildcardFallback = true,
): ComponentEventDefinition => ({
	id,
	label,
	description,
	legacyFallback,
	wildcardFallback,
});

/**
 * An event that only ever runs an exact handler. Used for every event added
 * after its component shipped: a surface authored before the event existed
 * never opted into it, so inheriting `*` or `actions[0]` would silently start
 * running workflows on typing, focus, or hover.
 */
const exactEvent = (id: string, label: string, description: string) =>
	event(id, label, description, false, false);

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
			"The value was committed on blur or Enter and differs from the last committed value.",
		),
		exactEvent(
			"input",
			"Typing paused",
			"The user stopped typing for `debounceMs` (400 ms by default). Never fires per keystroke — use it for search-as-you-type and live validation.",
		),
		exactEvent(
			"submit",
			"Submitted",
			"The user pressed Enter, or ⌘/Ctrl+Enter in a multiline field. Carries `via` so a composer can tell Enter apart from clicking away.",
		),
		exactEvent("focus", "Focused", "The field received focus."),
		exactEvent(
			"blur",
			"Blurred",
			"The field lost focus, whether or not the value changed.",
		),
	],
	richText: [
		exactEvent(
			"change",
			"Document changed",
			"The document was edited and the user paused for `debounceMs` (600 ms by default). Carries the `plate_json::` value.",
		),
		exactEvent(
			"blur",
			"Blurred",
			"The editor lost focus, whether or not the document changed.",
		),
		exactEvent(
			"imageUploaded",
			"Image uploaded",
			"An image finished uploading into storage. Carries `path`, `name`, `size` and `type`.",
		),
		exactEvent(
			"imageUploadError",
			"Image upload failed",
			"An image could not be uploaded. Carries `name` and `message`.",
		),
	],
	select: [
		event("change", "Value changed", "The selected value changed."),
		exactEvent("open", "Opened", "The option list was opened."),
		exactEvent("close", "Closed", "The option list was closed."),
	],
	slider: [
		event(
			"change",
			"Value committed",
			"The slider value was committed at the end of the interaction.",
		),
		exactEvent(
			"input",
			"Dragging paused",
			"The user paused while dragging. Debounced by `debounceMs` (400 ms by default) so a drag cannot flood the board.",
		),
	],
	checkbox: [event("change", "State changed", "The checked state changed.")],
	switch: [event("change", "State changed", "The switch state changed.")],
	radioGroup: [
		event("change", "Value changed", "The selected radio value changed."),
	],
	dateTimeInput: [
		event("change", "Value changed", "The date or time value changed."),
	],
	fileInput: [event("change", "Files changed", "The selected files changed.")],
	imageInput: [
		event("change", "Images changed", "The selected images changed."),
	],
	voiceInput: [
		event(
			"change",
			"Recording changed",
			"The recording or transcript changed.",
		),
	],
	link: [event("navigate", "Navigate", "The link was activated.")],
	imageLabeler: [
		event("change", "Labels changed", "The set of image labels changed."),
	],
	imageHotspot: [
		event("hotspotClick", "Hotspot clicked", "An image hotspot was clicked."),
	],
	card: [event("click", "Click", "The card was clicked.")],
	modal: [event("close", "Close", "The modal was closed.")],
	tabs: [event("change", "Tab changed", "The active tab changed.")],
	accordion: [
		event(
			"change",
			"Sections changed",
			"The expanded accordion items changed.",
		),
	],
	table: [
		exactEvent(
			"rowClick",
			"Row clicked",
			"A body row was clicked. Carries the source row object and its index in the unsorted data.",
		),
		exactEvent(
			"cellClick",
			"Cell clicked",
			"A body cell was clicked. Carries the column id and the cell value.",
		),
		exactEvent(
			"selectionChange",
			"Selection changed",
			"Rows were selected or deselected. Requires `selectable`.",
		),
		exactEvent(
			"sortChange",
			"Sort changed",
			"The user sorted by a column or cleared the sort.",
		),
	],
	nivoChart: [
		exactEvent(
			"pointClick",
			"Data point clicked",
			"A bar, slice, node, or point was clicked. The payload shape follows the chart type.",
		),
	],
	plotlyChart: [
		exactEvent(
			"pointClick",
			"Data point clicked",
			"A point was clicked. Carries the trace name, point index, and x/y/z values.",
		),
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
