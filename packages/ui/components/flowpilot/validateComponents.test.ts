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
import type { FileEditorComponent, SurfaceComponent } from "../a2ui/types";
import {
	createDefaultComponent,
	normalizeComponent,
} from "../builder/componentDefaults";
import {
	BASE_PROPS,
	KNOWN_PROPS,
	MAX_CUSTOM_CSS_CHARS,
	validateCanvasSettings,
	validateComponents,
} from "./validateComponents";

/** The only props the validator may accept beyond the types.ts interfaces. */
const ALLOWED_RUNTIME_ONLY_PROPS: Record<string, readonly string[]> = {
	widgetInstance: ["inlineWidgetDef"],
};

describe("camera audio configuration", () => {
	test("new and legacy cameras keep microphone capture off by default", () => {
		for (const component of [
			createDefaultComponent("cameraView"),
			normalizeComponent({ id: "camera", type: "cameraView" }),
		]) {
			expect(component).toMatchObject({
				audioEnabled: { literalBool: false },
				audioBufferSeconds: { literalNumber: 60 },
			});
		}
	});

	test("builder normalization and generated UI validation preserve audio settings", () => {
		for (const settings of [
			{
				audioEnabled: { literalBool: true },
				audioBufferSeconds: { literalNumber: 300 },
			},
			{
				audioEnabled: { path: "/camera/enableAudio" },
				audioBufferSeconds: { path: "/camera/historySeconds" },
			},
		]) {
			const component = normalizeComponent({
				id: "camera",
				type: "cameraView",
				...settings,
			});
			expect(component).toMatchObject(settings);
			const result = validateComponents([{ id: "camera", component }]);
			expect(result.components[0]?.component).toMatchObject(settings);
		}
	});
});

describe("saved forecast components", () => {
	test("preserves saved references and display bindings through builder and generation", () => {
		const props = {
			forecastId: { path: "$.data.savedForecastId" },
			appId: { literalString: "owning-project" },
			userScoped: { literalBool: false },
			target: { literalString: "sales" },
			seriesId: { literalString: "region-a" },
			view: { literalString: "evaluation" },
			showCovariates: { literalBool: false },
			showMetrics: { literalBool: true },
			showControls: { literalBool: false },
			showHeader: { literalBool: true },
			compact: { literalBool: true },
			height: { literalNumber: 220 },
		};
		const component = normalizeComponent({
			id: "saved-outlook",
			type: "forecastChart",
			...props,
		});
		expect(component).toMatchObject({ type: "forecastChart", ...props });
		const validated = validateComponents([{ id: "saved-outlook", component }]);
		expect(validated.components[0]?.component).toMatchObject({
			type: "forecastChart",
			...props,
		});
		expect(validated.warnings).toEqual([]);
	});

	test("new components start without a fabricated saved forecast ID", () => {
		expect(createDefaultComponent("forecastChart")).toMatchObject({
			type: "forecastChart",
			forecastId: { literalString: "" },
			userScoped: { literalBool: false },
		});
	});
});

describe("Excalidraw generated components", () => {
	test("preserves scene bindings, autosave settings, and exact handlers through both validators", () => {
		const props = {
			value: { path: "$.data.drawing" },
			width: { literalString: "100%" },
			height: { literalString: "600px" },
			readOnly: { literalBool: false },
			theme: { literalString: "dark" },
			autoSave: { literalBool: true },
			autoSaveDelayMs: { literalNumber: 2000 },
			autoSaveMinChanges: { literalNumber: 3 },
			eventHandlers: {
				save: [{ name: "workflow_event", context: { nodeId: "save-drawing" } }],
				autoSave: [
					{ name: "workflow_event", context: { nodeId: "autosave-drawing" } },
				],
			},
		};
		const component = normalizeComponent({
			id: "drawing",
			type: "excalidraw",
			...props,
		});
		expect(component).toMatchObject({ type: "excalidraw", ...props });
		const validated = validateComponents([{ id: "drawing", component }]);
		expect(validated.components[0]?.component).toMatchObject({
			type: "excalidraw",
			...props,
		});
		expect(validated.warnings).toEqual([]);
	});
});

describe("File Editor generated components", () => {
	test("repairs a missing required editor with the builder default", () => {
		const result = validateComponents([
			{ id: "editor", component: { type: "fileEditor" } },
		] as SurfaceComponent[]);
		expect(result.components[0]?.component).toMatchObject({
			type: "fileEditor",
			editor: { literalString: "wordcraft" },
		});
		expect(result.warnings).toEqual([
			'editor: injected default for required prop "editor" on fileEditor',
		]);
	});

	test("preserves the selected editor, optional source, and exact Save workflow", () => {
		const component = {
			type: "fileEditor",
			editor: { literalString: "solidcraft" },
			url: { path: "$.file.url" },
			eventHandlers: {
				save: [{ name: "workflow_event", context: { nodeId: "save" } }],
			},
		} satisfies Omit<FileEditorComponent, "id">;
		const result = validateComponents([
			{ id: "editor", component: { id: "editor", ...component } },
		]);
		// The surface entry owns the id; the validated component body carries none.
		expect<unknown>(result.components[0]?.component).toEqual(component);
		expect(result.warnings).toEqual([]);
	});
});

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
			{ name: "legacy-extension-entry", context: {} },
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
		expect(result.warnings).toEqual([
			'calendar: actions[1].name "legacy-extension-entry" is not a built-in action and will not run. The name is a fixed verb, never a board node / event / widget action id — put that id in the context. Valid: workflow_event, widget_event, navigate_page, external_link, navigate_app_config, navigate_app_overview, submit_feedback.',
		]);
	});

	test("lowering normalizes legacy action shorthands before validation", () => {
		const result = validateComponents([
			{
				id: "button",
				type: "button",
				label: "Go",
				actions: [
					"external_link",
					{ context: { url: "https://x.test" } },
					{ name: "external_link", context: { url: "https://x.test" } },
				],
			},
		]);
		const component = result.components[0]?.component as unknown as Record<
			string,
			unknown
		>;
		expect(component.actions).toEqual([
			{ name: "external_link", context: {} },
			{ name: "external_link", context: { url: "https://x.test" } },
		]);
	});

	test("removes malformed handler names and entries; lowering wraps single actions", () => {
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
			open: [{ name: "workflow_event", context: {} }],
			move: [
				{
					name: "workflow_event",
					context: { nodeId: "move-node" },
				},
			],
			resize: [{ name: "workflow_event", context: {} }],
		});
		expect(result.warnings.length).toBeGreaterThanOrEqual(5);
	});
});

describe("action routability", () => {
	const buttonWith = (component: Record<string, unknown>) =>
		validateComponents([
			{
				id: "approve-btn",
				component: {
					type: "button",
					label: { literalString: "Approve" },
					...component,
				},
			},
		] as unknown as SurfaceComponent[]);

	test("flags an action named after a board node or widget action id", () => {
		// ActionHandler.tsx dispatches a fixed set of verbs; anything else falls through to a
		// no-op userAction, so the control renders and silently does nothing.
		const result = buttonWith({
			actions: [{ name: "approve_request", context: {} }],
			eventHandlers: {
				click: [{ name: "custom:refresh", context: {} }],
			},
		});

		expect(
			result.warnings.some(
				(warning) =>
					warning.includes('actions[0].name "approve_request"') &&
					warning.includes("not a built-in action"),
			),
		).toBe(true);
		expect(
			result.warnings.some((warning) =>
				warning.includes('eventHandlers.click[0].name "custom:refresh"'),
			),
		).toBe(true);
	});

	test("keeps the unroutable action so the inspector can still show it", () => {
		const actions = [{ name: "approve_request", context: {} }];
		const result = buttonWith({ actions });

		const component = result.components[0]?.component as unknown as Record<
			string,
			unknown
		>;
		expect(component.actions).toEqual(actions);
	});

	test("flags routing actions that carry no target id", () => {
		const result = buttonWith({
			actions: [{ name: "workflow_event", context: {} }],
			eventHandlers: {
				click: [{ name: "widget_event", context: { actionId: "  " } }],
			},
		});

		expect(
			result.warnings.some(
				(warning) =>
					warning.includes("actions[0]") && warning.includes("context.nodeId"),
			),
		).toBe(true);
		expect(
			result.warnings.some(
				(warning) =>
					warning.includes("eventHandlers.click[0]") &&
					warning.includes("context.actionId"),
			),
		).toBe(true);
	});

	test("accepts the documented workflow and widget action contracts", () => {
		const result = buttonWith({
			actions: [{ name: "widget_event", context: { actionId: "approve" } }],
			eventHandlers: {
				click: [{ name: "workflow_event", context: { nodeId: "evt-approve" } }],
			},
		});

		expect(result.warnings).toEqual([]);
	});
});

describe("AI component contract repair", () => {
	test("preserves large custom CSS without truncating a rule", () => {
		const customCss = `${".large{color:red}".repeat(800)}.final{display:grid}`;
		expect(customCss.length).toBeGreaterThan(12_000);
		expect(customCss.length).toBeLessThan(MAX_CUSTOM_CSS_CHARS);

		const settings = validateCanvasSettings({ customCss });

		expect(settings?.customCss).toBe(customCss);
		expect(settings?.customCss).toEndWith(".final{display:grid}");
	});

	test("drops custom CSS past the limit whole instead of cutting it", () => {
		const customCss = ".x{color:red}".repeat(MAX_CUSTOM_CSS_CHARS);
		const warnings: string[] = [];

		const settings = validateCanvasSettings(
			{ padding: "1rem", customCss },
			warnings,
		);

		expect(settings?.customCss).toBeUndefined();
		expect(settings?.padding).toBe("1rem");
		expect(warnings).toHaveLength(1);
		expect(warnings[0]).toContain(`${MAX_CUSTOM_CSS_CHARS}`);
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

describe("plain-value authoring form", () => {
	test("flat components with plain values lower to the canonical shape", () => {
		const result = validateComponents([
			{ id: "root", type: "column", gap: "12px", children: ["title", "pick"] },
			{
				id: "title",
				type: "text",
				content: "Recipe Remix",
				variant: "heading",
				style: { className: "mb-2" },
			},
			{
				id: "pick",
				type: "select",
				value: "s",
				options: ["s", { value: "m", label: "Medium" }],
				action: { name: "external_link", context: { url: "https://x.test" } },
			},
		]);
		expect(result.warnings).toEqual([]);
		expect(result.components).toEqual([
			{
				id: "root",
				component: {
					type: "column",
					gap: { literalString: "12px" },
					children: { explicitList: ["title", "pick"] },
				},
			},
			{
				id: "title",
				style: { className: "mb-2" },
				component: {
					type: "text",
					content: { literalString: "Recipe Remix" },
					variant: { literalString: "heading" },
				},
			},
			{
				id: "pick",
				component: {
					type: "select",
					value: { literalString: "s" },
					options: {
						literalOptions: [
							{ value: "s", label: "s" },
							{ value: "m", label: "Medium" },
						],
					},
					actions: [
						{ name: "external_link", context: { url: "https://x.test" } },
					],
				},
			},
		] as unknown as SurfaceComponent[]);
	});

	test("bare children arrays in the canonical form are kept as explicit lists", () => {
		const result = validateComponents([
			{ id: "root", component: { type: "row", children: ["a", "gone"] } },
			{ id: "a", component: { type: "divider" } },
		]);
		expect(result.components[0]?.component.children).toEqual({
			explicitList: ["a"],
		});
	});

	test("manifest kinds decide coercion: structured bound props wrap, plain props unwrap", () => {
		const result = validateComponents([
			{
				id: "table",
				type: "table",
				columns: [{ id: "n", header: "Name", accessor: "name" }],
				data: [{ name: "A" }],
			},
			{
				id: "link",
				type: "link",
				href: "https://x.test",
				target: { literalString: "_blank" },
				underline: "none",
			},
			{
				id: "tabs",
				type: "tabs",
				value: "a",
				tabs: [{ id: "a", label: "First", contentComponentId: "table" }],
			},
		]);
		const [table, link, tabs] = result.components.map(
			(entry) => entry.component as unknown as Record<string, unknown>,
		);
		expect(table?.columns).toEqual({
			literalJson: JSON.stringify([
				{ id: "n", header: "Name", accessor: "name" },
			]),
		});
		expect(table?.data).toEqual({ literalJson: '[{"name":"A"}]' });
		expect(link?.target).toBe("_blank");
		expect(link?.underline).toBe("none");
		expect(tabs?.tabs).toEqual([
			{
				id: "a",
				label: { literalString: "First" },
				contentComponentId: "table",
			},
		]);
	});

	test("components that cannot be lowered are skipped with a warning", () => {
		const result = validateComponents([
			{ id: "untyped", content: "no type" },
			{ id: "bad", component: "text" },
			{ id: "ok", type: "divider" },
		]);
		expect(result.components.map((entry) => entry.id)).toEqual(["ok"]);
		expect(result.warnings).toEqual([
			"untyped: missing component `type`",
			"bad: `component` must be an object with a `type`",
		]);
	});
});
