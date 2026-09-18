import { describe, expect, test } from "bun:test";
import { applyA2UIMessage } from "./apply-a2ui-message";
import { pruneDetached } from "./detached-children";
import { foldA2UIServerMessage } from "./fold-surfaces";
import type { A2UIServerMessage, Surface, SurfaceComponent } from "./types";

const SURFACE = "page-1";

function component(
	id: string,
	data: Record<string, unknown>,
): SurfaceComponent {
	return { id, component: data } as unknown as SurfaceComponent;
}

function page(extra: Record<string, SurfaceComponent> = {}): Surface {
	return {
		id: SURFACE,
		rootComponentId: "root",
		components: {
			root: component("root", {
				type: "column",
				children: { explicitList: ["feed-list", "side-list"] },
			}),
			"feed-list": component("feed-list", {
				type: "grid",
				children: { explicitList: [] },
			}),
			"side-list": component("side-list", {
				type: "column",
				children: { explicitList: [] },
			}),
			"row-template": component("row-template", {
				type: "widgetInstance",
				instanceId: "row-template",
			}),
			...extra,
		},
	};
}

const upsert = (
	elementId: string,
	value: Record<string, unknown>,
): A2UIServerMessage => ({
	type: "upsertElement",
	element_id: elementId,
	value,
});

const instantiate = (id: string) =>
	upsert(id, {
		type: "createComponent",
		component: {
			type: "widgetInstance",
			instanceId: id,
			inlineWidgetDef: {
				rootComponentId: "row-root",
				components: [
					{
						id: "row-root",
						component: {
							type: "column",
							children: { explicitList: ["row-name"] },
						},
					},
					{ id: "row-name", component: { type: "text" } },
				],
			},
		},
	});

const create = (id: string, children: string[] = []) =>
	upsert(id, {
		type: "createComponent",
		component: { type: "row", children: { explicitList: children } },
	});

const push = (container: string, childId: string) =>
	upsert(container, { type: "pushChild", childId });
const clear = (container: string) =>
	upsert(container, { type: "clearChildren" });
const removeAt = (container: string, index: number) =>
	upsert(container, { type: "removeChildAt", index });
const prune = (...elementIds: string[]): A2UIServerMessage => ({
	type: "pruneDetached",
	element_ids: elementIds,
});

function apply(surface: Surface, messages: A2UIServerMessage[]): Surface {
	return messages.reduce(applyA2UIMessage, surface);
}

const ids = (surface: Surface) => Object.keys(surface.components).sort();
const childList = (surface: Surface, id: string) =>
	(
		surface.components[id]?.component as unknown as {
			children: { explicitList: string[] };
		}
	).children.explicitList;

function withRows(...rows: string[]): Surface {
	return apply(
		page(),
		rows.flatMap((row) => [instantiate(row), push("feed-list", row)]),
	);
}

describe("clearing a list and rendering the next page", () => {
	test("deletes the previous rows when the run ends", () => {
		const next = apply(withRows("a", "b"), [
			clear("feed-list"),
			instantiate("c"),
			push("feed-list", "c"),
			prune("feed-list"),
		]);

		expect(ids(next)).toEqual([
			"c",
			"feed-list",
			"root",
			"row-template",
			"side-list",
		]);
		expect(next.detachedChildren).toBeUndefined();
	});

	test("keeps the previous rows until the run ends", () => {
		const mid = apply(withRows("a", "b"), [
			clear("feed-list"),
			instantiate("c"),
			push("feed-list", "c"),
		]);

		expect(mid.components.a).toBeDefined();
		expect(mid.detachedChildren).toEqual({ "feed-list": ["a", "b"] });
	});

	test("keeps rows the run pushed back, in their new order", () => {
		const next = apply(withRows("a", "b"), [
			clear("feed-list"),
			push("feed-list", "b"),
			push("feed-list", "a"),
			prune("feed-list"),
		]);

		expect(childList(next, "feed-list")).toEqual(["b", "a"]);
		expect(next.components.a).toBeDefined();
		expect(next.components.b).toBeDefined();
	});

	test("keeps stable-id rows instantiated before the clear", () => {
		const next = apply(withRows("a", "b"), [
			instantiate("a"),
			instantiate("b"),
			clear("feed-list"),
			push("feed-list", "a"),
			prune("feed-list"),
		]);

		expect(next.components.a).toBeDefined();
		expect(next.components.b).toBeUndefined();
	});

	test("keeps a row re-created after the clear for a later run to push", () => {
		const next = apply(withRows("a"), [
			clear("feed-list"),
			instantiate("a"),
			prune("feed-list"),
		]);

		expect(next.components.a).toBeDefined();
	});

	test("never deletes authored components that no container lists", () => {
		const next = apply(withRows("a"), [clear("feed-list"), prune("feed-list")]);
		expect(next.components["row-template"]).toBeDefined();
	});
});

describe("removing and moving children", () => {
	test("removeChildAt deletes only the removed child", () => {
		const next = apply(withRows("a", "b"), [
			removeAt("feed-list", 0),
			prune("feed-list"),
		]);

		expect(next.components.a).toBeUndefined();
		expect(childList(next, "feed-list")).toEqual(["b"]);
	});

	test("a child moved to another container stays", () => {
		const next = apply(withRows("a"), [
			removeAt("feed-list", 0),
			push("side-list", "a"),
			prune("feed-list"),
		]);

		expect(childList(next, "side-list")).toEqual(["a"]);
		expect(next.components.a).toBeDefined();
	});

	test("a child still listed by another container is never recorded", () => {
		const next = apply(withRows("a"), [
			push("side-list", "a"),
			clear("feed-list"),
		]);
		expect(next.detachedChildren).toBeUndefined();
	});

	test("removeElement deletes the element now and its orphaned children at run end", () => {
		const built = apply(page(), [
			create("label"),
			create("card", ["label"]),
			push("feed-list", "card"),
		]);
		const removed = applyA2UIMessage(built, {
			type: "removeElement",
			surfaceId: SURFACE,
			elementId: "card",
		});

		expect(removed.components.card).toBeUndefined();
		expect(removed.components.label).toBeDefined();
		expect(
			applyA2UIMessage(removed, prune("card")).components.label,
		).toBeUndefined();
	});
});

describe("pruneDetached", () => {
	test("deletes descendants the deleted rows leave unreferenced, but not shared ones", () => {
		const next = apply(page(), [
			create("label"),
			create("shared"),
			create("card", ["label", "shared"]),
			push("feed-list", "card"),
			push("side-list", "shared"),
			clear("feed-list"),
			prune("feed-list"),
		]);

		expect(next.components.card).toBeUndefined();
		expect(next.components.label).toBeUndefined();
		expect(next.components.shared).toBeDefined();
	});

	test("only prunes the containers the run named", () => {
		const next = apply(page(), [
			instantiate("a"),
			push("feed-list", "a"),
			instantiate("x"),
			push("side-list", "x"),
			clear("feed-list"),
			clear("side-list"),
			prune("feed-list"),
		]);

		expect(next.components.a).toBeUndefined();
		expect(next.components.x).toBeDefined();
		expect(next.detachedChildren).toEqual({ "side-list": ["x"] });
	});

	test("keeps surface components pushed into a widget's internal container", () => {
		const next = apply(withRows("host"), [
			create("nested"),
			push("side-list", "nested"),
			push("host/row-root", "nested"),
			clear("side-list"),
			clear("feed-list"),
			prune("side-list", "feed-list"),
		]);

		expect(next.components.nested).toBeUndefined();
		expect(next.components.host).toBeUndefined();

		const kept = apply(withRows("host"), [
			create("nested"),
			push("side-list", "nested"),
			push("host/row-root", "nested"),
			clear("side-list"),
			prune("side-list"),
		]);
		expect(kept.components.nested).toBeDefined();
	});

	test("never deletes the surface root", () => {
		const surface = { ...page(), detachedChildren: { list: ["root"] } };
		expect(pruneDetached(surface, ["list"]).components.root).toBeDefined();
	});

	test("returns the same surface when nothing was recorded", () => {
		const surface = withRows("a");
		expect(applyA2UIMessage(surface, prune("feed-list"))).toBe(surface);
	});

	test("reaches the surface that recorded the detach in a surface map", () => {
		const other: Surface = {
			id: "page-2",
			rootComponentId: "",
			components: {},
		};
		let surfaces = new Map([
			[SURFACE, withRows("a")],
			["page-2", other],
		]);
		for (const message of [
			upsert(`${SURFACE}/feed-list`, { type: "clearChildren" }),
			prune(`${SURFACE}/feed-list`),
		]) {
			surfaces = foldA2UIServerMessage(surfaces, message);
		}

		expect(surfaces.get(SURFACE)?.components.a).toBeUndefined();
		expect(surfaces.get("page-2")).toBe(other);
	});
});
