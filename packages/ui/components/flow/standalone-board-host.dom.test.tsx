import { afterAll, afterEach, expect, test } from "bun:test";
import {
	type Edge,
	type EdgeChange,
	type Node,
	Position,
	applyEdgeChanges,
} from "@xyflow/react";
import { Window } from "happy-dom";
import { act, useState } from "react";
import type { BoardRuntime } from "./board-runtime";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(window.SVGGraphicsElement.prototype, {
	getBBox: () => ({ x: 0, y: -15, width: 80, height: 20 }),
});
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	SVGElement: window.SVGElement,
	Element: window.Element,
	Node: window.Node,
	ResizeObserver: window.ResizeObserver,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(performance.now()), 0),
	cancelAnimationFrame: clearTimeout,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const { createRoot } = await import("react-dom/client");
const { BoardRuntimeProvider, useBoardRuntime } = await import(
	"./board-runtime"
);
const { StandaloneBoardHost } = await import("./standalone-board-host");
const roots: ReturnType<typeof createRoot>[] = [];
afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.replaceChildren();
});
afterAll(() => {
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	window.happyDOM.abort();
});

const nodes: Node[] = [0, 1].map((index) => ({
	id: String(index),
	position: { x: index * 300, y: index * 100 },
	width: 100,
	height: 80,
	measured: { width: 100, height: 80 },
	data: { __flowNativeDom: false },
	handles: [
		{
			id: "out",
			type: "source",
			position: Position.Right,
			x: 95,
			y: 35,
			width: 10,
			height: 10,
		},
		{
			id: "in",
			type: "target",
			position: Position.Left,
			x: -5,
			y: 35,
			width: 10,
			height: 10,
		},
	],
}));
const edge = (id: string, extra: Partial<Edge> = {}): Edge => ({
	id,
	source: "0",
	target: "1",
	sourceHandle: "out",
	targetHandle: "in",
	type: "straight",
	...extra,
});
async function mount(edges: Edge[], extra = {}) {
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	const changes: EdgeChange[][] = [];
	let runtime: BoardRuntime | undefined;
	function Board() {
		runtime = useBoardRuntime();
		const [currentEdges, setEdges] = useState(edges);
		return (
			<StandaloneBoardHost
				nodes={nodes}
				edges={currentEdges}
				onEdgesChange={(next) => {
					changes.push(next);
					setEdges((previous) => applyEdgeChanges(next, previous));
				}}
				{...extra}
			/>
		);
	}
	await act(async () =>
		root.render(
			<BoardRuntimeProvider>
				<Board />
			</BoardRuntimeProvider>,
		),
	);
	if (!runtime) throw new Error("Missing board runtime");
	return { container, runtime, changes };
}
async function key(element: Element, key: string, modifiers = {}) {
	await act(async () =>
		element.dispatchEvent(
			new window.KeyboardEvent("keydown", {
				key,
				bubbles: true,
				cancelable: true,
				...modifiers,
			}) as unknown as KeyboardEvent,
		),
	);
}
function richNode(
	container: ReturnType<typeof window.document.createElement>,
	id: string,
) {
	const node = container.querySelector(
		`[data-board-rich-node][data-id="${id}"]`,
	);
	if (!(node instanceof window.HTMLElement))
		throw new Error(`Missing rich node ${id}`);
	return node;
}
function nodeInput(node: ReturnType<typeof window.document.createElement>) {
	const input = node.querySelector("input");
	if (!input) throw new Error("Missing rich node input");
	return input;
}

test("generic edge labels retain SVG position, styling, and configurable background without RF context", async () => {
	const { container } = await mount([
		edge("label", {
			label: "Retry branch",
			labelStyle: { fill: "red" },
			labelBgStyle: { fill: "blue" },
			labelBgPadding: [6, 8],
			labelBgBorderRadius: 4,
		}),
		edge("plain", { label: "No background", labelShowBg: false }),
	]);
	const label = container.querySelector('[data-id="label"]');
	expect(label?.querySelector(".react-flow__edge-text")?.textContent).toBe(
		"Retry branch",
	);
	expect(
		label?.querySelector(".react-flow__edge-text")?.getAttribute("style"),
	).toContain("fill: red");
	expect(
		label
			?.querySelector(".react-flow__edge-textwrapper")
			?.getAttribute("transform"),
	).toBe("translate(160 80)");
	const background = label?.querySelector(".react-flow__edge-textbg");
	expect(background?.getAttribute("width")).toBe("92");
	expect(background?.getAttribute("height")).toBe("36");
	expect(background?.getAttribute("rx")).toBe("4");
	expect(
		container.querySelector('[data-id="plain"] .react-flow__edge-textbg'),
	).toBeNull();
	expect(container.querySelector(".react-flow__renderer")).toBeNull();
});

test("focused edge Enter, additive Space, and Escape update controlled selection", async () => {
	const { container, runtime, changes } = await mount([edge("a"), edge("b")]);
	const a = container.querySelector('[data-id="a"]') as unknown as SVGGElement;
	const b = container.querySelector('[data-id="b"]') as unknown as SVGGElement;
	await key(a, "Enter");
	expect(
		runtime.flow
			.getEdges()
			.filter((item) => item.selected)
			.map((item) => item.id),
	).toEqual(["a"]);
	await key(b, " ", { ctrlKey: true });
	expect(
		runtime.flow
			.getEdges()
			.filter((item) => item.selected)
			.map((item) => item.id),
	).toEqual(["a", "b"]);
	await key(b, "Escape");
	expect(
		runtime.flow
			.getEdges()
			.filter((item) => item.selected)
			.map((item) => item.id),
	).toEqual(["a"]);
	expect(changes).toHaveLength(3);
	await key(b, "Enter");
	expect(
		runtime.flow
			.getEdges()
			.filter((item) => item.selected)
			.map((item) => item.id),
	).toEqual(["b"]);
});

test("edge selection respects locks and leaves nested custom controls to their component", async () => {
	const { container, changes } = await mount(
		[
			edge("locked", { selectable: false }),
			edge("unfocusable", { focusable: false }),
			edge("custom", { type: "custom" }),
		],
		{
			edgeTypes: {
				custom: () => (
					<foreignObject>
						<button type="button">Edge action</button>
					</foreignObject>
				),
			},
		},
	);
	await key(
		container.querySelector('[data-id="locked"]') as unknown as Element,
		"Enter",
	);
	await key(
		container.querySelector('[data-id="custom"] button') as unknown as Element,
		"Enter",
	);
	expect(changes).toEqual([]);
	expect(
		container
			.querySelector('[data-id="unfocusable"]')
			?.hasAttribute("tabindex"),
	).toBe(false);
	expect(
		container.querySelector('[data-id="custom"] .react-flow__edge-path'),
	).toBeNull();
	expect(
		container.querySelector('[data-id="custom"] button')?.textContent,
	).toBe("Edge action");
});

test("camera updates stable wrappers directly and preserves rich editor state", async () => {
	const { container, runtime } = await mount([edge("wire")], {
		nodes: nodes.map((node, index) =>
			index
				? node
				: { ...node, type: "editor", data: { __flowNativeDom: true } },
		),
		nodeTypes: {
			editor: () => (
				<input aria-label="Retained editor" defaultValue="Initial" />
			),
		},
	});
	const wrapper = richNode(container, "0");
	const editor = nodeInput(wrapper);
	editor.value = "Unsaved edit";
	await act(async () => {
		await runtime.setViewport({ x: -20, y: 30, zoom: 0.5 }, { notify: false });
	});
	expect(container.querySelector('[data-board-rich-node][data-id="0"]')).toBe(
		wrapper,
	);
	expect(editor.value).toBe("Unsaved edit");
	expect(wrapper.style.transform).toBe(
		"translate(-20px, 30px) scale(0.5) translate(0px, 0px)",
	);
	expect(
		container
			.querySelector('[data-id="wire"]')
			?.parentElement?.getAttribute("style"),
	).toContain("transform: translate(-20px, 30px) scale(0.5)");
	expect(
		container.querySelector(".board-world-overlays")?.getAttribute("style"),
	).toContain("transform: translate(-20px, 30px) scale(0.5)");
	expect(
		container
			.querySelector("[data-board-world-content]")
			?.getAttribute("style"),
	).not.toContain("--board-camera");
	const exported = await mount([edge("export")], { exporting: true });
	await act(async () => {
		await exported.runtime.setViewport(
			{ x: 200, y: 100, zoom: 2 },
			{ notify: false },
		);
	});
	expect(
		exported.container
			.querySelector('[data-id="export"]')
			?.parentElement?.getAttribute("style"),
	).toContain("transform: translate(0px, 0px) scale(1)");
});

test("an edge that gains valid handle IDs registers its camera without replacing node internals", async () => {
	const { container, runtime } = await mount([
		edge("late", { sourceHandle: "missing" }),
	]);
	await act(async () => {
		await runtime.setViewport(
			{ x: -200, y: 90, zoom: 0.25 },
			{ notify: false },
		);
	});
	const source = runtime.flow.getInternalNode("0");
	const target = runtime.flow.getInternalNode("1");
	expect(container.querySelector('[data-id="late"]')).toBeNull();
	await act(async () => runtime.flow.setEdges([edge("late")]));
	expect(runtime.flow.getInternalNode("0")).toBe(source);
	expect(runtime.flow.getInternalNode("1")).toBe(target);
	const svg = container.querySelector('[data-id="late"]')?.parentElement;
	expect(svg?.getAttribute("style")).toContain(
		"transform: translate(-200px, 90px) scale(0.25)",
	);
	await act(async () => {
		await runtime.setViewport({ x: 40, y: 30, zoom: 2 }, { notify: false });
	});
	expect(svg?.getAttribute("style")).toContain(
		"transform: translate(40px, 30px) scale(2)",
	);
	await act(async () =>
		runtime.flow.setEdges([edge("late", { targetHandle: "missing" })]),
	);
	expect(container.querySelector('[data-id="late"]')).toBeNull();
	await act(async () => {
		await runtime.setViewport({ x: 10, y: 15, zoom: 1 }, { notify: false });
	});
	expect(svg?.getAttribute("style")).toContain(
		"transform: translate(40px, 30px) scale(2)",
	);
});

test("selected workflow nodes keep stable parents, focus, and input selection", async () => {
	const visibleNodes = nodes.map((node) => ({
		...node,
		type: "node",
		zIndex: 20,
		data: { __flowNativeDom: true },
	}));
	const { container, runtime } = await mount([], {
		nodes: visibleNodes,
		nodeTypes: {
			node: () => <input aria-label="Draft" defaultValue="Initial" />,
		},
	});
	const first = richNode(container, "0");
	const second = richNode(container, "1");
	const editor = nodeInput(first);
	const initialParent = first.parentElement;
	expect(second.parentElement).toBe(initialParent);
	expect(initialParent?.hasAttribute("data-board-editor-stratum")).toBe(true);
	editor.value = "Unsaved text";
	editor.focus();
	editor.setSelectionRange(2, 7);
	await act(async () =>
		runtime.flow.setNodes((items) =>
			items.map((node) =>
				node.id === "0" ? { ...node, selected: true } : node,
			),
		),
	);
	expect(first.parentElement).toBe(initialParent);
	expect(first.style.zIndex).toBe("1020");
	expect(
		container.querySelector('[data-board-rich-node][data-id="0"] input'),
	).toBe(editor);
	expect(window.document.activeElement).toBe(editor);
	expect([editor.selectionStart, editor.selectionEnd, editor.value]).toEqual([
		2,
		7,
		"Unsaved text",
	]);
	await act(async () =>
		runtime.flow.setNodes((items) =>
			items.map((node) => ({ ...node, selected: false })),
		),
	);
	expect(first.parentElement).toBe(initialParent);
	expect(window.document.activeElement).toBe(editor);
	expect(editor.value).toBe("Unsaved text");
});

test("workflow editors share the untransformed stratum and get layers only while the camera moves", async () => {
	const { container, runtime } = await mount([], {
		nodes: nodes.map((node, index) => ({
			...node,
			type: index ? "comment" : "node",
			zIndex: index ? 1 : 20,
			data: { __flowNativeDom: true },
		})),
		nodeTypes: {
			node: () => <div>Node</div>,
			comment: () => <div>Comment</div>,
		},
	});
	const editor = container.querySelector(
		'[data-board-rich-node][data-id="0"]',
	) as unknown as HTMLElement;
	const comment = container.querySelector(
		'[data-board-rich-node][data-id="1"]',
	) as unknown as HTMLElement;
	const stratum = editor.parentElement;
	expect(stratum?.hasAttribute("data-board-editor-stratum")).toBe(true);
	expect(stratum?.style.transform).toBe("");
	expect(stratum?.style.zIndex).toBe("19");
	expect(comment.parentElement?.hasAttribute("data-board-world-content")).toBe(
		true,
	);
	expect(editor.style.willChange).toBe("");
	await act(async () => {
		await runtime.setViewport({ x: 10, y: 20, zoom: 2 }, { notify: false });
	});
	expect(editor.style.transform).toBe(
		"translate(10px, 20px) scale(2) translate(0px, 0px)",
	);
	expect(editor.style.willChange).toBe("transform");
	expect(comment.style.willChange).toBe("");
	await act(() => new Promise((resolve) => setTimeout(resolve, 200)));
	expect(editor.style.willChange).toBe("");
	expect(editor.parentElement).toBe(stratum);
});

test("media keeps its DOM parent through selected elevation and connection changes", async () => {
	const visibleNodes = nodes.map((node, index) => ({
		...node,
		type: index ? "media" : "node",
		zIndex: index ? 19 : 20,
		data: { __flowNativeDom: true },
	}));
	const { container, runtime } = await mount([], {
		nodes: visibleNodes,
		nodeTypes: {
			node: () => <div>Node</div>,
			media: () => <video muted aria-label="Embedded media" />,
		},
	});
	const media = container.querySelector('[data-id="1"] video');
	const wrapper = media?.closest(".react-flow__node");
	const parent = wrapper?.parentElement;
	expect(wrapper).toBeDefined();
	const moved: unknown[] = [];
	if (!parent) throw new Error("Missing media plane");
	const insert = parent.insertBefore.bind(parent);
	parent.insertBefore = (node, before) => {
		moved.push(node);
		return insert(node, before);
	};
	await act(async () =>
		runtime.flow.setNodes((items) =>
			items.map((node) =>
				node.id === "1" ? { ...node, selected: true } : node,
			),
		),
	);
	expect(media?.closest(".react-flow__node")).toBe(wrapper);
	expect(wrapper?.getAttribute("style")).toContain("z-index: 1019");
	await act(async () => runtime.flow.setEdges([edge("added")]));
	expect(container.querySelector('[data-id="1"] video')).toBe(media);
	expect(media?.closest(".react-flow__node")).toBe(wrapper);
	expect(moved).not.toContain(wrapper);
});

test("small ordinary editor sets stay warm while larger populations cull distant wrappers", async () => {
	for (const count of [256, 257]) {
		const editorNodes: Node[] = Array.from({ length: count }, (_, index) => ({
			id: `editor-${index}`,
			type: "node",
			position: { x: index ? 10000 + index * 120 : 0, y: 0 },
			width: 100,
			height: 80,
			measured: { width: 100, height: 80 },
			selected: count > 256 && index === count - 1,
			data: { __flowNativeDom: true },
		}));
		const { container, runtime } = await mount([], { nodes: editorNodes });
		await act(async () => runtime.configure({ width: 1000, height: 800 }));
		expect(container.querySelectorAll("[data-board-rich-node]").length).toBe(
			count <= 256 ? 256 : 2,
		);
		if (count > 256) {
			expect(container.querySelector('[data-id="editor-256"]')).not.toBeNull();
			expect(container.querySelector('[data-id="editor-100"]')).toBeNull();
		}
	}
});
