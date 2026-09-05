import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { SurfaceComponent } from "../a2ui/types";
import {
	type BuilderContextType,
	BuilderProvider,
	useBuilder,
} from "./BuilderContext";
import { getExplicitChildren } from "./componentTree";

test("component reparenting is one undoable change and redo restores both parents", async () => {
	const window = new Window({ url: "https://builder.local" });
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
	const initialComponents = [
		{
			id: "root",
			component: { type: "column", children: { explicitList: ["a", "group"] } },
		},
		{ id: "a", component: { type: "text", content: { literalString: "a" } } },
		{
			id: "group",
			component: { type: "column", children: { explicitList: [] } },
		},
	] as SurfaceComponent[];
	const host = window.document.createElement("div");
	const root = createRoot(host as unknown as HTMLElement);
	await act(() =>
		root.render(
			<BuilderProvider initialComponents={initialComponents}>
				<Reader />
			</BuilderProvider>,
		),
	);
	const context = () => {
		if (!builder) throw new Error("Builder did not mount");
		return builder;
	};
	await act(() => context().moveComponent("a", "group", 0));
	expect(getExplicitChildren(context().components.get("root"))).toEqual([
		"group",
	]);
	expect(getExplicitChildren(context().components.get("group"))).toEqual(["a"]);
	await act(() => context().undo());
	expect(getExplicitChildren(context().components.get("root"))).toEqual([
		"a",
		"group",
	]);
	expect(getExplicitChildren(context().components.get("group"))).toEqual([]);
	expect(context().canUndo).toBe(false);
	await act(() => context().redo());
	expect(getExplicitChildren(context().components.get("root"))).toEqual([
		"group",
	]);
	expect(getExplicitChildren(context().components.get("group"))).toEqual(["a"]);
	await act(() => root.unmount());
});
