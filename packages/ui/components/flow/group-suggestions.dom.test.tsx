import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act, useState } from "react";
import type { GroupSuggestion } from "../../lib/flow-grouping";
import { IVariableType } from "../../lib/schema/flow/node";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
});

const translate = (
	_key: string,
	fallback: string,
	variables: Record<string, unknown> = {},
) =>
	fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => String(variables[key] ?? ""));
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: translate }),
}));

const { getBezierPath, Position } = await import("@xyflow/react");
let renderedNodes: {
	id: string;
	position: { x: number; y: number };
	width: number;
	height: number;
}[] = [];
const getInternalNode = () => undefined;
mock.module("@xyflow/react", () => ({
	useNodes: () => renderedNodes,
	useReactFlow: () => ({ getInternalNode }),
	useViewport: () => ({ x: 0, y: 0, zoom: 1 }),
	getBezierPath,
	Position,
	Panel: ({ children }: { children: ReactNode }) => <div>{children}</div>,
	ViewportPortal: ({ children }: { children: ReactNode }) => (
		<div>{children}</div>
	),
}));
const { createRoot } = await import("react-dom/client");
const { GroupSuggestionsOverlay } = await import("./group-suggestions");
const roots: ReturnType<typeof createRoot>[] = [];

async function render(element: React.ReactElement) {
	const container = window.document.createElement("div");
	container.className = "react-flow";
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	await act(async () => root.render(element));
	return { root, container };
}

afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.innerHTML = "";
	renderedNodes = [];
});
afterAll(() => mock.restore());

function suggestion(id: string): GroupSuggestion {
	return {
		id,
		label: `Group ${id}`,
		memberIds: [`${id}-1`, `${id}-2`, `${id}-3`],
		nodeCount: 3,
		internalEdgeCount: 2,
		inputCount: 1,
		outputCount: 1,
		bounds: { x: 100, y: 100, width: 300, height: 200 },
		anchor: { x: 175, y: 150 },
		score: 10,
		boundaryPorts: [
			{
				id: `${id}-in`,
				direction: "input",
				label: "Input",
				dataType: IVariableType.String,
				connections: [{ nodeId: "outside-left", pinId: "output" }],
			},
			{
				id: `${id}-out`,
				direction: "output",
				label: "Output",
				dataType: IVariableType.Boolean,
				connections: [{ nodeId: "outside-right", pinId: "input" }],
			},
		],
	};
}

function button(text: string) {
	const result = [...window.document.querySelectorAll("button")].find(
		(element) => element.textContent === text,
	);
	if (!result) throw new Error(`Missing button ${text}`);
	return result;
}

const noAction = () => {};
const callbacks = {
	onSelect: noAction,
	onPreview: noAction,
	onCollapse: noAction,
	onDismiss: noAction,
	onClose: noAction,
};

describe("GroupSuggestionsOverlay", () => {
	test("limits review to three suggestions and changes the reviewed group without collapsing it", async () => {
		const selections: string[] = [];
		let collapses = 0;
		function Harness() {
			const [selectedId, setSelectedId] = useState("a");
			return (
				<GroupSuggestionsOverlay
					{...callbacks}
					suggestions={["a", "b", "c", "d"].map(suggestion)}
					selectedId={selectedId}
					preview={false}
					busy={false}
					onSelect={(id) => {
						selections.push(id);
						setSelectedId(id);
					}}
					onCollapse={() => {
						collapses++;
					}}
				/>
			);
		}
		const { container } = await render(<Harness />);
		expect(container.querySelectorAll("[data-group-outline]")).toHaveLength(3);
		expect(container.textContent).not.toContain("Group d");
		expect(container.querySelector("section")?.textContent).toContain(
			"3 nodes · 2 internal wires · 1 in / 1 out",
		);
		await act(async () => button("2. Group b").click());
		expect(selections).toEqual(["b"]);
		expect(button("2. Group b").getAttribute("aria-pressed")).toBe("true");
		expect(container.querySelector("section")?.textContent).toContain(
			"Group b",
		);
		expect(collapses).toBe(0);
		await act(async () => button("Collapse").click());
		expect(collapses).toBe(1);
	});

	test("outlines and preview follow dragged members without changing the suggestion", async () => {
		const current = suggestion("a");
		renderedNodes = [
			{ id: "a-1", position: { x: 40, y: 60 }, width: 100, height: 80 },
			{ id: "a-2", position: { x: 200, y: 80 }, width: 100, height: 80 },
			{ id: "a-3", position: { x: 120, y: 100 }, width: 16, height: 12 },
		];
		const element = () => (
			<GroupSuggestionsOverlay
				{...callbacks}
				suggestions={[current]}
				preview={true}
				busy={false}
			/>
		);
		const { root, container } = await render(element());
		const outline = () =>
			container.querySelector(
				'[data-group-outline="a"]',
			) as unknown as HTMLElement | null;
		const ghost = () =>
			container.querySelector(
				'[data-group-preview="a"]',
			) as unknown as HTMLElement | null;
		expect(outline()?.style.left).toBe("20px");
		expect(outline()?.style.width).toBe("300px");
		const initialGhostLeft = ghost()?.style.left;
		expect(initialGhostLeft).toBe("120px");
		expect(ghost()?.style.top).toBe("80px");
		expect(ghost()?.style.width).toBe("150px");
		expect(ghost()?.style.height).toBe("43px");
		renderedNodes = renderedNodes.map((node) => ({
			...node,
			position: { ...node.position, x: node.position.x + 100 },
		}));
		await act(async () => root.render(element()));
		expect(outline()?.style.left).toBe("120px");
		expect(outline()?.style.width).toBe("300px");
		expect(ghost()?.style.left).toBe("220px");
		expect(current.memberIds).toEqual(["a-1", "a-2", "a-3"]);
		expect(current.bounds).toEqual({ x: 100, y: 100, width: 300, height: 200 });
	});

	test("moves only badges covered by the review strip below their outlines", async () => {
		const original = window.HTMLElement.prototype.getBoundingClientRect;
		window.HTMLElement.prototype.getBoundingClientRect = function () {
			if (this.tagName === "SECTION")
				return new window.DOMRect(100, 50, 500, 80);
			if (this.tagName === "BUTTON") return new window.DOMRect(0, 0, 160, 24);
			return original.call(this);
		};
		try {
			const upper = suggestion("a");
			const lower = suggestion("b");
			lower.bounds.y = 400;
			const { root } = await render(
				<GroupSuggestionsOverlay
					{...callbacks}
					suggestions={[upper, lower]}
					preview={false}
					busy={false}
				/>,
			);
			expect(button("1. Group a").style.top).toBe("208px");
			expect(button("2. Group b").style.top).toBe("-32px");
			await act(async () =>
				root.render(
					<GroupSuggestionsOverlay
						{...callbacks}
						suggestions={[
							{ ...upper, bounds: { ...upper.bounds, x: 800 } },
							lower,
						]}
						preview={false}
						busy={false}
					/>,
				),
			);
			expect(button("1. Group a").style.top).toBe("-32px");
		} finally {
			window.HTMLElement.prototype.getBoundingClientRect = original;
		}
	});

	test("preview draws typed ports and wires only for known outside endpoints", async () => {
		const current = suggestion("a");
		current.boundaryPorts[0].connections.push({
			nodeId: "missing",
			pinId: "missing",
		});
		let previewed = 0;
		function Harness() {
			const [preview, setPreview] = useState(false);
			return (
				<GroupSuggestionsOverlay
					{...callbacks}
					suggestions={[current]}
					preview={preview}
					busy={false}
					onPreview={() => {
						previewed++;
						setPreview((value) => !value);
					}}
					getPinPosition={(node) =>
						node === "outside-left"
							? { x: 10, y: 25 }
							: node === "outside-right"
								? { x: 700, y: 250 }
								: undefined
					}
				/>
			);
		}
		const { container } = await render(<Harness />);
		expect(container.querySelector("[data-group-preview]")).toBeNull();
		await act(async () => button("Preview").click());
		expect(previewed).toBe(1);
		expect(container.querySelector('[data-group-preview="a"]')).not.toBeNull();
		expect(container.querySelectorAll("[data-group-port]")).toHaveLength(2);
		expect(
			container
				.querySelector('[data-group-port="a-in"]')
				?.getAttribute("data-type"),
		).toBe("String");
		expect(
			container
				.querySelector('[data-group-port="a-out"]')
				?.getAttribute("data-type"),
		).toBe("Boolean");
		const paths = [
			...container.querySelectorAll("[data-group-preview-wires] path"),
		];
		expect(paths).toHaveLength(2);
		expect(paths[0].getAttribute("d")).toStartWith("M10,25");
		expect(paths[1].getAttribute("d")).toEndWith("700,250");
		await act(async () => button("Hide preview").click());
		expect(container.querySelector("[data-group-preview]")).toBeNull();
	});

	test("opening from the toolbar focuses review and restores its opener on exit", async () => {
		const opener = window.document.createElement("button");
		opener.textContent = "Find groups";
		window.document.body.append(opener);
		opener.focus();
		let closes = 0;
		const { root, container } = await render(
			<GroupSuggestionsOverlay
				{...callbacks}
				suggestions={[suggestion("a")]}
				preview={false}
				busy={false}
				onClose={() => {
					closes++;
				}}
			/>,
		);
		const strip = container.querySelector("section");
		if (!strip) throw new Error("Missing review strip");
		expect(window.document.activeElement).toBe(strip);
		strip?.dispatchEvent(
			new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		);
		expect(closes).toBe(1);
		await act(async () => root.render(<div />));
		expect(window.document.activeElement).toBe(opener);
	});

	test("empty review receives focus without trapping it or overriding a later focus change", async () => {
		const opener = window.document.createElement("button");
		const nextControl = window.document.createElement("input");
		window.document.body.append(opener, nextControl);
		opener.focus();
		const { root, container } = await render(
			<GroupSuggestionsOverlay
				{...callbacks}
				suggestions={[]}
				preview={false}
				busy={false}
			/>,
		);
		const strip = container.querySelector("section");
		if (!strip) throw new Error("Missing review strip");
		expect(window.document.activeElement).toBe(strip);
		nextControl.focus();
		await act(async () => root.render(<div />));
		expect(window.document.activeElement).toBe(nextControl);
	});

	test("closing review tolerates an opener removed from the document", async () => {
		const opener = window.document.createElement("button");
		window.document.body.append(opener);
		opener.focus();
		const { root } = await render(
			<GroupSuggestionsOverlay
				{...callbacks}
				suggestions={[]}
				preview={false}
				busy={false}
			/>,
		);
		opener.remove();
		await act(async () => root.render(<div />));
		expect(window.document.activeElement).not.toBe(opener);
	});

	test("Escape exits review without triggering board reset and ignores unrelated dialogs", async () => {
		let closes = 0;
		let boardEscapes = 0;
		const { container, root } = await render(
			<GroupSuggestionsOverlay
				{...callbacks}
				suggestions={[suggestion("a")]}
				preview={false}
				busy={false}
				onClose={() => {
					closes++;
				}}
			/>,
		);
		container.addEventListener("keydown", () => {
			boardEscapes++;
		});
		const escapeEvent = new window.KeyboardEvent("keydown", {
			key: "Escape",
			bubbles: true,
			cancelable: true,
		});
		await act(async () => container.dispatchEvent(escapeEvent));
		expect(closes).toBe(1);
		expect(boardEscapes).toBe(0);
		expect(escapeEvent.defaultPrevented).toBe(true);
		const dialog = window.document.createElement("div");
		dialog.setAttribute("role", "dialog");
		container.append(dialog);
		dialog.dispatchEvent(
			new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		);
		expect(closes).toBe(1);
		const outside = window.document.createElement("input");
		window.document.body.append(outside);
		outside.dispatchEvent(
			new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		);
		expect(closes).toBe(1);
		await act(async () => root.render(<div />));
		container.dispatchEvent(
			new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		);
		expect(closes).toBe(1);
		expect(boardEscapes).toBe(2);
	});

	test("busy review blocks edits while preserving its close action", async () => {
		let actions = 0;
		let closes = 0;
		await render(
			<GroupSuggestionsOverlay
				suggestions={[suggestion("a")]}
				preview={false}
				busy={true}
				onSelect={() => {
					actions++;
				}}
				onPreview={() => {
					actions++;
				}}
				onCollapse={() => {
					actions++;
				}}
				onDismiss={() => {
					actions++;
				}}
				onClose={() => {
					closes++;
				}}
			/>,
		);
		for (const label of ["1. Group a", "Preview", "Collapsing…", "Dismiss"]) {
			expect(button(label).disabled).toBe(true);
			await act(async () => button(label).click());
		}
		expect(actions).toBe(0);
		const close = window.document.querySelector(
			'button[aria-label="Close group suggestions"]',
		) as unknown as HTMLButtonElement | null;
		await act(async () => close?.click());
		expect(closes).toBe(1);
	});

	test("empty suggestions offer a close action without collapse or preview controls", async () => {
		let closes = 0;
		const { container } = await render(
			<GroupSuggestionsOverlay
				{...callbacks}
				suggestions={[]}
				preview={false}
				busy={false}
				onClose={() => {
					closes++;
				}}
			/>,
		);
		expect(container.textContent).toContain(
			"No groups to suggest in this layer.",
		);
		expect(container.querySelectorAll("button")).toHaveLength(1);
		await act(async () => container.querySelector("button")?.click());
		expect(closes).toBe(1);
	});
});
