import { describe, expect, test } from "bun:test";
import {
	type LivePageHandle,
	isLivePageComponentEffectivelyHidden,
	isLivePageValueBearingComponent,
	registerLivePage,
	resolveLivePageComponentId,
} from "../components/a2ui/live-page-registry";
import type { A2UIComponent, Surface } from "../components/a2ui/types";
import { inspectLiveAppPage, interactWithAppPage } from "./interact-app-page";

const workflowAction = { name: "run", context: {} };

function resolveLiteral(value: unknown): unknown {
	if (!value || typeof value !== "object") return value;
	const bound = value as Record<string, unknown>;
	if ("literalString" in bound) return bound.literalString;
	if ("literalNumber" in bound) return bound.literalNumber;
	if ("literalBool" in bound) return bound.literalBool;
	if ("literalOptions" in bound) return bound.literalOptions;
	if ("path" in bound) {
		return bound.path === "form.disabled" ? true : bound.defaultValue;
	}
	return value;
}

function liveHandle(
	surface: Surface,
	overrides: Partial<LivePageHandle> = {},
): LivePageHandle {
	return {
		appId: "app",
		pageId: "page",
		eventId: "page-event",
		getSurface: () => surface,
		getElementValues: () => ({}),
		resolveBoundValue: resolveLiteral,
		setElementValue: () => {},
		triggerComponentEvent: async () => ({
			triggered: false,
			source: "none",
			actionCount: 0,
			runs: [],
		}),
		isLoading: () => false,
		...overrides,
	};
}

describe("inspectLiveAppPage", () => {
	test("returns resolved, bounded semantics and redacts password values", () => {
		const options = Array.from({ length: 30 }, (_, index) => ({
			value: `value-${index}`,
			label: `Option ${index}`,
		}));
		const surface: Surface = {
			id: "surface",
			rootComponentId: "root",
			components: {
				root: {
					id: "root",
					component: {
						id: "root",
						type: "column",
						children: {
							explicitList: ["intro", "email", "password", "choice", "submit"],
						},
					},
				},
				intro: {
					id: "intro",
					component: {
						id: "intro",
						type: "text",
						content: { literalString: "Create an order" },
					},
				},
				email: {
					id: "email",
					component: {
						id: "email",
						type: "textField",
						value: { path: "form.email", defaultValue: "default@example.com" },
						label: { literalString: "Email" },
						placeholder: { literalString: "name@example.com" },
						disabled: { path: "form.disabled" },
						eventHandlers: { submit: [workflowAction] },
					},
				},
				password: {
					id: "password",
					component: {
						id: "password",
						type: "textField",
						value: { literalString: "surface-secret" },
						inputType: { literalString: "Password" },
						label: { literalString: "Password" },
					},
				},
				choice: {
					id: "choice",
					component: {
						id: "choice",
						type: "select",
						value: { literalString: "value-1" },
						options: { literalOptions: options },
					},
				},
				submit: {
					id: "submit",
					component: {
						id: "submit",
						type: "button",
						label: { literalString: "Submit" },
						actions: [workflowAction],
					},
				},
			},
		};
		const handle = liveHandle(surface, {
			getElementValues: () => ({
				"surface/email": "stored@example.com",
				"surface/password": "stored-secret",
			}),
		});

		const inspection = inspectLiveAppPage(handle);
		expect(inspection).toMatchObject({
			page_id: "page",
			event_id: "page-event",
			root_component_id: "root",
			element_count: 6,
			elements_truncated: false,
		});
		expect(inspection.elements.map((element) => element.component_id)).toEqual([
			"root",
			"intro",
			"email",
			"password",
			"choice",
			"submit",
		]);
		expect(inspection.elements[0].child_ids).toEqual([
			"intro",
			"email",
			"password",
			"choice",
			"submit",
		]);
		expect(inspection.elements[1]).toMatchObject({
			element_ref: "page/intro",
			parent_id: "root",
			text: "Create an order",
		});
		expect(inspection.elements[2]).toMatchObject({
			label: "Email",
			placeholder: "name@example.com",
			disabled: true,
			current_value: "stored@example.com",
			configured_events: ["submit"],
		});
		const password = inspection.elements[3];
		expect(password).toMatchObject({
			sensitive: true,
			value_redacted: true,
		});
		expect(password).not.toHaveProperty("current_value");
		expect(JSON.stringify(password)).not.toContain("stored-secret");
		expect(JSON.stringify(password)).not.toContain("surface-secret");
		expect(inspection.elements[4].options).toHaveLength(25);
		expect(inspection.elements[4].options_truncated).toBe(true);
		expect(inspection.elements[5].configured_events).toContain("click");
	});

	test("caps the semantic inventory", () => {
		const components: Surface["components"] = {};
		const childIds: string[] = [];
		for (let index = 0; index < 160; index += 1) {
			const id = `text-${index}`;
			childIds.push(id);
			components[id] = {
				id,
				component: {
					id,
					type: "text",
					content: { literalString: `Text ${index}` },
				},
			};
		}
		components.root = {
			id: "root",
			component: {
				id: "root",
				type: "column",
				children: { explicitList: childIds },
			},
		};
		const inspection = inspectLiveAppPage(
			liveHandle({
				id: "surface",
				rootComponentId: "root",
				components,
			}),
		);
		expect(inspection.element_count).toBe(161);
		expect(inspection.elements).toHaveLength(150);
		expect(inspection.elements_truncated).toBe(true);
	});
});

describe("live page component targeting", () => {
	const component = (id: string, component: A2UIComponent) => ({
		id,
		component,
	});
	const surface: Surface = {
		id: "surface",
		rootComponentId: "field",
		components: {
			field: component("field", {
				id: "field",
				type: "textField",
				value: { literalString: "" },
			}),
			"group/field": component("group/field", {
				id: "group/field",
				type: "textField",
				value: { literalString: "" },
			}),
		},
	};

	test("accepts bare ids and page-scoped refs, retargeting another page's refs", () => {
		expect(resolveLivePageComponentId("page", surface, "field")).toBe("field");
		expect(resolveLivePageComponentId("page", surface, "page/field")).toBe(
			"field",
		);
		expect(resolveLivePageComponentId("page", surface, "group/field")).toBe(
			"group/field",
		);
		expect(resolveLivePageComponentId("page", surface, "other/field")).toBe(
			"field",
		);
		expect(() =>
			resolveLivePageComponentId("page", surface, "other/missing"),
		).toThrow("different page");
	});

	test("widget-host refs are never retargeted to page components", () => {
		const widgetSurface: Surface = {
			...surface,
			components: {
				...surface.components,
				host: component("host", {
					id: "host",
					type: "widgetInstance",
				} as unknown as A2UIComponent),
			},
		};
		expect(() =>
			resolveLivePageComponentId("page", widgetSurface, "host/field"),
		).toThrow("different page");
	});

	test("only scalar input controls are set_value targets", () => {
		expect(
			isLivePageValueBearingComponent(surface.components.field.component),
		).toBe(true);
		expect(
			isLivePageValueBearingComponent({
				id: "button",
				type: "button",
				label: { literalString: "Run" },
			}),
		).toBe(false);
	});

	test("treats descendants of hidden containers as unreachable", () => {
		const hiddenSurface: Surface = {
			id: "hidden-surface",
			rootComponentId: "group",
			components: {
				group: component("group", {
					id: "group",
					type: "column",
					hidden: { literalBool: true },
					children: { explicitList: ["field"] },
				}),
				field: component("field", {
					id: "field",
					type: "textField",
					value: { literalString: "" },
				}),
			},
		};

		expect(
			isLivePageComponentEffectivelyHidden(
				hiddenSurface,
				"field",
				resolveLiteral,
			),
		).toBe(true);
		const field = inspectLiveAppPage(liveHandle(hiddenSurface)).elements.find(
			(element) => element.component_id === "field",
		);
		expect(field?.hidden).toBe(true);
	});
});

describe("interactWithAppPage", () => {
	test("validates targets and reports failed workflow runs as partial", async () => {
		const surface: Surface = {
			id: "surface",
			rootComponentId: "root",
			components: {
				root: {
					id: "root",
					component: { id: "root", type: "column" },
				},
				field: {
					id: "field",
					component: {
						id: "field",
						type: "textField",
						value: { literalString: "" },
					},
				},
				display: {
					id: "display",
					component: {
						id: "display",
						type: "text",
						content: { literalString: "Read only" },
					},
				},
				disabled: {
					id: "disabled",
					component: {
						id: "disabled",
						type: "button",
						label: { literalString: "Disabled" },
						disabled: { literalBool: true },
						actions: [workflowAction],
					},
				},
				hidden: {
					id: "hidden",
					component: {
						id: "hidden",
						type: "button",
						label: { literalString: "Hidden" },
						hidden: { literalBool: true },
						actions: [workflowAction],
					},
				},
				readOnly: {
					id: "readOnly",
					component: {
						id: "readOnly",
						type: "richText",
						value: { literalString: "plate_json::[]" },
						readOnly: { literalBool: true },
					},
				},
				slider: {
					id: "slider",
					component: {
						id: "slider",
						type: "slider",
						value: { literalNumber: 50 },
					},
				},
				submit: {
					id: "submit",
					component: {
						id: "submit",
						type: "button",
						label: { literalString: "Submit" },
						actions: [workflowAction],
					},
				},
			},
		};
		const writes: Array<[string, unknown]> = [];
		const triggers: Array<[string, string]> = [];
		let unregister = () => {};
		const handle = liveHandle(surface, {
			setElementValue: (componentId, value) => {
				writes.push([componentId, value]);
			},
			triggerComponentEvent: async (componentId, event) => {
				triggers.push([componentId, event]);
				unregister();
				return {
					triggered: true,
					source: "legacy",
					actionCount: 1,
					runs: [
						{
							status: "not_executed",
							componentId,
							endedAtMs: Date.now(),
						},
					],
				};
			},
		});
		unregister = registerLivePage(handle);
		try {
			const result = await interactWithAppPage(
				{} as Parameters<typeof interactWithAppPage>[0],
				{
					appId: "app",
					eventId: "page-event",
					captureScreenshots: false,
					actions: [
						{
							action: "set_value",
							component_id: "page/field",
							value: "new value",
							hasValue: true,
						},
						{
							action: "set_value",
							component_id: "page/display",
							value: "overwrite",
							hasValue: true,
						},
						{
							action: "set_value",
							component_id: "page/readOnly",
							value: "plate_json::[]",
							hasValue: true,
						},
						{
							action: "set_value",
							component_id: "page/slider",
							value: "not a number",
							hasValue: true,
						},
						{
							action: "trigger",
							component_id: "page/disabled",
							event: "click",
						},
						{
							action: "trigger",
							component_id: "page/hidden",
							event: "click",
						},
						{
							action: "trigger",
							component_id: "page/submit",
							event: "click",
						},
						{
							action: "set_value",
							component_id: "page/field",
							value: "must not be written",
							hasValue: true,
						},
					],
				},
			);

			expect(result.status).toBe("partial");
			expect(result.failed_run_count).toBe(1);
			expect(result.page_changed).toBe(true);
			expect(writes).toEqual([["field", "new value"]]);
			expect(triggers).toEqual([["submit", "click"]]);
			const applied = result.applied_actions as Array<Record<string, unknown>>;
			expect(applied[1].detail).toContain("does not accept set_value");
			expect(applied[2].detail).toContain("read-only");
			expect(applied[3].detail).toContain("finite number");
			expect(applied[4].detail).toContain("disabled");
			expect(applied[5].detail).toContain("hidden");
			expect(applied[7].detail).toContain("replaced");
		} finally {
			unregister();
		}
	});

	test("reports navigation by the final action without inspecting the detached page", async () => {
		const surface: Surface = {
			id: "surface",
			rootComponentId: "next",
			components: {
				next: {
					id: "next",
					component: {
						id: "next",
						type: "button",
						label: { literalString: "Next" },
						actions: [workflowAction],
					},
				},
			},
		};
		let unregisterSelected = () => {};
		let unregisterReplacement = () => {};
		const selected = liveHandle(surface, {
			triggerComponentEvent: async () => {
				unregisterSelected();
				unregisterReplacement = registerLivePage({
					...liveHandle(surface),
					pageId: "destination-page",
					eventId: "destination-event",
				});
				return {
					triggered: true,
					source: "legacy",
					actionCount: 1,
					runs: [],
				};
			},
		});
		unregisterSelected = registerLivePage(selected);

		try {
			const result = await interactWithAppPage(
				{} as Parameters<typeof interactWithAppPage>[0],
				{
					appId: "app",
					eventId: "page-event",
					captureScreenshots: false,
					actions: [
						{
							action: "trigger",
							component_id: "next",
							event: "click",
						},
					],
				},
			);

			expect(result.status).toBe("partial");
			expect(result.page_changed).toBe(true);
			expect(result.element_count).toBe(0);
			expect(result.elements).toEqual([]);
		} finally {
			unregisterSelected();
			unregisterReplacement();
		}
	});
});
