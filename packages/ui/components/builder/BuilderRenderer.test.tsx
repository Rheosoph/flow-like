import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { ComponentProps } from "../a2ui/ComponentRegistry";
import { A2UIText } from "../a2ui/display/Text";
import { A2UIBox } from "../a2ui/layout/Box";
import { A2UIColumn } from "../a2ui/layout/Column";
import { A2UIRow } from "../a2ui/layout/Row";
import { A2UISpacer } from "../a2ui/layout/Spacer";
import type { Surface, SurfaceComponent } from "../a2ui/types";

const noop = () => {};
const runtimeClick = mock(noop);
const selected = mock((id: string, additive: boolean) => {});
const setActivatorNodeRef = mock((node: HTMLElement | null) => {});
const dragState = { activeId: null as string | null, overData: null };
function ActionProbe({ elementRef }: ComponentProps) {
	return (
		<button ref={elementRef} type="button" onClick={runtimeClick}>
			<span>Run action</span>
		</button>
	);
}
const builder = {
	selection: { componentIds: [] as string[] },
	selectComponent: selected,
	deleteComponents: noop,
	copy: noop,
	cut: noop,
	paste: noop,
	isComponentHidden: () => false,
	components: new Map<string, SurfaceComponent>(),
	actionContext: undefined,
	widgetRefs: {},
};

mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: string) => fallback ?? key,
	}),
}));
mock.module("./BuilderContext", () => ({ useBuilder: () => builder }));
mock.module("./WidgetBuilder", () => ({
	CONTAINER_TYPES: new Set(["row", "column", "box"]),
	ROOT_ID: "root",
}));
mock.module("./BuilderDndContext", () => ({
	COMPONENT_MOVE_TYPE: "a2ui-component-move",
	useBuilderDnd: () => dragState,
}));
mock.module("@dnd-kit/core", () => ({
	useDraggable: () => ({
		attributes: {},
		listeners: {},
		setNodeRef: noop,
		setActivatorNodeRef,
		isDragging: false,
	}),
	useDroppable: () => ({ setNodeRef: noop }),
}));
mock.module("../../lib/use-runtime-tailwind", () => ({
	useRuntimeTailwindStyles: noop,
}));
mock.module("../a2ui/ActionHandler", () => ({
	ActionProvider: ({ children }: { children: ReactNode }) => children,
}));
mock.module("../ui/tooltip", () => ({
	Tooltip: ({ children }: { children: ReactNode }) => children,
	TooltipTrigger: ({ children }: { children: ReactNode }) => children,
	TooltipContent: () => null,
}));
mock.module("../a2ui/ComponentRegistry", () => ({
	getComponentRenderer: (type: string) => {
		const renderers = {
			row: A2UIRow,
			column: A2UIColumn,
			box: A2UIBox,
			text: A2UIText,
			spacer: A2UISpacer,
			button: ActionProbe,
		};
		return renderers[type as keyof typeof renderers];
	},
}));

afterAll(() => mock.restore());

let root: Root | undefined;
let restoreGlobals: (() => void) | undefined;

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	restoreGlobals?.();
	restoreGlobals = undefined;
});

function createSurface(): Surface {
	const components = [
		{
			id: "root",
			component: {
				type: "row",
				children: { explicitList: ["title", "space", "group", "action"] },
			},
			style: { className: "content-row [&>span]:text-red-500" },
		},
		{
			id: "title",
			component: { type: "text", content: { literalString: "Title" } },
			style: { className: "grow order-2" },
		},
		{ id: "space", component: { type: "spacer" } },
		{
			id: "group",
			component: { type: "column", children: { explicitList: ["box"] } },
		},
		{
			id: "box",
			component: {
				type: "box",
				as: { literalString: "section" },
				children: { explicitList: ["nested"] },
			},
		},
		{
			id: "nested",
			component: { type: "text", content: { literalString: "Nested" } },
		},
		{ id: "action", component: { type: "button" } },
	] as SurfaceComponent[];
	return {
		id: "builder-test",
		rootComponentId: "root",
		components: Object.fromEntries(components.map((c) => [c.id, c])),
		dataModel: [],
	} as Surface;
}

async function renderBuilder(surface = createSurface()) {
	const window = new Window({ url: "https://example.test/builder" });
	Object.assign(window, { SyntaxError, TypeError });
	window.requestAnimationFrame = (() =>
		0) as unknown as Window["requestAnimationFrame"];
	window.cancelAnimationFrame = noop;
	const globals = {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		SVGElement: window.SVGElement,
		Node: window.Node,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	restoreGlobals = () => {
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	selected.mockClear();
	runtimeClick.mockClear();
	setActivatorNodeRef.mockClear();
	dragState.activeId = null;
	builder.selection = { componentIds: [] };
	builder.components = new Map(Object.entries(surface.components));
	const host = window.document.createElement("div");
	window.document.body.append(host);
	root = createRoot(host as unknown as HTMLElement);
	const { BuilderRenderer } = await import("./BuilderRenderer");
	const rerender = async () => {
		await act(() => root?.render(<BuilderRenderer surface={surface} />));
	};
	await rerender();
	return { host, window, rerender };
}

describe("BuilderRenderer element roots", () => {
	test("keeps styled elements as direct flex children and editor chrome outside the layout", async () => {
		const { host, window, rerender } = await renderBuilder();
		const row = host.querySelector('[data-builder-component="root"]');
		expect(row?.classList.contains("flex-row")).toBe(true);
		expect(Array.from(row?.children ?? []).map((node) => node.tagName)).toEqual(
			["SPAN", "DIV", "DIV", "BUTTON"],
		);
		const title = row?.querySelector(
			':scope > span[data-builder-component="title"]',
		);
		expect(title?.classList.contains("grow")).toBe(true);
		expect(title?.classList.contains("order-2")).toBe(true);
		const spacer = row?.querySelector(
			':scope > [data-builder-component="space"]',
		);
		expect((spacer as unknown as HTMLElement).style.flex).toBe("1 1 0%");
		expect(
			host.querySelector('[data-builder-component="group"] > section > span')
				?.textContent,
		).toBe("Nested");

		builder.selection = { componentIds: ["title"] };
		await rerender();
		expect(row?.querySelector('[data-builder-component="title"]')).toBe(title);
		expect(row?.children.length).toBe(4);
		expect(host.querySelector("[data-builder-chrome]")).toBeNull();
		expect(
			window.document.body.querySelector("[data-builder-chrome]"),
		).not.toBeNull();
	});

	test("selects only the clicked nested element and blocks its runtime action", async () => {
		const { host, window, rerender } = await renderBuilder();
		const nested = host.querySelector('[data-builder-component="nested"]');
		await act(() => {
			nested?.dispatchEvent(
				new window.MouseEvent("click", { bubbles: true, shiftKey: true }),
			);
		});
		expect(selected.mock.calls).toEqual([["nested", true]]);
		selected.mockClear();
		const button = host.querySelector('[data-builder-component="action"]');
		await act(() => {
			button?.firstElementChild?.dispatchEvent(
				new window.MouseEvent("click", { bubbles: true }),
			);
		});
		expect(selected.mock.calls).toEqual([["action", false]]);
		expect(runtimeClick).not.toHaveBeenCalled();
		builder.selection = { componentIds: ["action"] };
		await rerender();
		selected.mockClear();
		await act(() => {
			button?.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
		});
		expect(selected.mock.calls).toEqual([["action", false]]);
		expect(runtimeClick).not.toHaveBeenCalled();
	});

	test("keeps the selected element's drag handle mounted throughout a drag", async () => {
		const { window, rerender } = await renderBuilder();
		builder.selection = { componentIds: ["title"] };
		await rerender();
		const handle = window.document.querySelector(
			"[data-builder-toolbar] button",
		);
		expect(handle).not.toBeNull();
		setActivatorNodeRef.mockClear();
		dragState.activeId = "move-title";
		await rerender();
		expect(window.document.querySelector("[data-builder-toolbar] button")).toBe(
			handle,
		);
		expect(setActivatorNodeRef.mock.calls).toEqual([]);
		dragState.activeId = null;
		await rerender();
		expect(window.document.querySelector("[data-builder-toolbar] button")).toBe(
			handle,
		);
	});

	test("shows one action toolbar outside clipped outlines for a multiple selection", async () => {
		const { host, window, rerender } = await renderBuilder();
		host.setAttribute("data-builder-root", "editor-a");
		builder.selection = { componentIds: ["root", "title"] };
		await rerender();
		const toolbars = window.document.querySelectorAll("[data-builder-toolbar]");
		expect(toolbars.length).toBe(1);
		const toolbar = toolbars[0];
		expect(toolbar?.parentElement?.tagName).toBe("BODY");
		expect(toolbar?.closest("[data-builder-chrome]")).toBeNull();
		expect(toolbar?.getAttribute("data-builder-owner")).toBe("editor-a");
		expect(toolbar?.textContent).toContain("text");
		expect(toolbar?.querySelector('[aria-label="Copy"]')).not.toBeNull();
		expect(toolbar?.querySelector('[aria-label="Paste"]')).not.toBeNull();
		expect(host.querySelector("[data-builder-toolbar]")).toBeNull();
	});

	test("makes an empty container selectable through a portal without changing its layout", async () => {
		const surface = createSurface();
		surface.components.box = {
			...surface.components.box,
			component: {
				...surface.components.box.component,
				children: { explicitList: [] },
			},
			style: { className: "empty-slot w-0 h-0" },
		};
		const { host, window } = await renderBuilder(surface);
		const empty = host.querySelector('[data-builder-component="box"]');
		expect(empty?.tagName).toBe("SECTION");
		expect(empty?.hasAttribute("data-builder-empty")).toBe(true);
		expect(empty?.className).toBe("empty-slot w-0 h-0");
		expect(empty?.getAttribute("style") ?? "").toBe("");
		expect(empty?.children.length).toBe(0);
		expect(empty?.parentElement?.getAttribute("data-builder-component")).toBe(
			"group",
		);
		expect(host.querySelector("[data-builder-chrome]")).toBeNull();

		const hint = window.document.querySelector("[data-builder-chrome] button");
		const hintFrame = hint?.parentElement as unknown as HTMLElement | undefined;
		expect(hint).not.toBeNull();
		expect(hint?.textContent).toContain("Empty");
		expect(hintFrame?.style.width).toBe("48px");
		expect(hintFrame?.style.height).toBe("32px");
		await act(() => {
			hint?.dispatchEvent(
				new window.MouseEvent("click", { bubbles: true, ctrlKey: true }),
			);
		});
		expect(selected.mock.calls).toEqual([["box", true]]);
		expect(empty?.children.length).toBe(0);
	});
});
