import { describe, expect, test } from "bun:test";
import type { SurfaceComponent } from "../a2ui/types";
import {
	canAcceptComponentChildren,
	canMoveComponent,
	canReorderComponent,
	findComponentParent,
	getExplicitChildren,
	moveComponentInTree,
} from "./componentTree";

function component(id: string, children?: string[]): SurfaceComponent {
	return {
		id,
		component: children
			? { type: "column", children: { explicitList: children } }
			: { type: "text", text: { literalString: id } },
	} as SurfaceComponent;
}

function tree() {
	return new Map(
		[
			component("root", ["a", "b", "group"]),
			component("a"),
			component("b"),
			component("group", ["nested"]),
			component("nested", ["leaf"]),
			component("leaf"),
		].map((entry) => [entry.id, entry]),
	);
}

describe("builder component moves", () => {
	test("moves down and up using insertion boundaries without duplicates", () => {
		const original = tree();
		const down = moveComponentInTree(original, "a", "root", 2);
		expect(getExplicitChildren(down.get("root"))).toEqual(["b", "a", "group"]);
		const up = moveComponentInTree(down, "a", "root", 0);
		expect(getExplicitChildren(up.get("root"))).toEqual(["a", "b", "group"]);
		expect(getExplicitChildren(original.get("root"))).toEqual([
			"a",
			"b",
			"group",
		]);
	});

	test("moves between parents in one returned snapshot", () => {
		const next = moveComponentInTree(tree(), "a", "group", 0);
		expect(getExplicitChildren(next.get("root"))).toEqual(["b", "group"]);
		expect(getExplicitChildren(next.get("group"))).toEqual(["a", "nested"]);
	});

	test("appends at the end and ignores unchanged insertion boundaries", () => {
		const original = tree();
		expect(moveComponentInTree(original, "a", "root", 0)).toBe(original);
		expect(moveComponentInTree(original, "a", "root", 1)).toBe(original);
		expect(
			getExplicitChildren(
				moveComponentInTree(original, "a", "root").get("root"),
			),
		).toEqual(["b", "group", "a"]);
	});

	test("rejects self, descendant, missing target and root moves", () => {
		const original = tree();
		for (const [id, parentId] of [
			["group", "group"],
			["group", "nested"],
			["group", "leaf"],
			["a", "missing"],
			["root", "group"],
		]) {
			expect(moveComponentInTree(original, id, parentId)).toBe(original);
		}
	});

	test("detaches named child slots and checks them for cycles", () => {
		const original = tree();
		original.set("group", {
			id: "group",
			component: { type: "box", child: "nested" },
		} as unknown as SurfaceComponent);
		expect(canMoveComponent(original, "group", "leaf")).toBe(false);
		const next = moveComponentInTree(original, "nested", "root", 0);
		expect(next.get("group")?.component).not.toHaveProperty("child");
		expect(getExplicitChildren(next.get("root"))).toEqual([
			"nested",
			"a",
			"b",
			"group",
		]);
	});

	test("preserves data-bound children", () => {
		const original = tree();
		original.set("group", {
			id: "group",
			component: {
				type: "column",
				children: {
					template: { componentId: "nested", dataBinding: "/items" },
				},
			},
		} as unknown as SurfaceComponent);
		expect(moveComponentInTree(original, "a", "group")).toBe(original);
	});

	test("rejects parents whose renderers do not consume explicit children", () => {
		for (const type of ["text", "tabs", "accordion", "overlay"]) {
			expect(
				canAcceptComponentChildren({
					id: "target",
					component: { type },
				} as SurfaceComponent),
			).toBe(false);
		}
	});

	test("finds named content and prevents moving an ancestor into it", () => {
		for (const props of [
			{ type: "tabs", tabs: [{ contentComponentId: "nested" }] },
			{ type: "accordion", items: [{ contentComponentId: "nested" }] },
			{ type: "overlay", baseComponentId: "nested", overlays: [] },
			{ type: "overlay", overlays: [{ componentId: "nested" }] },
			{ type: "popover", contentComponentId: "nested" },
			{
				type: "column",
				children: { template: { templateComponentId: "nested" } },
			},
		]) {
			const original = tree();
			original.set("group", {
				id: "group",
				component: props,
			} as SurfaceComponent);
			expect(findComponentParent(original, "nested")).toBe("group");
			expect(canReorderComponent(original, "nested")).toBe(false);
			expect(moveComponentInTree(original, "group", "nested")).toBe(original);
		}
	});
});
