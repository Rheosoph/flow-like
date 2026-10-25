import { afterAll, afterEach, expect, mock, test } from "bun:test";
import type { Node } from "@xyflow/react";
import { Window } from "happy-dom";
import { act, useLayoutEffect } from "react";
import {
	type BoardRuntime,
	BoardRuntimeProvider,
	useBoardRuntime,
} from "./board-runtime";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLButtonElement: window.HTMLButtonElement,
	Element: window.Element,
	Node: window.Node,
	NodeFilter: window.NodeFilter,
	MutationObserver: window.MutationObserver,
	ResizeObserver: window.ResizeObserver,
	CustomEvent: window.CustomEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const locales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...locales,
	useTranslation: () => ({
		t: (_key: string, fallback: string, values: Record<string, unknown> = {}) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => String(values[key] ?? "")),
	}),
}));
const { createRoot } = await import("react-dom/client");
const { LargeGraphNavigation } = await import("./large-graph-navigation");
const roots: ReturnType<typeof createRoot>[] = [];

afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.replaceChildren();
});
afterAll(() => {
	mock.restore();
	mock.module("@flow-like/locales", () => locales);
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	window.happyDOM.abort();
});

function graph(count = 1634): Node[] {
	return Array.from({ length: count }, (_, index) => ({
		id: `node-${index}`,
		position: { x: index * 200, y: 100 },
		measured: { width: 150, height: 80 },
		data: { node: { friendly_name: `Step ${index}` } },
	}));
}

async function render(nodes: Node[], native = true) {
	let runtime: BoardRuntime | undefined;
	function Setup() {
		const current = useBoardRuntime();
		useLayoutEffect(() => {
			runtime = current;
			current?.configure({
				nodes,
				width: 1000,
				height: 600,
				transform: [0, 0, 0.1],
			});
		}, [current, nodes]);
		return <LargeGraphNavigation nodes={nodes} native={native} />;
	}
	const element = window.document.createElement("div");
	window.document.body.append(element);
	const root = createRoot(element as unknown as HTMLElement);
	roots.push(root);
	await act(async () =>
		root.render(
			<BoardRuntimeProvider>
				<Setup />
			</BoardRuntimeProvider>,
		),
	);
	if (!runtime) throw new Error("Board runtime did not mount");
	return { runtime, element };
}

function button(label: string) {
	const found = [...window.document.querySelectorAll("button")].find(
		(candidate) => candidate.textContent === label,
	);
	if (!found) throw new Error(`Missing button: ${label}`);
	return found;
}

test("small boards keep their existing controls", async () => {
	const { element } = await render(graph(20));
	expect(element.querySelector("[data-large-graph-navigation]")).toBeNull();
});

test("selected-node focus restores readable zoom without changing the graph", async () => {
	const nodes = graph();
	nodes[42].selected = true;
	const { runtime, element } = await render(nodes);
	expect(element.textContent).toContain("Overview · 1,634 nodes");
	expect(element.textContent).toContain(
		"Node details are reduced at this zoom",
	);
	await act(async () => button("Focus selected node").click());
	expect(runtime.flow.getViewport()).toEqual({ x: -7975, y: 160, zoom: 1 });
	expect(runtime.flow.getNodes()).toEqual(nodes);
	expect(element.textContent).not.toContain("Overview");
	await act(async () => button("Fit graph").click());
	expect(runtime.flow.getZoom()).toBeLessThan(0.5);
	expect(element.textContent).toContain("Overview");
});

test("search bounds the list and focuses matching nodes by name or ID", async () => {
	const nodes = graph();
	nodes[42].data = { node: { friendly_name: "Destination" } };
	const { runtime, element } = await render(nodes, false);
	expect(element.textContent).not.toContain("Node details are reduced");
	await act(async () => button("Browse nodes").click());
	expect(window.document.querySelectorAll("li").length).toBe(25);
	const input = window.document.querySelector("input");
	if (!input) throw new Error("Node search did not open");
	const setter = Object.getOwnPropertyDescriptor(
		window.HTMLInputElement.prototype,
		"value",
	)?.set;
	for (const query of ["destination", "node-42"]) {
		await act(async () => {
			setter?.call(input, query);
			input.dispatchEvent(new window.Event("input", { bubbles: true }));
		});
		expect(window.document.querySelector("ul")?.textContent).toContain(
			"Destination",
		);
	}
	await act(async () => button("Destinationnode-42").click());
	expect(runtime.flow.getViewport()).toEqual({ x: -7975, y: 160, zoom: 1 });
	expect(window.document.querySelector("input")).toBeNull();
	expect(runtime.flow.getNodes()).toEqual(nodes);
});
