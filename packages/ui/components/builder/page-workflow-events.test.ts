import { describe, expect, test } from "bun:test";
import type { A2UIComponent } from "../a2ui/types";
import {
	WORKFLOW_EVENT_NODE_GAP,
	getPageComponentEvents,
	getPageWorkflowEvents,
	nameNewWorkflowEvent,
	nextWorkflowEventCoordinates,
	resolvePageBoardId,
	workflowEventKindsForComponent,
	workflowEventsForComponent,
} from "./page-workflow-events";

const nodes = {
	simple: {
		id: "simple-node",
		name: "events_simple",
		friendly_name: "Refresh page",
	},
	widget: {
		id: "widget-node",
		name: "events_widget_action",
		friendly_name: "Select entity",
	},
	generic: {
		id: "generic-node",
		name: "events_generic",
		friendly_name: "API request",
	},
	other: { id: "other-node", name: "http_get", friendly_name: "Fetch data" },
};

describe("page workflow event options", () => {
	test("includes Widget Action Event without an Instantiate Widget node", () => {
		expect(getPageWorkflowEvents(nodes, "Unnamed")).toEqual([
			{ nodeId: "simple-node", name: "Refresh page", kind: "simple" },
			{ nodeId: "widget-node", name: "Select entity", kind: "widget_action" },
		]);
	});

	test.each(["microWidgetInstance", "widgetInstance"])(
		"allows widget context events on %s",
		(type) => {
			expect(
				workflowEventsForComponent(
					getPageWorkflowEvents(nodes, "Unnamed"),
					type,
				).map((event) => event.nodeId),
			).toEqual(["simple-node", "widget-node"]);
		},
	);

	test.each([undefined, "button", "text"])(
		"keeps widget-only entries out of %s actions and page lifecycle hooks",
		(type) => {
			expect(
				workflowEventsForComponent(
					getPageWorkflowEvents(nodes, "Unnamed"),
					type,
				).map((event) => event.nodeId),
			).toEqual(["simple-node"]);
		},
	);

	test("preserves options supplied without a kind and handles boards with no nodes", () => {
		const event = { nodeId: "legacy-simple", name: "Legacy event" };
		expect(workflowEventsForComponent([event])).toEqual([event]);
		expect(getPageWorkflowEvents(undefined, "Unnamed")).toEqual([]);
		expect(
			workflowEventsForComponent(undefined, "microWidgetInstance"),
		).toEqual([]);
	});

	test("keeps comment and unnamed labels as fallbacks", () => {
		expect(
			getPageWorkflowEvents(
				{
					comment: {
						...nodes.widget,
						friendly_name: "",
						comment: "Camera selected",
					},
					unnamed: { ...nodes.simple, friendly_name: "", comment: null },
				},
				"Unnamed",
			).map((event) => event.name),
		).toEqual(["Camera selected", "Unnamed"]);
	});
});

describe("creating a workflow event", () => {
	test.each([
		["microWidgetInstance", ["widget_action", "simple"]],
		["widgetInstance", ["widget_action", "simple"]],
		["button", ["simple"]],
		[undefined, ["simple"]],
	] as const)("offers %s the event kinds it can receive", (type, kinds) => {
		expect(workflowEventKindsForComponent(type)).toEqual([...kinds]);
	});

	test("names the event after its handler, then the component, then the fallback", () => {
		expect(
			nameNewWorkflowEvent({
				eventName: " Entity clicked ",
				componentId: "map",
				fallback: "New Event",
			}),
		).toBe("Entity clicked");
		expect(
			nameNewWorkflowEvent({
				eventName: " ",
				componentId: "map",
				fallback: "New Event",
			}),
		).toBe("map");
		expect(nameNewWorkflowEvent({ fallback: "New Event" })).toBe("New Event");
	});

	test("never reuses an existing board event name", () => {
		expect(
			nameNewWorkflowEvent({ eventName: "Click", fallback: "New Event" }, [
				{ name: "click" },
				{ name: "Click 2" },
				{ name: "Other" },
			]),
		).toBe("Click 3");
	});

	test("places the node below the lowest root node, ignoring layered and unplaced nodes", () => {
		expect(
			nextWorkflowEventCoordinates({
				top: { coordinates: [40, 10, 0] },
				lowest: { coordinates: [120, 480, 0] },
				layered: { coordinates: [0, 9000, 0], layer: "layer" },
				unplaced: { coordinates: null },
			}),
		).toEqual([120, 480 + WORKFLOW_EVENT_NODE_GAP, 0]);
	});

	test("starts a board without placed root nodes at the origin", () => {
		expect(nextWorkflowEventCoordinates(undefined)).toEqual([0, 0, 0]);
		expect(
			nextWorkflowEventCoordinates({
				layered: { coordinates: [5, 5, 0], layer: "layer" },
			}),
		).toEqual([0, 0, 0]);
	});
});

describe("page board resolution", () => {
	test("uses the saved page board when the URL has no board", () => {
		expect(
			resolvePageBoardId(undefined, "page", {
				id: "page",
				boardId: "saved-board",
			}),
		).toBe("saved-board");
	});
	test("keeps an explicit route board and ignores a stale loaded page", () => {
		expect(
			resolvePageBoardId("route-board", "page", {
				id: "page",
				boardId: "saved-board",
			}),
		).toBe("route-board");
		expect(
			resolvePageBoardId(undefined, "next-page", {
				id: "page",
				boardId: "old-board",
			}),
		).toBeUndefined();
		expect(resolvePageBoardId(undefined, "page", null)).toBeUndefined();
	});
});

describe("page widget event rows", () => {
	const inline = {
		id: "inline-1",
		type: "widgetInstance",
		instanceId: "inline-1",
		widgetId: "inline",
		inlineWidgetDef: {
			actions: [
				{
					id: "selected",
					label: "Select item",
					description: "An item was selected",
				},
			],
		},
	} as A2UIComponent;

	test("exposes initially unbound inline widget actions with runtime fallback rules", () => {
		expect(getPageComponentEvents(inline)).toEqual([
			{
				id: "selected",
				label: "Select item",
				description: "An item was selected",
				legacyFallback: true,
				wildcardFallback: true,
			},
		]);
	});

	test("uses a saved widget reference before the inline definition, including no declared actions", () => {
		expect(
			getPageComponentEvents(inline, { actions: [{ id: "saved" }] }).map(
				(event) => event.id,
			),
		).toEqual(["saved"]);
		expect(getPageComponentEvents(inline, {})).toEqual([]);
	});

	test("ignores malformed and duplicate action IDs without treating the wildcard as a declared event", () => {
		expect(
			getPageComponentEvents(inline, {
				actions: [
					null,
					{},
					{ id: 1 },
					{ id: "" },
					{ id: "*" },
					{ id: "saved" },
					{ id: "saved" },
				],
			}).map((event) => event.id),
		).toEqual(["saved"]);
	});

	test("keeps packaged widget contracts and existing component events intact", () => {
		const packaged = {
			id: "map",
			type: "microWidgetInstance",
			instanceId: "map",
			packageId: "geo",
			widgetId: "map",
			packageVersion: "1",
			contract: {
				contractVersion: 1,
				id: "map",
				events: { entityClicked: { description: "Entity clicked" } },
			},
		} as A2UIComponent;
		expect(
			getPageComponentEvents(packaged, { actions: [{ id: "unrelated" }] }).map(
				(event) => event.id,
			),
		).toEqual(["entityClicked"]);
		expect(
			getPageComponentEvents({
				type: "button",
				label: { literalString: "Run" },
			} as A2UIComponent).map((event) => event.id),
		).toContain("click");
	});
});
