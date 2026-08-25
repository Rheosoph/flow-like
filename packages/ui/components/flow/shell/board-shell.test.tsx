import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import type { IBoard } from "../../../lib/schema/flow/board";

const window = new Window({ url: "https://localhost" });
// happy-dom's Window does not carry the error constructors its own selector parser
// reaches for, so querying inside this harness throws unless they are patched in.
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	ResizeObserver: class {
		observe() {}
		unobserve() {}
		disconnect() {}
	},
});
// @ts-expect-error — react-dom checks this flag before touching the DOM.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

const { BoardActivityRail } = await import("./board-activity-rail");
const { BoardPane, BoardPanel, usePanelToolbarSlot } = await import(
	"./board-panes"
);
const { BoardInspector } = await import("./board-inspector");

const roots: ReturnType<typeof createRoot>[] = [];

function render(element: React.ReactElement): HTMLElement {
	const container = window.document.createElement(
		"div",
	) as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	const root = createRoot(container);
	roots.push(root);
	act(() => root.render(element));
	return container;
}

afterAll(() => {
	act(() => {
		for (const root of roots) root.unmount();
	});
	mock.restore();
});

describe("board shell surfaces", () => {
	test("the rail labels every entry and reports which view is open", () => {
		let opened = "";
		const container = render(
			createElement(BoardActivityRail, {
				top: [
					{
						id: "variables",
						title: "Variables",
						icon: null,
						active: true,
						onSelect: () => {
							opened = "variables";
						},
					},
					{
						id: "runs",
						title: "Runs",
						icon: null,
						badge: 3,
						onSelect: () => {},
					},
				],
				bottom: [
					{
						id: "flowpilot",
						title: "FlowPilot",
						icon: null,
						onSelect: () => {},
					},
				],
			}),
		);

		const buttons = Array.from(container.querySelectorAll("button"));
		expect(buttons).toHaveLength(3);
		// The dock rendered identically whether a surface was open or not.
		expect(buttons[0].getAttribute("aria-pressed")).toBe("true");
		expect(buttons[1].getAttribute("aria-pressed")).toBe("false");
		expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual([
			"Variables",
			"Runs",
			"FlowPilot",
		]);
		expect(container.textContent).toContain("3");

		act(() => buttons[0].click());
		expect(opened).toBe("variables");
	});

	test("a pane exposes its title and one close affordance", () => {
		let closed = false;
		const container = render(
			createElement(
				BoardPane,
				{
					title: "Inspector",
					onClose: () => {
						closed = true;
					},
				},
				"body",
			),
		);
		expect(container.textContent).toContain("Inspector");
		act(() => container.querySelector("button")?.click());
		expect(closed).toBe(true);
	});

	test("the panel switches tabs and reports its badge", () => {
		let active = "problems";
		const container = render(
			createElement(
				BoardPanel,
				{
					tabs: [
						{
							id: "problems",
							label: "Problems",
							badge: 2,
							badgeTone: "danger",
						},
						{ id: "runs", label: "Runs" },
					],
					active: "problems",
					onSelect: (id: string) => {
						active = id;
					},
					onClose: () => {},
				},
				"body",
			),
		);
		expect(container.textContent).toContain("Problems");
		expect(container.textContent).toContain("2");

		const runsTab = Array.from(container.querySelectorAll("button")).find((b) =>
			b.textContent?.includes("Runs"),
		);
		act(() => runsTab?.click());
		expect(active).toBe("runs");
	});

	test("a view hoists its own toolbar into the panel tab strip", () => {
		function ViewWithToolbar() {
			const slot = usePanelToolbarSlot();
			return slot
				? createPortal(
						createElement("button", { type: "button" }, "Refresh"),
						slot,
					)
				: null;
		}

		const container = render(
			createElement(
				BoardPanel,
				{
					tabs: [{ id: "runs", label: "Runs" }],
					active: "runs",
					onSelect: () => {},
					onClose: () => {},
				},
				createElement(ViewWithToolbar),
			),
		);

		// The control belongs to the view but renders in the strip, so a short
		// panel spends no row on it.
		const strip = container.firstElementChild?.firstElementChild;
		expect(strip?.textContent).toContain("Refresh");
	});

	test("the inspector shows pin types, constraints and scores for one node", () => {
		const board = {
			nodes: {
				n1: {
					id: "n1",
					name: "for_each",
					friendly_name: "For Each",
					category: "control",
					description: "Iterates an array",
					scores: {
						privacy: 9,
						security: 8,
						performance: 4,
						governance: 7,
						reliability: 6,
						cost: 5,
					},
					pins: {
						p1: {
							id: "p1",
							name: "array",
							friendly_name: "Array",
							description: "",
							index: 0,
							pin_type: "Input",
							data_type: "String",
							value_type: "HashSet",
							connected_to: [],
							depends_on: [],
							options: { sensitive: true, step: 2 },
						},
						p2: {
							id: "p2",
							name: "exec_out",
							friendly_name: "Done",
							description: "",
							index: 1,
							pin_type: "Output",
							data_type: "Execution",
							value_type: "Normal",
							connected_to: [],
							depends_on: [],
						},
					},
				},
			},
		} as unknown as IBoard;

		const container = render(
			createElement(BoardInspector, { board, selectedNodeIds: ["n1"] }),
		);

		expect(container.textContent).toContain("For Each");
		expect(container.textContent).toContain("HashSet<String>");
		expect(container.textContent).toContain("Step 2");
		expect(container.textContent).toContain("Not connected");
		expect(container.textContent).toContain("performance");
	});

	test("the inspector says what to do when the selection is not one node", () => {
		const empty = render(
			createElement(BoardInspector, {
				board: { nodes: {} } as unknown as IBoard,
				selectedNodeIds: [],
			}),
		);
		expect(empty.textContent).toContain("Select a node to inspect it.");

		const many = render(
			createElement(BoardInspector, {
				board: { nodes: {} } as unknown as IBoard,
				selectedNodeIds: ["a", "b"],
			}),
		);
		expect(many.textContent).toContain("Select one node to inspect it.");
	});
});
