/**
 * Drift-prevention tests: the validator's known-prop tables must stay in sync
 * with the A2UIComponent union in components/a2ui/types.ts (mirrored by the
 * compile-time-checked manifest in components/a2ui/component-prop-manifest.ts).
 */
import { describe, expect, test } from "bun:test";
import {
	COMPONENT_BASE_PROPS,
	COMPONENT_PROPS,
} from "../a2ui/component-prop-manifest";
import type { SurfaceComponent } from "../a2ui/types";
import {
	BASE_PROPS,
	KNOWN_PROPS,
	validateCanvasSettings,
	validateComponents,
} from "./validateComponents";

/** The only props the validator may accept beyond the types.ts interfaces. */
const ALLOWED_RUNTIME_ONLY_PROPS: Record<string, readonly string[]> = {
	widgetInstance: ["inlineWidgetDef"],
};

describe("KNOWN_PROPS drift protection", () => {
	test("has an entry for every component type in the A2UIComponent union", () => {
		const manifestTypes = Object.keys(COMPONENT_PROPS).sort();
		const validatorTypes = Object.keys(KNOWN_PROPS).sort();
		expect(validatorTypes).toEqual(manifestTypes);
	});

	test("accepts every prop declared on each component interface", () => {
		for (const [type, props] of Object.entries(COMPONENT_PROPS)) {
			const known = KNOWN_PROPS[type];
			expect(known).toBeDefined();
			const missing = props.filter((prop) => !known?.has(prop));
			expect(`${type}: ${missing.join(",")}`).toBe(`${type}: `);
		}
	});

	test("accepts no props beyond the interface plus declared runtime-only extras", () => {
		for (const [type, known] of Object.entries(KNOWN_PROPS)) {
			const allowed = new Set<string>([
				...(COMPONENT_PROPS[type as keyof typeof COMPONENT_PROPS] ?? []),
				...(ALLOWED_RUNTIME_ONLY_PROPS[type] ?? []),
			]);
			const extras = [...known].filter((prop) => !allowed.has(prop));
			expect(`${type}: ${extras.join(",")}`).toBe(`${type}: `);
		}
	});

	test("documented interactive/planning components are not rejected", () => {
		for (const type of [
			"feedback",
			"appLink",
			"calendar",
			"gantt",
			"userProfile",
			"voiceInput",
			"geoMap",
			"tableRow",
			"tableCell",
		]) {
			expect(KNOWN_PROPS[type]).toBeDefined();
			expect(KNOWN_PROPS[type].size).toBeGreaterThan(0);
		}
	});

	test("nivoChart accepts its current chart props", () => {
		for (const prop of ["title", "indexBy", "keys", "axisTop", "axisRight"]) {
			expect(KNOWN_PROPS.nivoChart.has(prop)).toBe(true);
		}
	});

	test("dialogue uses the current prop names, not the stale ones", () => {
		expect(KNOWN_PROPS.dialogue.has("typewriterSpeed")).toBe(true);
		expect(KNOWN_PROPS.dialogue.has("speakerPortraitId")).toBe(true);
		expect(KNOWN_PROPS.dialogue.has("speed")).toBe(false);
		expect(KNOWN_PROPS.dialogue.has("portrait")).toBe(false);
	});
});

describe("BASE_PROPS drift protection", () => {
	test("matches ComponentBase (including hidden) plus the type discriminant", () => {
		expect([...BASE_PROPS].sort()).toEqual(
			["type", ...COMPONENT_BASE_PROPS].sort(),
		);
		expect(BASE_PROPS.has("hidden")).toBe(true);
		expect(BASE_PROPS.has("eventHandlers")).toBe(true);
	});
});

describe("named event handler validation", () => {
	test("preserves ordered named handlers, explicit empty lists, and legacy actions", () => {
		const legacyActions = [
			{ name: "workflow_event", context: { nodeId: "legacy-node" } },
			"legacy-extension-entry",
		];
		const result = validateComponents([
			{
				id: "calendar",
				component: {
					type: "calendar",
					events: { literalJson: "[]" },
					actions: legacyActions,
					eventHandlers: {
						open: [
							{
								name: "workflow_event",
								context: { nodeId: "open-node" },
							},
							{
								name: "navigate_page",
								context: { route: "/details" },
							},
						],
						delete: [],
					},
				},
			},
		] as unknown as SurfaceComponent[]);

		const component = result.components[0]?.component as unknown as Record<
			string,
			unknown
		>;
		expect(component.actions).toEqual(legacyActions);
		expect(component.eventHandlers).toEqual({
			open: [
				{
					name: "workflow_event",
					context: { nodeId: "open-node" },
				},
				{ name: "navigate_page", context: { route: "/details" } },
			],
			delete: [],
		});
		expect(result.warnings).toEqual([]);
	});

	test("removes malformed handler names, action arrays, and action entries", () => {
		const result = validateComponents([
			{
				id: "gantt",
				component: {
					type: "gantt",
					tasks: { literalJson: "[]" },
					eventHandlers: {
						"": [{ name: "workflow_event", context: {} }],
						open: { name: "workflow_event", context: {} },
						move: [
							null,
							{ name: "", context: {} },
							{ name: "workflow_event", context: "invalid" },
							{
								name: "workflow_event",
								context: { nodeId: "move-node" },
								ignored: true,
							},
						],
						resize: [{ name: "workflow_event" }],
					},
				},
			},
		] as unknown as SurfaceComponent[]);

		const component = result.components[0]?.component as unknown as Record<
			string,
			unknown
		>;
		expect(component.eventHandlers).toEqual({
			move: [
				{
					name: "workflow_event",
					context: { nodeId: "move-node" },
				},
			],
		});
		expect(result.warnings.length).toBeGreaterThanOrEqual(5);
	});
});

describe("AI component contract repair", () => {
	test("preserves large custom CSS without truncating a rule", () => {
		const customCss = `${".large{color:red}".repeat(800)}.final{display:grid}`;
		expect(customCss.length).toBeGreaterThan(12_000);

		const settings = validateCanvasSettings({ customCss });

		expect(settings?.customCss).toBe(customCss);
		expect(settings?.customCss).toEndWith(".final{display:grid}");
	});

	test("normalizes compatibility style fields before components reach persistence", () => {
		const input = [
			{
				id: "root",
				style: {
					background: {
						gradient: {
							gradientType: "linear",
							direction: "90deg",
							stops: [
								{ color: "red", position: 0 },
								{ color: "blue", position: 1 },
							],
						},
					},
					padding: { value: "4px 8px" },
				},
				component: { type: "column" },
			},
		] as unknown as SurfaceComponent[];

		const result = validateComponents(input);
		expect(result.components[0]?.style).toMatchObject({
			background: {
				gradient: {
					type: "linear",
					angle: 90,
					stops: [
						{ color: "red", position: 0 },
						{ color: "blue", position: 100 },
					],
				},
			},
			padding: { top: "4px", right: "8px", bottom: "4px", left: "8px" },
		});
	});

	test("injects safe structured defaults and skips missing reference props", () => {
		const result = validateComponents([
			{
				id: "tabs",
				component: { type: "tabs", value: { literalString: "first" } },
			},
			{
				id: "popover",
				component: { type: "popover" },
			},
		] as unknown as SurfaceComponent[]);

		expect(result.components).toHaveLength(1);
		expect(result.components[0]?.component).toMatchObject({
			type: "tabs",
			tabs: [],
		});
		expect(
			result.warnings.some((warning) => warning.includes("contentComponentId")),
		).toBe(true);
	});
});
