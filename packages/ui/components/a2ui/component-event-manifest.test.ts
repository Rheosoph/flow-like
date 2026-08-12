import { describe, expect, test } from "bun:test";
import {
	COMPONENT_EVENT_MANIFEST,
	getComponentEventDefinitions,
} from "./component-event-manifest";
import type { A2UIComponent } from "./types";

function component(type: A2UIComponent["type"]): A2UIComponent {
	return { type } as A2UIComponent;
}

describe("component event manifest", () => {
	test("covers every built-in component that currently dispatches configured actions", () => {
		const expected: Record<string, string[]> = {
			boundingBoxOverlay: ["boxClick"],
			button: ["click"],
			feedback: ["submit"],
			textField: ["change", "input", "submit", "focus", "blur"],
			richText: ["change", "blur", "imageUploaded", "imageUploadError"],
			select: ["change", "open", "close"],
			slider: ["change", "input"],
			checkbox: ["change"],
			switch: ["change"],
			radioGroup: ["change"],
			dateTimeInput: ["change"],
			fileInput: ["change"],
			imageInput: ["change"],
			voiceInput: ["change"],
			link: ["navigate"],
			imageLabeler: ["change"],
			imageHotspot: ["hotspotClick"],
			card: ["click"],
			modal: ["close"],
			tabs: ["change"],
			accordion: ["change"],
			drawer: ["close"],
			popover: ["close"],
			choiceMenu: ["choiceSelect"],
			inventoryGrid: ["itemClick"],
			miniMap: ["markerClick"],
			geoMap: [
				"markerClick",
				"markerDragEnd",
				"routeClick",
				"locate",
				"viewportChange",
			],
			graph: ["nodeClick", "edgeClick"],
			ontologyGraph: ["nodeClick", "edgeClick"],
			calendar: ["open", "create", "update", "move", "resize", "delete"],
			gantt: [
				"open",
				"create",
				"update",
				"move",
				"resize",
				"delete",
				"link",
				"reorder",
			],
			table: ["rowClick", "cellClick", "selectionChange", "sortChange"],
			nivoChart: ["pointClick"],
			plotlyChart: ["pointClick"],
		};

		expect(Object.keys(COMPONENT_EVENT_MANIFEST).sort()).toEqual(
			Object.keys(expected).sort(),
		);
		for (const [type, ids] of Object.entries(expected)) {
			expect(
				getComponentEventDefinitions(
					component(type as A2UIComponent["type"]),
				).map((definition) => definition.id),
			).toEqual(ids);
		}
	});

	test("keeps the legacy fallback except for high-frequency viewport changes", () => {
		const definitions = getComponentEventDefinitions(component("geoMap"));
		expect(
			definitions.map(({ id, legacyFallback }) => ({ id, legacyFallback })),
		).toEqual([
			{ id: "markerClick", legacyFallback: true },
			{ id: "markerDragEnd", legacyFallback: true },
			{ id: "routeClick", legacyFallback: true },
			{ id: "locate", legacyFallback: true },
			{ id: "viewportChange", legacyFallback: false },
		]);
	});

	test("events added after a component shipped inherit neither fallback", () => {
		const inherited = new Set(["change"]);

		for (const type of ["textField", "slider", "select"] as const) {
			for (const definition of getComponentEventDefinitions(component(type))) {
				const expected = inherited.has(definition.id);
				expect({
					type,
					id: definition.id,
					legacyFallback: definition.legacyFallback,
					wildcardFallback: definition.wildcardFallback,
				}).toEqual({
					type,
					id: definition.id,
					legacyFallback: expected,
					wildcardFallback: expected,
				});
			}
		}

		for (const type of [
			"table",
			"nivoChart",
			"plotlyChart",
			"richText",
		] as const) {
			for (const definition of getComponentEventDefinitions(component(type))) {
				expect(definition.legacyFallback).toBe(false);
				expect(definition.wildcardFallback).toBe(false);
			}
		}
	});

	test("reads event names and descriptions from a micro-widget contract", () => {
		const definitions = getComponentEventDefinitions({
			id: "sales-chart",
			type: "microWidgetInstance",
			instanceId: "sales-1",
			packageId: "sales",
			widgetId: "chart",
			packageVersion: "1.0.0",
			contract: {
				contractVersion: 1,
				id: "chart",
				events: {
					pointSelected: { description: "A bucket was clicked" },
					refreshRequested: {},
				},
			},
		} as A2UIComponent);

		expect(definitions).toEqual([
			{
				id: "pointSelected",
				label: "pointSelected",
				description: "A bucket was clicked",
				legacyFallback: true,
				wildcardFallback: true,
			},
			{
				id: "refreshRequested",
				label: "refreshRequested",
				description: "The widget emitted “refreshRequested”.",
				legacyFallback: true,
				wildcardFallback: true,
			},
		]);
	});

	test("merges unique literal image-hotspot action names", () => {
		const definitions = getComponentEventDefinitions({
			id: "scene-hotspots",
			type: "imageHotspot",
			src: { literalString: "/scene.png" },
			hotspots: {
				literalJson: JSON.stringify([
					{ id: "door", x: 10, y: 20, label: "Door", action: "openDoor" },
					{ id: "window", x: 40, y: 20, action: "lookOutside" },
					{ id: "duplicate", x: 20, y: 40, action: "openDoor" },
					{ id: "empty", x: 0, y: 0, action: "  " },
					{ id: "generic", x: 0, y: 0, action: "hotspotClick" },
				]),
			},
		} as A2UIComponent);

		expect(definitions.map((definition) => definition.id)).toEqual([
			"hotspotClick",
			"openDoor",
			"lookOutside",
		]);
		expect(definitions[1]).toEqual({
			id: "openDoor",
			label: "openDoor",
			description: "Emitted by the “Door” hotspot.",
			legacyFallback: false,
			wildcardFallback: true,
		});
		expect(definitions[2]?.description).toBe(
			"Emitted by the “window” hotspot.",
		);
	});

	test("ignores bound and malformed hotspot data", () => {
		const bound = getComponentEventDefinitions({
			id: "bound-hotspots",
			type: "imageHotspot",
			src: { literalString: "/scene.png" },
			hotspots: {
				path: "$.hotspots",
				defaultValue: [{ id: "door", action: "openDoor" }],
			},
		} as A2UIComponent);
		const malformed = getComponentEventDefinitions({
			id: "malformed-hotspots",
			type: "imageHotspot",
			src: { literalString: "/scene.png" },
			hotspots: { literalJson: "not json" },
		} as A2UIComponent);

		expect(bound.map((definition) => definition.id)).toEqual(["hotspotClick"]);
		expect(malformed.map((definition) => definition.id)).toEqual([
			"hotspotClick",
		]);
	});
});
