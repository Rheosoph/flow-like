import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { Root } from "react-dom/client";
import type { SurfaceComponent } from "../a2ui/types";
import type { BuilderContextType } from "./BuilderContext";
import { getExplicitChildren } from "./componentTree";

const window = new Window({ url: "https://builder.local" });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	SVGElement: window.SVGElement,
	Element: window.Element,
	Node: window.Node,
	Document: window.Document,
	DOMRect: window.DOMRect,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	NodeFilter: window.NodeFilter,
	MutationObserver: window.MutationObserver,
	ResizeObserver: window.ResizeObserver,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: window.requestAnimationFrame.bind(window),
	cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const previousGlobals = Object.fromEntries(
	Object.keys(globals).map((key) => [
		key,
		Object.getOwnPropertyDescriptor(globalThis, key),
	]),
);
Object.assign(globalThis, globals);
Object.assign(window, { SyntaxError, TypeError });

mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: string, values?: Record<string, string>) =>
			(fallback ?? key).replace("{{id}}", values?.id ?? ""),
	}),
}));
mock.module("./WidgetBuilder", () => ({
	CONTAINER_TYPES: new Set(["row", "column", "box"]),
	ROOT_ID: "root",
}));
mock.module("../../state/backend-state", () => ({
	useBackend: () => ({ widgetState: {} }),
}));

const { createRoot } = await import("react-dom/client");
const { BuilderProvider, useBuilder } = await import("./BuilderContext");
const { BuilderDndProvider, useBuilderDnd } = await import(
	"./BuilderDndContext"
);
const { HierarchyTree } = await import("./HierarchyTree");

let root: Root | undefined;
let builder: BuilderContextType | undefined;
let dragState: ReturnType<typeof useBuilderDnd> | undefined;

function Reader() {
	builder = useBuilder();
	dragState = useBuilderDnd();
	return null;
}

function context() {
	if (!builder) throw new Error("Builder did not mount");
	return builder;
}

// Happy DOM does not lay out CSS. The hierarchy rows and their insertion zones
// receive deterministic bounds while the real refs, sensors, and drop handlers run.
window.HTMLElement.prototype.getBoundingClientRect = function () {
	const row = this.closest('[role="treeitem"]');
	if (row) {
		const rows = Array.from(
			window.document.querySelectorAll('[role="treeitem"]'),
		);
		const top = 60 + rows.indexOf(row) * 32;
		if (this === row) return new window.DOMRect(0, top, 240, 32);
		if (this.classList.contains("absolute")) {
			const height = this.classList.contains("h-1/4") ? 8 : 16;
			return new window.DOMRect(
				0,
				this.classList.contains("bottom-0") ? top + 32 - height : top,
				240,
				height,
			);
		}
		return new window.DOMRect(24, top + 8, 16, 16);
	}
	return new window.DOMRect(0, 0, 800, 600);
};

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	builder = undefined;
	dragState = undefined;
	window.document.body.innerHTML = "";
});

afterAll(() => {
	mock.restore();
	for (const [key, descriptor] of Object.entries(previousGlobals)) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	window.happyDOM.abort();
});

async function renderHierarchy() {
	const host = window.document.createElement("div");
	window.document.body.append(host);
	root = createRoot(host as unknown as HTMLElement);
	const components = [
		{
			id: "root",
			component: {
				type: "column",
				children: { explicitList: ["a", "b", "group"] },
			},
		},
		{ id: "a", component: { type: "text", content: { literalString: "A" } } },
		{ id: "b", component: { type: "text", content: { literalString: "B" } } },
		{
			id: "group",
			component: { type: "column", children: { explicitList: [] } },
		},
	] as SurfaceComponent[];
	await act(() =>
		root?.render(
			<BuilderProvider initialComponents={components}>
				<BuilderDndProvider setIsDraggingGlobal={() => {}}>
					<Reader />
					<HierarchyTree />
				</BuilderDndProvider>
			</BuilderProvider>,
		),
	);
	await act(() => context().selectComponent("a"));
	return host;
}

async function pointerEvent(
	target: Pick<Window["document"], "dispatchEvent">,
	type: string,
	x: number,
	y: number,
) {
	await act(async () => {
		target.dispatchEvent(
			new window.PointerEvent(type, {
				bubbles: true,
				cancelable: true,
				isPrimary: true,
				pointerType: "mouse",
				pointerId: 1,
				button: 0,
				buttons: type === "pointerup" ? 0 : 1,
				clientX: x,
				clientY: y,
			}),
		);
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function startDrag(id: string) {
	const handle = window.document.querySelector(`[aria-label="Reorder ${id}"]`);
	if (!handle) throw new Error(`Missing drag handle for ${id}`);
	const rect = handle.getBoundingClientRect();
	await pointerEvent(handle, "pointerdown", rect.x + 8, rect.y + 8);
	await pointerEvent(window.document, "pointermove", rect.x + 20, rect.y + 8);
	expect(dragState?.activeId).toBe(`tree-move-${id}`);
}

test("hierarchy pointer drag reorders siblings and undo restores the order", async () => {
	await renderHierarchy();
	await startDrag("a");
	const destination = window.document.getElementById("tree-node-b");
	if (!destination) throw new Error("Missing destination row");
	const rect = destination.getBoundingClientRect();
	await pointerEvent(window.document, "pointermove", 100, rect.bottom - 3);
	expect(dragState?.overId).toBe("tree-after-b");
	await pointerEvent(window.document, "pointerup", 100, rect.bottom - 3);
	expect(getExplicitChildren(context().components.get("root"))).toEqual([
		"b",
		"a",
		"group",
	]);
	expect(
		Array.from(window.document.querySelectorAll('[role="treeitem"]')).map(
			(row) => row.id,
		),
	).toEqual([
		"tree-node-root",
		"tree-node-b",
		"tree-node-a",
		"tree-node-group",
	]);
	expect(dragState?.activeId).toBeNull();
	await act(() => context().undo());
	expect(getExplicitChildren(context().components.get("root"))).toEqual([
		"a",
		"b",
		"group",
	]);
});

test("hierarchy pointer drag reparents into an empty container", async () => {
	await renderHierarchy();
	await startDrag("a");
	const destination = window.document.getElementById("tree-node-group");
	if (!destination) throw new Error("Missing destination row");
	const rect = destination.getBoundingClientRect();
	await pointerEvent(
		window.document,
		"pointermove",
		100,
		rect.top + rect.height / 2,
	);
	expect(dragState?.overId).toBe("tree-drop-group");
	await pointerEvent(
		window.document,
		"pointerup",
		100,
		rect.top + rect.height / 2,
	);
	expect(getExplicitChildren(context().components.get("root"))).toEqual([
		"b",
		"group",
	]);
	expect(getExplicitChildren(context().components.get("group"))).toEqual(["a"]);
	expect(destination.getAttribute("aria-expanded")).toBe("true");
	expect(
		window.document
			.getElementById("tree-node-a")
			?.getAttribute("aria-selected"),
	).toBe("true");
});

test("hierarchy context menu copies and cuts the targeted row", async () => {
	await renderHierarchy();
	const row = window.document.getElementById("tree-node-b");
	if (!row) throw new Error("Missing context menu row");
	const openMenu = async () => {
		await act(async () => {
			row.dispatchEvent(
				new window.MouseEvent("contextmenu", {
					bubbles: true,
					cancelable: true,
					clientX: 100,
					clientY: 140,
				}),
			);
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	};
	const chooseItem = async (label: string) => {
		const item = Array.from(
			window.document.querySelectorAll('[role="menuitem"]'),
		).find((candidate) => candidate.textContent === label);
		if (!item) throw new Error(`Missing ${label} menu item`);
		await act(() => {
			item.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
		});
	};
	await openMenu();
	await chooseItem("Copy");
	expect(context().clipboard?.rootIds).toEqual(["b"]);
	expect(context().clipboard?.cut).toBe(false);
	await openMenu();
	await chooseItem("Cut");
	expect(context().clipboard?.rootIds).toEqual(["b"]);
	expect(context().clipboard?.cut).toBe(true);
});
