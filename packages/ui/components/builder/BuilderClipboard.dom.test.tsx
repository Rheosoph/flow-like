import { afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { IWidgetRef } from "../../state/backend-state/page-state";
import type { A2UIComponent, SurfaceComponent } from "../a2ui/types";
import {
	type BuilderContextType,
	BuilderProvider,
	useBuilder,
} from "./BuilderContext";
import { getComponentChildren, getExplicitChildren } from "./componentTree";

const column = (id: string, children: string[] = []): SurfaceComponent => ({
	id,
	component: { id, type: "column", children: { explicitList: children } },
});
const textComponent = (id: string): SurfaceComponent => ({
	id,
	component: { id, type: "text", content: { literalString: id } },
});
const cleanups: (() => Promise<void>)[] = [];

afterEach(async () => {
	for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function mountBuilder(
	initialComponents: SurfaceComponent[],
	initialWidgetRefs: Record<string, IWidgetRef> = {},
	storedClipboard?: string | null,
) {
	const window = new Window({ url: "https://builder.local" });
	if (storedClipboard)
		window.localStorage.setItem("a2ui-clipboard", storedClipboard);
	Object.assign(globalThis, {
		window,
		document: window.document,
		localStorage: window.localStorage,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	let builder: BuilderContextType | undefined;
	function Reader() {
		builder = useBuilder();
		return null;
	}
	const root = createRoot(
		window.document.createElement("div") as unknown as HTMLElement,
	);
	await act(() =>
		root.render(
			<BuilderProvider
				initialComponents={initialComponents}
				initialWidgetRefs={initialWidgetRefs}
			>
				<Reader />
			</BuilderProvider>,
		),
	);
	cleanups.push(async () => {
		await act(() => root.unmount());
	});
	const context = () => {
		if (!builder) throw new Error("Builder did not mount");
		return builder;
	};
	return { context, window };
}

test("copy then default paste works immediately on a selected leaf and is one undoable edit", async () => {
	const { context } = await mountBuilder([
		column("root", ["a"]),
		textComponent("a"),
	]);
	await act(() => context().selectComponent("a"));
	await act(() => {
		context().copy();
		context().paste();
	});
	const pastedId = context().selection.componentIds[0];
	expect(pastedId).not.toBe("a");
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"a",
		pastedId,
	]);
	expect(context().getComponent(pastedId)?.component).toEqual({
		...textComponent("a").component,
		id: pastedId,
	});
	await act(() => context().undo());
	expect([...context().components.keys()]).toEqual(["root", "a"]);
	expect(context().canUndo).toBe(false);
	await act(() => context().redo());
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"a",
		pastedId,
	]);
});

test("multi-selection copies each subtree once and pastes at the root with no selection", async () => {
	const { context } = await mountBuilder([
		column("root", ["group"]),
		column("group", ["a"]),
		textComponent("a"),
	]);
	await act(() => context().setSelection({ componentIds: ["a", "group"] }));
	await act(() => context().copy());
	expect(context().clipboard?.rootIds).toEqual(["group"]);
	expect(context().clipboard?.components).toHaveLength(2);
	await act(() => context().deselectAll());
	await act(() => context().paste());
	const pastedId = context().selection.componentIds[0];
	const children = getExplicitChildren(context().getComponent(pastedId));
	expect(children).toHaveLength(1);
	expect(children[0]).not.toBe("a");
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"group",
		pastedId,
	]);
	expect(context().components.size).toBe(5);
});

test("paste into an empty container initializes its child list", async () => {
	const { context } = await mountBuilder([
		column("root", ["a", "empty"]),
		textComponent("a"),
		{ id: "empty", component: { id: "empty", type: "column" } },
	]);
	await act(() => context().copy(["a"]));
	await act(() => context().selectComponent("empty"));
	await act(() => context().paste());
	expect(getExplicitChildren(context().getComponent("empty"))).toEqual(
		context().selection.componentIds,
	);
});

test("rapid repeated paste creates distinct components without overwriting the prior paste", async () => {
	const { context } = await mountBuilder([
		column("root", ["a"]),
		textComponent("a"),
	]);
	await act(() => context().copy(["a"]));
	await act(() => {
		context().paste("root");
		context().paste("root");
	});
	expect(context().components.size).toBe(4);
	const children = getExplicitChildren(context().getComponent("root"));
	expect(children).toHaveLength(3);
	expect(new Set(children).size).toBe(3);
	await act(() => context().undo());
	expect(context().components.size).toBe(3);
	await act(() => context().undo());
	expect(context().components.size).toBe(2);
});

test("duplicate synchronously inserts sibling subtrees and preserves the existing clipboard", async () => {
	const { context } = await mountBuilder([
		column("root", ["left", "right"]),
		column("left", ["a"]),
		column("right", ["b"]),
		textComponent("a"),
		textComponent("b"),
	]);
	await act(() => context().copy(["right"]));
	await act(() => context().setSelection({ componentIds: ["a", "b"] }));
	await act(() => context().duplicate());
	const [aCopy, bCopy] = context().selection.componentIds;
	expect(getExplicitChildren(context().getComponent("left"))).toEqual([
		"a",
		aCopy,
	]);
	expect(getExplicitChildren(context().getComponent("right"))).toEqual([
		"b",
		bCopy,
	]);
	expect(context().clipboard?.rootIds).toEqual(["right"]);
	await act(() => context().undo());
	expect(context().components.size).toBe(5);
	expect(context().canUndo).toBe(false);
});

test("copy includes and remaps nested tab, accordion, overlay, and template references", async () => {
	const components = [
		column("root", ["tabs"]),
		{
			id: "tabs",
			component: {
				type: "tabs",
				tabs: [
					{
						id: "tab",
						label: { literalString: "accordion" },
						contentComponentId: "accordion",
					},
				],
			},
		},
		{
			id: "accordion",
			component: {
				type: "accordion",
				items: [
					{
						id: "item",
						title: { literalString: "overlay" },
						contentComponentId: "overlay",
					},
				],
			},
		},
		{
			id: "overlay",
			component: {
				type: "overlay",
				baseComponentId: "template",
				overlays: [{ componentId: "text" }],
			},
		},
		{
			id: "template",
			component: {
				type: "column",
				children: {
					template: { templateComponentId: "text", dataBinding: "/items" },
				},
			},
		},
		textComponent("text"),
	] as SurfaceComponent[];
	const { context } = await mountBuilder(components);
	await act(() => context().copy(["tabs"]));
	expect(context().clipboard?.components).toHaveLength(5);
	await act(() => context().paste("root"));
	const originalIds = new Set(components.map((component) => component.id));
	const clones = [...context().components.values()].filter(
		(component) => !originalIds.has(component.id),
	);
	expect(clones).toHaveLength(5);
	for (const clone of clones) {
		for (const childId of getComponentChildren(clone)) {
			expect(originalIds.has(childId)).toBe(false);
			expect(context().components.has(childId)).toBe(true);
		}
	}
	const clonedTabs = clones.find(
		(component) => component.component.type === "tabs",
	)?.component;
	if (clonedTabs?.type !== "tabs") throw new Error("Missing copied tabs");
	expect(clonedTabs.tabs[0].label).toEqual({ literalString: "accordion" });
	const clonedText = clones.find(
		(component) => component.component.type === "text",
	);
	if (!clonedText) throw new Error("Missing copied text");
	expect(clonedText.component).toEqual({
		...textComponent("text").component,
		id: clonedText.id,
	});
});

test("cut cannot paste a subtree into itself and moves it intact with one undo entry", async () => {
	const { context } = await mountBuilder([
		column("root", ["group", "destination"]),
		column("group", ["nested"]),
		column("nested", ["a"]),
		textComponent("a"),
		column("destination"),
	]);
	await act(() => context().cut(["group"]));
	await act(() => context().paste("nested"));
	expect(context().components.size).toBe(5);
	expect(context().canUndo).toBe(false);
	expect(context().clipboard?.cut).toBe(true);
	await act(() => context().paste("destination"));
	expect(context().selection.componentIds).toEqual(["group"]);
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"destination",
	]);
	expect(getExplicitChildren(context().getComponent("destination"))).toEqual([
		"group",
	]);
	expect(getExplicitChildren(context().getComponent("group"))).toEqual([
		"nested",
	]);
	expect(context().clipboard).toBeNull();
	await act(() => context().undo());
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"group",
		"destination",
	]);
	expect(context().canUndo).toBe(false);
	await act(() => context().redo());
	expect(getExplicitChildren(context().getComponent("destination"))).toEqual([
		"group",
	]);
});

test("cut preserves the root and required named content slots", async () => {
	const { context } = await mountBuilder([
		column("root", ["tabs"]),
		{
			id: "tabs",
			component: {
				type: "tabs",
				tabs: [
					{
						id: "tab",
						label: { literalString: "Content" },
						contentComponentId: "content",
					},
				],
			},
		},
		column("content"),
	] as SurfaceComponent[]);
	await act(() => context().cut(["root"]));
	expect(context().clipboard).toBeNull();
	await act(() => context().cut(["content"]));
	expect(context().clipboard).toBeNull();
	expect(context().components.size).toBe(3);
});

test("widget copies retain their definition across editors and undo/redo restores their refs", async () => {
	const widget: IWidgetRef = {
		id: "widget",
		name: "Widget",
		rootComponentId: "widget-root",
		components: [textComponent("widget-root")],
		tags: [],
		createdAt: "",
		updatedAt: "",
	};
	const first = await mountBuilder(
		[
			column("root", ["instance"]),
			{
				id: "instance",
				component: {
					id: "instance",
					type: "widgetInstance",
					instanceId: "original-instance",
					widgetId: "widget",
				},
			},
		],
		{ "original-instance": widget },
	);
	await act(() => first.context().cut(["instance"]));
	const { context } = await mountBuilder(
		[column("root", ["instance"]), textComponent("instance")],
		{},
		first.window.localStorage.getItem("a2ui-clipboard"),
	);
	expect(context().clipboard?.cut).toBe(false);
	await act(() => context().paste());
	const copyId = context().selection.componentIds[0];
	const copied = context().getComponent(copyId)?.component;
	if (copied?.type !== "widgetInstance")
		throw new Error("Missing copied widget");
	expect(copied.instanceId).not.toBe("original-instance");
	expect(context().getWidgetRef(copied.instanceId)).toEqual(widget);
	expect(context().getComponent("instance")?.component.type).toBe("text");
	await act(() => context().undo());
	expect(context().widgetRefs.size).toBe(0);
	expect(context().components.size).toBe(2);
	await act(() => context().redo());
	expect(context().getWidgetRef(copied.instanceId)).toEqual(widget);
	expect(context().getComponent(copyId)?.component).toEqual(copied);
});

test("micro widget copies receive an independent instance ID", async () => {
	const { context } = await mountBuilder([
		column("root", ["instance"]),
		{
			id: "instance",
			component: {
				type: "microWidgetInstance",
				instanceId: "micro-instance",
				widgetId: "widget",
				packageId: "package",
				packageVersion: "1.0.0",
			} as A2UIComponent,
		},
	]);
	await act(() => context().selectComponent("instance"));
	await act(() => context().duplicate());
	const copy = context().getComponent(
		context().selection.componentIds[0],
	)?.component;
	if (copy?.type !== "microWidgetInstance")
		throw new Error("Missing copied micro widget");
	expect(copy.instanceId).not.toBe("micro-instance");
	expect(copy.widgetId).toBe("widget");
});
