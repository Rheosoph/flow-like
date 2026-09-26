import type { INode } from "../../lib/schema/flow/node";
import {
	type ComponentEventDefinition,
	getComponentEventDefinitions,
} from "../a2ui/component-event-manifest";
import type { A2UIComponent } from "../a2ui/types";

export type WorkflowEventKind = "simple" | "widget_action";

export interface WorkflowEventInfo {
	nodeId: string;
	name: string;
	kind?: WorkflowEventKind;
}

export interface CreateWorkflowEventRequest {
	name: string;
	kind: WorkflowEventKind;
}

export const WORKFLOW_EVENT_NODE_NAMES: Readonly<
	Record<WorkflowEventKind, string>
> = {
	simple: "events_simple",
	widget_action: "events_widget_action",
};

export const WORKFLOW_EVENT_NODE_GAP = 250;

function workflowEventKind(nodeName: string): WorkflowEventKind | undefined {
	return (Object.keys(WORKFLOW_EVENT_NODE_NAMES) as WorkflowEventKind[]).find(
		(kind) => WORKFLOW_EVENT_NODE_NAMES[kind] === nodeName,
	);
}

export function isWidgetInstanceType(componentType?: string): boolean {
	return (
		componentType === "microWidgetInstance" ||
		componentType === "widgetInstance"
	);
}

/** Only use a loaded page's board after that page matches the current route. */
export function resolvePageBoardId(
	boardId: string | undefined,
	pageId: string,
	page: { id: string; boardId?: string } | null,
): string | undefined {
	return boardId || (page?.id === pageId ? page.boardId : undefined);
}

export function getPageWorkflowEvents(
	nodes:
		| Record<string, Pick<INode, "id" | "name" | "friendly_name" | "comment">>
		| undefined,
	unnamedEvent: string,
): WorkflowEventInfo[] {
	return Object.values(nodes ?? {}).flatMap((node) => {
		const kind = workflowEventKind(node.name);
		return kind
			? [
					{
						nodeId: node.id,
						name: node.friendly_name || node.comment || unnamedEvent,
						kind,
					},
				]
			: [];
	});
}

/** Page lifecycle hooks and ordinary components have no widget instance context. */
export function workflowEventsForComponent(
	events: readonly WorkflowEventInfo[] | undefined,
	componentType?: string,
): WorkflowEventInfo[] {
	const isWidget = isWidgetInstanceType(componentType);
	return (events ?? []).filter(
		(event) => isWidget || event.kind !== "widget_action",
	);
}

/** Widget instances offer the event that carries the widget payload and instance ID first. */
export function workflowEventKindsForComponent(
	componentType?: string,
): WorkflowEventKind[] {
	return isWidgetInstanceType(componentType)
		? ["widget_action", "simple"]
		: ["simple"];
}

/** Names a new event after the handler it is created for without reusing a board event name. */
export function nameNewWorkflowEvent(
	source: { eventName?: string; componentId?: string; fallback: string },
	existing: readonly Pick<WorkflowEventInfo, "name">[] = [],
): string {
	const base =
		[source.eventName, source.componentId]
			.map((candidate) => candidate?.trim())
			.find(Boolean) ?? source.fallback;
	const taken = new Set(existing.map((event) => event.name.toLowerCase()));
	let name = base;
	for (let suffix = 2; taken.has(name.toLowerCase()); suffix++) {
		name = `${base} ${suffix}`;
	}
	return name;
}

/** Below the lowest root-layer node, or at the origin on a board without placed root nodes. */
export function nextWorkflowEventCoordinates(
	nodes: Record<string, Pick<INode, "coordinates" | "layer">> | undefined,
): [number, number, number] {
	let lowest: { x: number; y: number } | undefined;
	for (const node of Object.values(nodes ?? {})) {
		const [x, y] = node.coordinates ?? [];
		if (node.layer || !Number.isFinite(x) || !Number.isFinite(y)) continue;
		if (!lowest || y > lowest.y) lowest = { x, y };
	}
	return lowest ? [lowest.x, lowest.y + WORKFLOW_EVENT_NODE_GAP, 0] : [0, 0, 0];
}

/** Saved page references take precedence over inline definitions, as in the renderer. */
export function getPageComponentEvents(
	component: A2UIComponent,
	savedDefinition?: { actions?: unknown[] },
): ComponentEventDefinition[] {
	const events = getComponentEventDefinitions(component);
	if (component.type !== "widgetInstance") return events;
	const inline = (
		component as A2UIComponent & { inlineWidgetDef?: { actions?: unknown[] } }
	).inlineWidgetDef;
	const definition = savedDefinition ?? inline;
	const seen = new Set(events.map((event) => event.id));
	for (const action of definition?.actions ?? []) {
		if (
			!action ||
			typeof action !== "object" ||
			!("id" in action) ||
			typeof action.id !== "string" ||
			!action.id ||
			action.id === "*" ||
			seen.has(action.id)
		)
			continue;
		seen.add(action.id);
		events.push({
			id: action.id,
			label:
				"label" in action && typeof action.label === "string" && action.label
					? action.label
					: action.id,
			description:
				"description" in action && typeof action.description === "string"
					? action.description
					: `The widget emitted “${action.id}”.`,
			legacyFallback: true,
			wildcardFallback: true,
		});
	}
	return events;
}
