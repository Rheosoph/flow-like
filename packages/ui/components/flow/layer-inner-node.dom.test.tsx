import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { ComponentProps } from "react";
import type { ILayer } from "../../lib/schema/flow/board";
import { IPinType } from "../../lib/schema/flow/pin";
import { makePin } from "./logs/test-fixtures";

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
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);

const { createRoot } = await import("react-dom/client");
const { ReactFlowProvider } = await import("@xyflow/react");
const { LayerInnerNode, InnerLayerNodeType } = await import(
	"./layer-inner-node"
);
const { FlowNode } = await import("./flow-node");
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

test("a layer pin update resizes its mounted boundary when the runtime hash is unchanged", async () => {
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	const layer = {
		id: "layer",
		name: "Layer",
		pins: {},
		comments: {},
		coordinates: [0, 0, 0],
		nodes: {},
		variables: {},
		type: "Collapsed",
	} as ILayer;
	const props = {
		id: "layer-input",
		data: {
			layer,
			type: InnerLayerNodeType.INPUT,
			appId: "app",
			boardId: "board",
			hash: "same-runtime-hash",
			version: [1, 0, 0],
		},
	} as ComponentProps<typeof LayerInnerNode>;
	const render = async (current: typeof layer) => {
		await act(async () => {
			root.render(
				<ReactFlowProvider>
					<LayerInnerNode {...props} data={{ ...props.data, layer: current }} />
				</ReactFlowProvider>,
			);
		});
	};
	await render(layer);
	const boundary = container.querySelector(".react-flow__node-default");
	if (!(boundary instanceof window.HTMLElement))
		throw new Error("Layer boundary was not rendered");
	const originalHeight = boundary.style.height;
	await render({
		...layer,
		pins: {
			first: makePin("first", IPinType.Input),
			second: makePin("second", IPinType.Input),
		},
	});
	expect(container.querySelector(".react-flow__node-default")).toBe(boundary);
	expect(boundary.style.height).not.toBe(originalHeight);
	expect(boundary.style.height).toContain("30px");
});

test("node rendering observes execution settings without changing the node's runtime hash", () => {
	type Props = ComponentProps<typeof FlowNode>;
	const compare = (
		FlowNode as unknown as {
			compare: (previous: Props, next: Props) => boolean;
		}
	).compare;
	const onExecute: Props["data"]["onExecute"] = async () => {};
	const onRemoteExecute: Props["data"]["onRemoteExecute"] = async () => {};
	const previous = {
		id: "event",
		data: {
			hash: "same-runtime-hash",
			node: { id: "event", pins: {}, start: true },
			executionMode: "Hybrid",
			isOffline: false,
			onExecute,
			onRemoteExecute,
		},
	} as Props;
	expect(compare(previous, { ...previous, data: { ...previous.data } })).toBe(
		true,
	);
	for (const update of [
		{ executionMode: "Remote" },
		{ isOffline: true },
		{ onExecute: async () => {} },
		{ onRemoteExecute: undefined },
	]) {
		expect(
			compare(previous, {
				...previous,
				data: { ...previous.data, ...update },
			} as Props),
		).toBe(false);
	}
});
