import { afterAll, afterEach, expect, test } from "bun:test";
import { type HTMLElement as HappyHTMLElement, Window } from "happy-dom";
import { type ReactNode, act, useLayoutEffect } from "react";
import type { BoardRuntime } from "./board-runtime";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
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
const {
	BoardRuntimeProvider,
	LegacyBoardRuntimeScope,
	useBoardRuntime,
	useNodes,
} = await import("./board-runtime");
const {
	BoardNodeIdProvider,
	Handle,
	NodeResizer,
	NodeToolbar,
	ViewportPortal,
	Panel,
	resizeBoardNode,
} = await import("./board-runtime-primitives");
const { Position, ReactFlowProvider } = await import("@xyflow/react");
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

async function mount(children: ReactNode) {
	const host = window.document.createElement("div");
	const world = window.document.createElement("div");
	const screen = window.document.createElement("div");
	const container = window.document.createElement("div");
	host.append(world, screen, container);
	window.document.body.append(host);
	const value: { runtime?: BoardRuntime } = {};
	function Setup() {
		const runtime = useBoardRuntime();
		useLayoutEffect(() => {
			if (!runtime) throw new Error("Missing native runtime");
			value.runtime = runtime;
			runtime.configure({
				domNode: host as unknown as HTMLDivElement,
				viewportElement: world as unknown as HTMLElement,
				overlayElement: screen as unknown as HTMLElement,
				width: 1000,
				height: 800,
				transform: [20, 30, 0.5],
				nodes: [
					{
						id: "n",
						position: { x: 100, y: 200 },
						width: 150,
						height: 80,
						measured: { width: 150, height: 80 },
						selected: true,
						data: { label: "Native editor" },
					},
				],
			});
		}, [runtime]);
		return <BoardNodeIdProvider nodeId="n">{children}</BoardNodeIdProvider>;
	}
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	await act(async () =>
		root.render(
			<BoardRuntimeProvider>
				<Setup />
			</BoardRuntimeProvider>,
		),
	);
	if (!value.runtime) throw new Error("Native runtime did not mount");
	return { host, world, screen, container, runtime: value.runtime };
}

function resizeControl(
	container: ReturnType<typeof window.document.createElement>,
) {
	const control = container.querySelector<HappyHTMLElement>(
		".react-flow__resize-control.bottom.right",
	);
	if (!control) throw new Error("Missing resize control");
	let captured: number | undefined;
	control.setPointerCapture = (id) => {
		captured = id;
	};
	control.hasPointerCapture = (id) => captured === id;
	control.releasePointerCapture = () => {
		captured = undefined;
	};
	return control;
}

async function pointer(
	control: HappyHTMLElement,
	type: string,
	x: number,
	y: number,
) {
	await act(async () => {
		control.dispatchEvent(
			new window.PointerEvent(type, {
				bubbles: true,
				pointerId: 7,
				button: 0,
				clientX: x,
				clientY: y,
			}),
		);
	});
}

test("native handles and portals render without any React Flow provider", async () => {
	let clicked = 0;
	const { container, world, screen } = await mount(
		<>
			<Handle id="pin" type="target" position={Position.Left}>
				<button type="button" onClick={() => clicked++}>
					Edit value
				</button>
			</Handle>
			<ViewportPortal>
				<span data-world="yes">World overlay</span>
			</ViewportPortal>
			<Panel position="bottom-right">
				<span>Screen control</span>
			</Panel>
		</>,
	);
	const handle = container.querySelector<HappyHTMLElement>(
		'[data-handleid="pin"]',
	);
	expect(handle?.getAttribute("data-nodeid")).toBe("n");
	expect(handle?.getAttribute("data-handlepos")).toBe("left");
	expect(handle?.classList.contains("target")).toBe(true);
	expect(handle?.style.pointerEvents).toBe("all");
	await act(async () => container.querySelector("button")?.click());
	expect(clicked).toBe(1);
	expect(world.querySelector("[data-world]")?.textContent).toBe(
		"World overlay",
	);
	expect(screen.textContent).toBe("Screen control");
});

test("independent preview providers do not inherit the surrounding native graph", async () => {
	function NodeNames({ name }: { name: string }) {
		const nodes = useNodes();
		return (
			<span data-scope={name}>{nodes.map((node) => node.id).join(",")}</span>
		);
	}
	const { container } = await mount(
		<>
			<NodeNames name="native" />
			<LegacyBoardRuntimeScope>
				<ReactFlowProvider
					initialNodes={[{ id: "preview", position: { x: 0, y: 0 }, data: {} }]}
				>
					<NodeNames name="preview" />
				</ReactFlowProvider>
			</LegacyBoardRuntimeScope>
		</>,
	);
	expect(container.querySelector('[data-scope="native"]')?.textContent).toBe(
		"n",
	);
	expect(container.querySelector('[data-scope="preview"]')?.textContent).toBe(
		"preview",
	);
});

test("native toolbar stays in screen space while following node bounds and zoom", async () => {
	const { runtime, screen } = await mount(
		<NodeToolbar position={Position.Bottom} offset={10} isVisible>
			<button type="button">Add branch</button>
		</NodeToolbar>,
	);
	const toolbar = screen.querySelector<HappyHTMLElement>(
		".react-flow__node-toolbar",
	);
	expect(toolbar?.style.left).toBe("107.5px");
	expect(toolbar?.style.top).toBe("180px");
	expect(toolbar?.style.transform).toBe("translate(-50%, 0%)");
	await act(async () => {
		await runtime.setViewport({ x: 100, y: -20, zoom: 2 });
	});
	expect(toolbar?.style.left).toBe("450px");
	expect(toolbar?.style.top).toBe("550px");
	expect(toolbar?.style.transform.includes("scale")).toBe(false);
});

test("native resize applies the release position even without an intervening move and commits once", async () => {
	const commits: unknown[] = [];
	const { runtime, container } = await mount(
		<NodeResizer onResizeEnd={(_event, params) => commits.push(params)} />,
	);
	await act(async () => {
		await runtime.setViewport({ x: 20, y: 30, zoom: 2 });
	});
	const control = resizeControl(container);
	await pointer(control, "pointerdown", 400, 500);
	await pointer(control, "pointerup", 440, 520);
	expect(runtime.flow.getNode("n")?.measured).toEqual({
		width: 170,
		height: 90,
	});
	expect(runtime.flow.getNode("n")?.resizing).toBe(false);
	expect(commits).toEqual([{ x: 100, y: 200, width: 170, height: 90 }]);
});

test("native resize restores its preview on Escape or a lock change without committing", async () => {
	let commits = 0;
	const { runtime, container } = await mount(
		<NodeResizer
			onResizeEnd={() => {
				commits++;
			}}
		/>,
	);
	const control = resizeControl(container);
	await pointer(control, "pointerdown", 400, 500);
	await pointer(control, "pointermove", 420, 520);
	expect(runtime.flow.getNode("n")?.width).toBe(190);
	await act(async () =>
		window.dispatchEvent(
			new window.KeyboardEvent("keydown", { key: "Escape" }),
		),
	);
	expect(runtime.flow.getNode("n")?.width).toBe(150);
	await pointer(control, "pointerdown", 400, 500);
	await pointer(control, "pointermove", 420, 520);
	await act(async () => runtime.configure({ nodesDraggable: false }));
	expect(runtime.flow.getNode("n")?.width).toBe(150);
	expect(commits).toBe(0);
});

test("a remote node replacement cancels resize without overwriting its new geometry", async () => {
	let commits = 0;
	const { runtime, container } = await mount(
		<NodeResizer
			onResizeEnd={() => {
				commits++;
			}}
		/>,
	);
	const control = resizeControl(container);
	await pointer(control, "pointerdown", 400, 500);
	await pointer(control, "pointermove", 420, 520);
	await act(async () =>
		runtime.configure({
			nodes: [
				{
					id: "n",
					position: { x: 500, y: 600 },
					width: 300,
					height: 200,
					measured: { width: 300, height: 200 },
					data: { remote: true },
				},
			],
		}),
	);
	await pointer(control, "pointerup", 450, 550);
	expect(runtime.flow.getNode("n")?.position).toEqual({ x: 500, y: 600 });
	expect(runtime.flow.getNode("n")?.width).toBe(300);
	expect(commits).toBe(0);
});

test("resize constraints preserve the opposite corner and optional aspect ratio", () => {
	const start = { x: 10, y: 20, width: 120, height: 60 };
	expect(
		resizeBoardNode(
			start,
			[-1, -1],
			{ x: 200, y: 200 },
			{ minWidth: 30, minHeight: 20 },
		),
	).toEqual({ x: 100, y: 60, width: 30, height: 20 });
	expect(
		resizeBoardNode(
			start,
			[1, 1],
			{ x: 80, y: 10 },
			{ keepAspectRatio: true, maxHeight: 80 },
		),
	).toEqual({ x: 10, y: 20, width: 160, height: 80 });
	expect(
		resizeBoardNode(start, [1, 1], { x: 6, y: 7 }, { snapGrid: [20, 20] }),
	).toEqual({ x: 10, y: 20, width: 130, height: 60 });
});
