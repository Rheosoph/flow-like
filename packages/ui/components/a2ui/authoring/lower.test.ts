import { describe, expect, test } from "bun:test";
import {
	lowerAuthoredComponent,
	lowerAuthoredComponents,
	lowerBound,
	lowerChildren,
} from "./lower";

function lowered(value: unknown) {
	const outcome = lowerAuthoredComponent(value);
	if (!outcome.ok) throw new Error(outcome.error);
	return outcome.component;
}

describe("lowerAuthoredComponent (mirrors authoring.rs)", () => {
	test("flat plain component lowers to the canonical shape", () => {
		expect(
			lowered({
				id: "title",
				type: "text",
				content: "Recipe Remix",
				size: "xl",
				maxLines: 2,
				truncate: true,
				style: { className: "mb-2" },
			}),
		).toEqual({
			id: "title",
			style: { className: "mb-2" },
			component: {
				type: "text",
				content: { literalString: "Recipe Remix" },
				size: { literalString: "xl" },
				maxLines: { literalNumber: 2 },
				truncate: { literalBool: true },
			},
		});
	});

	test("canonical components pass through unchanged", () => {
		const canonical = {
			id: "root",
			component: {
				type: "column",
				gap: { literalString: "8px" },
				children: { explicitList: ["a", "b"] },
			},
		};
		expect(lowered(canonical)).toEqual(canonical);
	});

	test("children, bindings, actions and options lower", () => {
		const { component } = lowered({
			id: "pick",
			type: "select",
			value: { path: "$.choice", defaultValue: "a" },
			options: ["Small", { value: 2, label: "Two" }],
			hidden: false,
			children: ["x"],
			action: { name: "send_message", context: { message: "go" } },
		});
		expect(component.value).toEqual({ path: "$.choice", defaultValue: "a" });
		expect(component.options).toEqual({
			literalOptions: [
				{ value: "Small", label: "Small" },
				{ value: "2", label: "Two" },
			],
		});
		expect(component.hidden).toEqual({ literalBool: false });
		expect(component.children).toEqual({ explicitList: ["x"] });
		expect(component.actions).toEqual([
			{ name: "send_message", context: { message: "go" } },
		]);
		expect(component).not.toHaveProperty("action");
	});

	test("structured props keep plain fields and wrap nested bound values", () => {
		const { component } = lowered({
			id: "o",
			type: "overlay",
			baseComponentId: { literalString: "base" },
			overlays: [{ componentId: "badge", anchor: "topRight" }],
		});
		expect(component.baseComponentId).toBe("base");
		expect(component.overlays).toEqual([
			{ componentId: "badge", anchor: { literalString: "topRight" } },
		]);
	});

	test("JSON-valued bound props become literalJson", () => {
		const { component } = lowered({
			id: "t",
			type: "table",
			data: [{ name: "A", value: 1 }],
		});
		const data = component.data as { literalJson: string };
		expect(JSON.parse(data.literalJson)).toEqual([{ name: "A", value: 1 }]);
	});

	test("invalid components report errors without dropping the rest", () => {
		const { components, errors } = lowerAuthoredComponents([
			{ id: "ok", type: "divider" },
			{ id: "bad", content: "no type" },
			"not an object",
		]);
		expect(components).toHaveLength(1);
		expect(errors).toHaveLength(2);
		expect(errors[0]?.id).toBe("bad");
		expect(errors[1]?.index).toBe(2);
	});

	test("the chat prompt example lowers cleanly", () => {
		const { components, errors } = lowerAuthoredComponents([
			{ id: "root", type: "column", gap: "12px", children: ["title", "cards"] },
			{
				id: "title",
				type: "text",
				content: "Recipe Remix",
				variant: "heading",
				size: "xl",
			},
			{ id: "cards", type: "grid", columns: 2, gap: "12px", children: ["c1"] },
			{
				id: "c1",
				type: "card",
				title: "Bean & Rice Skillet",
				description: "25 min · serves 2",
				children: ["c1-go"],
			},
			{
				id: "c1-go",
				type: "button",
				label: "Remix this",
				action: {
					name: "send_message",
					context: { message: "Remix the bean skillet" },
				},
			},
		]);
		expect(errors).toEqual([]);
		expect(components.map((c) => c.component.type)).toEqual([
			"column",
			"text",
			"grid",
			"card",
			"button",
		]);
		expect(components[2]?.component.columns).toEqual({ literalNumber: 2 });
	});
});

describe("lowering details", () => {
	test("rejects non-objects, non-object `component` and a missing type", () => {
		expect(lowerAuthoredComponent([1])).toEqual({
			ok: false,
			error: "a component must be a JSON object",
		});
		expect(lowerAuthoredComponent({ id: "x", component: null })).toEqual({
			ok: false,
			error: "`component` must be an object with a `type`",
		});
		expect(lowerAuthoredComponent({ id: "x", content: "hi" })).toEqual({
			ok: false,
			error: "missing component `type`",
		});
	});

	test("nested form keeps unknown top-level keys and lowers inner props", () => {
		expect(
			lowered({
				id: "t",
				eventRelevant: true,
				extra: 1,
				component: { type: "text", content: "Hi" },
			}),
		).toEqual({
			id: "t",
			eventRelevant: true,
			extra: 1,
			component: { type: "text", content: { literalString: "Hi" } },
		});
	});

	test("flat form moves only id, style and eventRelevant to the surface entry", () => {
		const entry = lowered({
			id: "t",
			type: "text",
			content: "Hi",
			eventRelevant: true,
			unknownProp: "kept for the validator",
		});
		expect(entry.eventRelevant).toBe(true);
		expect(entry.component.unknownProp).toBe("kept for the validator");
	});

	test("null bound props are removed, null plain props are kept", () => {
		const { component } = lowered({
			id: "l",
			type: "link",
			href: "https://example.com",
			label: null,
			target: null,
		});
		expect(component).not.toHaveProperty("label");
		expect(component.target).toBeNull();
	});

	test("plain props unwrap single literals but keep literalJson/literalOptions", () => {
		const { component } = lowered({
			id: "l",
			type: "link",
			href: "https://example.com",
			target: { literalString: "_blank" },
			variant: { literalJson: '"muted"' },
			external: { literalBool: true },
		});
		expect(component.target).toBe("_blank");
		expect(component.variant).toEqual({ literalJson: '"muted"' });
		expect(component.external).toBe(true);
	});

	test("action merges into the normalized actions", () => {
		const first = { name: "send_message", context: { message: "a" } };
		const second = { name: "send_message", context: { message: "b" } };
		expect(
			lowered({ id: "b", type: "button", actions: [first], action: second })
				.component.actions,
		).toEqual([first, second]);
		expect(
			lowered({ id: "b", type: "button", action: [first, second] }).component
				.actions,
		).toEqual([first, second]);
		expect(
			lowered({ id: "b", type: "button", actions: null, action: first })
				.component.actions,
		).toEqual([first]);
		expect(
			lowered({ id: "b", type: "button", action: 42 }).component.actions,
		).toEqual([]);
	});

	test("action names, numeric child ids and handler lists normalize", () => {
		const { component } = lowered({
			id: "go",
			type: "button",
			label: "Go",
			action: "send_message",
			eventHandlers: {
				click: { name: "external_link", context: { url: "https://a.b" } },
			},
			children: [1, "two", null],
		});
		expect(component.actions).toEqual([{ name: "send_message", context: {} }]);
		expect(component.eventHandlers).toEqual({
			click: [{ name: "external_link", context: { url: "https://a.b" } }],
		});
		expect(component.children).toEqual({ explicitList: ["1", "two"] });
	});

	test("actions without a string name drop; an existing context is kept as written", () => {
		const { component } = lowered({
			id: "b",
			type: "button",
			label: "Go",
			actions: [
				{ context: { message: "no name" } },
				"   ",
				{ name: 7 },
				{ name: "", context: null },
				{ name: "send_message" },
			],
			eventHandlers: "not a map",
		});
		expect(component.actions).toEqual([
			{ name: "", context: null },
			{ name: "send_message", context: {} },
		]);
		expect(component.eventHandlers).toBe("not a map");
	});

	test("inline widget trees lower and report their failing child", () => {
		const { component } = lowered({
			id: "w",
			type: "widgetInstance",
			instanceId: "w",
			widgetId: "card",
			inlineWidgetDef: {
				rootComponentId: "t",
				components: [{ id: "t", type: "text", content: "Inside" }],
			},
		});
		const definition = component.inlineWidgetDef as {
			rootComponentId: string;
			components: { component: Record<string, unknown> }[];
		};
		expect(definition.rootComponentId).toBe("t");
		expect(definition.components[0]?.component.content).toEqual({
			literalString: "Inside",
		});
		const outcome = lowerAuthoredComponent({
			id: "w",
			type: "widgetInstance",
			inlineWidgetDef: { components: [{ id: "bad" }] },
		});
		expect(outcome.ok).toBe(false);
		expect(outcome.ok ? "" : outcome.error).toStartWith(
			"inlineWidgetDef.components[0]: ",
		);
	});

	test("inline widget trees lower four levels deep", () => {
		const nest = (level: number): Record<string, unknown> =>
			level === 6
				? { id: "leaf", type: "text", content: "deepest" }
				: {
						id: `w${level}`,
						type: "widgetInstance",
						inlineWidgetDef: { components: [nest(level + 1)] },
					};
		let entry: Record<string, unknown> = lowered(nest(1));
		for (let level = 1; level <= 4; level++) {
			const component = entry.component as {
				inlineWidgetDef: { components: Record<string, unknown>[] };
			};
			entry = component.inlineWidgetDef.components[0] ?? {};
			expect(entry.component).toBeDefined();
		}
		const fifth = entry.component as {
			inlineWidgetDef: { components: Record<string, unknown>[] };
		};
		expect(fifth.inlineWidgetDef.components[0]).toEqual(nest(6));
	});

	test("unknown types and props are kept verbatim", () => {
		expect(lowered({ id: "x", type: "notAType", content: "Hi" })).toEqual({
			id: "x",
			component: { type: "notAType", content: "Hi" },
		});
	});

	test("does not mutate its input", () => {
		const input = {
			id: "c",
			component: {
				type: "column",
				children: ["a"],
				gap: "4px",
				action: { name: "send_message", context: { message: "x" } },
			},
		};
		const snapshot = structuredClone(input);
		lowered(input);
		expect(input).toEqual(snapshot);
	});

	test("list item fields lower recursively (tabs)", () => {
		const { component } = lowered({
			id: "tabs",
			type: "tabs",
			value: "a",
			tabs: [{ id: "a", label: "First", contentComponentId: "pa" }],
		});
		expect(component.tabs).toEqual([
			{ id: "a", label: { literalString: "First" }, contentComponentId: "pa" },
		]);
	});
});

describe("lowerBound", () => {
	test("wraps scalars and keeps bound values", () => {
		expect(lowerBound("x", "a")).toEqual({ literalString: "a" });
		expect(lowerBound("x", 0)).toEqual({ literalNumber: 0 });
		expect(lowerBound("x", false)).toEqual({ literalBool: false });
		expect(lowerBound("x", null)).toBeUndefined();
		const path = { path: "/a", defaultValue: 1 };
		expect(lowerBound("x", path)).toBe(path);
	});

	test("options arrays become literalOptions unless an item has no scalar value", () => {
		expect(lowerBound("options", [])).toEqual({ literalOptions: [] });
		expect(lowerBound("options", [true])).toEqual({ literalJson: "[true]" });
		expect(lowerBound("options", [{ label: "No value" }])).toEqual({
			literalJson: '[{"label":"No value"}]',
		});
		expect(lowerBound("options", [{ value: false }])).toEqual({
			literalOptions: [{ value: "false", label: "false" }],
		});
		expect(lowerBound("keys", ["a"])).toEqual({ literalJson: '["a"]' });
	});

	test("objects without bound keys become literalJson", () => {
		expect(lowerBound("x", { a: 1 })).toEqual({ literalJson: '{"a":1}' });
	});
});

describe("lowerChildren", () => {
	test("normalizes ids and keeps structured children", () => {
		expect(lowerChildren("a")).toEqual({ explicitList: ["a"] });
		expect(lowerChildren(["a", 1, true, { id: "x" }, "b"])).toEqual({
			explicitList: ["a", "1", "b"],
		});
		expect(lowerChildren(null)).toBeUndefined();
		const template = {
			template: { dataPath: "/items", templateComponentId: "row" },
		};
		expect(lowerChildren(template)).toBe(template);
	});
});
