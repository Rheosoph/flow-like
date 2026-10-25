import { describe, expect, test } from "bun:test";
import { applyElementUpdate } from "./apply-a2ui-message";
import {
	type FlowValueBaseline,
	collectFlowValueWrites,
	hiddenRestoredInputs,
} from "./flow-value-sync";
import type { SurfaceComponent } from "./types";
import { mergeStoredElementValues } from "./workflow-elements";

const SCOPE = JSON.stringify(["app", "page"]);

type Components = Record<string, SurfaceComponent>;

function input(
	id: string,
	data: Record<string, unknown>,
	type = "textField",
): SurfaceComponent {
	return { id, component: { type, ...data } } as unknown as SurfaceComponent;
}

function text(id: string, value: string): SurfaceComponent {
	return input(id, { value: { literalString: value } });
}

function setValue(
	components: Components,
	id: string,
	value: unknown,
): Components {
	return {
		...components,
		[id]: applyElementUpdate(components[id], { type: "setValue", value }),
	};
}

function widget(instanceId: string, children: SurfaceComponent[]): Components {
	return {
		host: input(
			"host",
			{ instanceId, inlineWidgetDef: { components: children } },
			"widgetInstance",
		),
	};
}

/** Runs the sync over successive surfaces and applies its writes to `stored`. */
function syncer(stored: Record<string, unknown>, surfaceId = "page") {
	let baseline: FlowValueBaseline | null = null;
	return (components: Components, scope = SCOPE) => {
		const result = collectFlowValueWrites(
			baseline,
			scope,
			components,
			surfaceId,
		);
		baseline = result.baseline;
		for (const [elementId, value] of result.writes) stored[elementId] = value;
		return result.writes;
	};
}

function sentValue(
	stored: Record<string, unknown>,
	components: Components,
	elementId: string,
	surfaceId = "page",
): unknown {
	const merged = mergeStoredElementValues({}, stored, components, surfaceId);
	return (merged[elementId] as { component: { value: unknown } }).component
		.value;
}

describe("flow writes to inputs reach the run payload", () => {
	test("a flow estimate replaces the values loaded and edited before it", () => {
		const stored: Record<string, unknown> = {
			"page/kcal": "210",
			"page/meal-id": "meal-1",
		};
		const sync = syncer(stored);
		let surface: Components = {
			kcal: text("kcal", "210"),
			"meal-id": text("meal-id", "meal-1"),
		};
		expect(sync(surface)).toEqual([]);

		surface = setValue(surface, "kcal", "405");
		surface = setValue(surface, "meal-id", "");
		expect(sync(surface)).toEqual([
			["page/kcal", "405"],
			["page/meal-id", ""],
		]);
		expect(sentValue(stored, surface, "page/kcal")).toEqual({
			literalString: "405",
		});
		expect(sentValue(stored, surface, "page/meal-id")).toEqual({
			literalString: "",
		});
	});

	test("a user edit after the flow write survives later surface updates", () => {
		const stored: Record<string, unknown> = {};
		const sync = syncer(stored);
		let surface: Components = { kcal: text("kcal", "210") };
		sync(surface);
		surface = setValue(surface, "kcal", "405");
		sync(surface);

		stored["page/kcal"] = "410";
		surface = {
			...surface,
			kcal: applyElementUpdate(surface.kcal, {
				type: "setPlaceholder",
				placeholder: "kcal",
			}),
			other: text("other", "x"),
		};
		expect(sync(surface)).toEqual([]);
		expect(sync({ ...surface })).toEqual([]);
		expect(stored["page/kcal"]).toBe("410");
	});

	test("rewriting the value the input already declared still resets the edit", () => {
		const stored: Record<string, unknown> = {};
		const sync = syncer(stored);
		let surface: Components = { composer: text("composer", "") };
		sync(surface);

		stored["page/composer"] = "draft message";
		surface = setValue(surface, "composer", "");
		expect(sync(surface)).toEqual([["page/composer", ""]]);
	});

	test("a component replaced with a new literal resets the edit", () => {
		const stored: Record<string, unknown> = { "page/kcal": "410" };
		const sync = syncer(stored);
		sync({ kcal: text("kcal", "210") });

		expect(sync({ kcal: text("kcal", "210") })).toEqual([]);
		expect(sync({ kcal: text("kcal", "500") })).toEqual([["page/kcal", "500"]]);
	});

	test("typed setters keep the shape a user edit stores", () => {
		const stored: Record<string, unknown> = {};
		const sync = syncer(stored);
		let surface: Components = {
			done: input("done", { checked: false }, "checkbox"),
			amount: input("amount", { value: { literalNumber: 1 } }, "slider"),
			tags: input("tags", { value: { literalJson: "[]" } }, "select"),
		};
		sync(surface);

		surface = {
			...surface,
			done: applyElementUpdate(surface.done, {
				type: "setChecked",
				checked: true,
			}),
		};
		surface = setValue(surface, "amount", 7);
		surface = setValue(surface, "tags", { literalJson: '["a","b"]' });
		expect(sync(surface)).toEqual([
			["page/done", true],
			["page/amount", 7],
			["page/tags", ["a", "b"]],
		]);
	});

	test("the first pass over a scope only records a baseline", () => {
		const stored: Record<string, unknown> = { "page/kcal": "410" };
		const sync = syncer(stored);
		const written = setValue({ kcal: text("kcal", "210") }, "kcal", "405");

		expect(sync(written)).toEqual([]);
		expect(sync(written, JSON.stringify(["app", "other-page"]))).toEqual([]);
		expect(stored["page/kcal"]).toBe("410");
	});

	test("an input a flow creates already written is synced", () => {
		const stored: Record<string, unknown> = {};
		const sync = syncer(stored);
		sync({});

		expect(sync({ kcal: text("kcal", "210") })).toEqual([]);
		const created = setValue({ fresh: text("fresh", "") }, "fresh", "405");
		expect(sync({ kcal: text("kcal", "210"), ...created })).toEqual([
			["page/fresh", "405"],
		]);
	});

	test("path bindings and display components are left alone", () => {
		const stored: Record<string, unknown> = {};
		const sync = syncer(stored);
		let surface: Components = {
			bound: input("bound", { value: { path: "/meal/kcal" } }),
			progress: input("progress", { value: { literalNumber: 0 } }, "progress"),
		};
		sync(surface);

		surface = setValue(surface, "bound", { path: "/meal/estimate" });
		surface = setValue(surface, "progress", 100);
		expect(sync(surface)).toEqual([]);
	});

	test("widget children sync under their instance key only", () => {
		const stored: Record<string, unknown> = { "instance/kcal": "210" };
		const sync = syncer(stored);
		sync(widget("instance", [text("kcal", "210")]));

		const written = widget("instance", [
			applyElementUpdate(text("kcal", "210"), {
				type: "setValue",
				value: "405",
			}),
		]);
		expect(sync(written)).toEqual([["instance/kcal", "405"]]);

		stored["instance/kcal"] = "410";
		expect(sentValue(stored, written, "page/kcal")).toEqual({
			literalString: "410",
		});
		expect(sentValue(stored, written, "instance/kcal")).toEqual({
			literalString: "410",
		});
	});
});

describe("restored input values the screen does not show", () => {
	const hidden = (values: Record<string, unknown>, components: Components) =>
		hiddenRestoredInputs(
			values,
			new Set(Object.keys(values)),
			components,
			"page",
		);

	test("a stale draft under an empty field is hidden, a shown value is not", () => {
		expect(
			hidden(
				{
					"page/description": "Hähnchen",
					"page/kcal": "405",
					"page/amount": 7,
					"page/tags": ["a"],
				},
				{
					description: text("description", ""),
					kcal: text("kcal", "405"),
					amount: input("amount", { value: { literalNumber: 7 } }, "slider"),
					tags: input("tags", { value: { literalJson: '["a"]' } }, "select"),
				},
			),
		).toEqual(["page/description"]);
	});

	test("values edited on this mount are not judged", () => {
		expect(
			hiddenRestoredInputs(
				{ "page/description": "typed now" },
				new Set(),
				{ description: text("description", "") },
				"page",
			),
		).toEqual([]);
	});

	test("path bindings and undeclared values never show a restored value", () => {
		expect(
			hidden(
				{ "page/bound": "old", "page/blank": "old", "page/done": true },
				{
					bound: input("bound", { value: { path: "/meal/description" } }),
					blank: input("blank", {}),
					done: input("done", { checked: false }, "checkbox"),
				},
			),
		).toEqual(["page/bound", "page/blank", "page/done"]);
	});

	test("display components, micro widget values and unmounted ids keep theirs", () => {
		expect(
			hidden(
				{
					"page/progress": 40,
					"micro/values": { draft: "kept" },
					"page/later": "x",
				},
				{
					progress: input(
						"progress",
						{ value: { literalNumber: 0 } },
						"progress",
					),
					micro: input("micro", { instanceId: "micro" }, "microWidgetInstance"),
				},
			),
		).toEqual([]);
	});

	test("widget children are judged under their instance key", () => {
		expect(
			hidden(
				{ "instance/kcal": "210" },
				widget("instance", [text("kcal", "")]),
			),
		).toEqual(["instance/kcal"]);
	});
});
