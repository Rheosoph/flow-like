import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, useState } from "react";
import type { IBoard, INode } from "../../lib/schema/flow/board";
import {
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
} from "../../lib/schema/flow/board";
import type { IBoardSummary } from "../../lib/schema/flow/board-summary";
import type { IBackendState } from "../../state/backend-state";
import type { PageListItem } from "../../state/backend-state/page-state";
import type { IEventMapping } from "../interfaces/interfaces";
import {
	allByRole,
	byRole,
	click,
	installDom,
	keyDown,
	queryByRole,
	settle,
	typeInto,
} from "../settings/devices/testing/dom-harness";
import type {
	EventStartSelection,
	EventStartTarget,
} from "./event-start-picker";

const dom = installDom();
const { EventStartPicker } = await import("./event-start-picker");
const { useBackendStore } = await import("../../state/backend-state");
const initialBackend = useBackendStore.getState().backend;
const clients: QueryClient[] = [];

afterEach(async () => {
	await dom.cleanup();
	for (const client of clients.splice(0)) client.clear();
	useBackendStore.setState({ backend: initialBackend });
});
afterAll(dom.restore);

const mapping: IEventMapping = {
	events_simple: {
		configs: {},
		eventTypes: ["quick_action", "api"],
		defaultEventType: "quick_action",
		useInterfaces: {},
		configInterfaces: {},
		withSink: [],
	},
};

function summary(
	id: string,
	name: string,
	nodeId: string,
	friendlyName: string,
): IBoardSummary {
	return {
		id,
		name,
		description: "",
		stage: IExecutionStage.Dev,
		executionMode: IExecutionMode.Hybrid,
		logLevel: ILogLevel.Info,
		version: [1, 0, 0],
		nodeCount: 1,
		connectionCount: 0,
		variableCount: 0,
		layerCount: 0,
		commentCount: 0,
		pages: [],
		entryNodes: [{ nodeId, friendlyName, nodeType: "events_simple" }],
	};
}

const summaries = [
	summary("orders", "Order intake", "receive", "Receive order"),
	summary("returns", "Returns", "refund", "Refund approved"),
];
const page: PageListItem = {
	appId: "app",
	boardId: "orders",
	pageId: "desk",
	name: "Order desk",
};

const graph = (name: string): IBoard =>
	({
		id: "orders",
		name: "Order intake",
		execution_mode: IExecutionMode.Remote,
		nodes: {
			receive: {
				id: "receive",
				name: "events_simple",
				friendly_name: name,
				start: true,
			} as INode,
		},
	}) as unknown as IBoard;

async function mount(
	options: {
		summaries?: IBoardSummary[];
		pages?: PageListItem[];
		getBoard?: IBackendState["boardState"]["getBoard"];
		selected?: EventStartSelection;
		disabled?: boolean;
		onRowRender?: (key: string) => void;
	} = {},
) {
	const boardCalls: unknown[][] = [];
	useBackendStore.setState({
		backend: {
			boardState: {
				getBoardSummaries: async function getBoardSummaries() {
					return options.summaries ?? summaries;
				},
				getBoard: async function getBoard(
					...args: Parameters<IBackendState["boardState"]["getBoard"]>
				) {
					boardCalls.push(args);
					return options.getBoard
						? options.getBoard(...args)
						: graph("Receive order");
				},
			},
			pageState: {
				getPages: async function getPages() {
					return options.pages ?? [page];
				},
			},
		} as unknown as IBackendState,
	});
	const client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	clients.push(client);
	const selections: EventStartTarget[] = [];
	function Controlled() {
		const [selected, setSelected] = useState<EventStartSelection>(
			options.selected ?? { boardId: "orders", nodeId: "receive" },
		);
		return (
			<EventStartPicker
				appId="app"
				eventConfig={mapping}
				selected={selected}
				disabled={options.disabled}
				onRowRender={options.onRowRender}
				onSelect={(target) => {
					selections.push(target);
					setSelected(target);
				}}
			/>
		);
	}
	const view = await dom.render(
		<QueryClientProvider client={client}>
			<Controlled />
		</QueryClientProvider>,
	);
	for (let i = 0; i < 4; i++) await settle();
	return { ...view, boardCalls, selections };
}

async function chooseFlow(name: string) {
	await click(byRole("combobox", /^Filter by flow/));
	await click(byRole("option", name));
}

const radios = () => allByRole("radio");
const labelOf = (key: string) => {
	const [, flow, node] = key.split(":");
	return `Flow ${flow?.replace("flow-", "")}: Refund ${node?.slice(1)}`;
};
const search = () => byRole("textbox", "Search start nodes, pages and flows");

function volume(flowCount: number, perFlow: number): IBoardSummary[] {
	return Array.from({ length: flowCount }, (_, flow) => ({
		...summary(`flow-${flow}`, `Flow ${flow}`, "unused", ""),
		entryNodes: Array.from({ length: perFlow }, (_, node) => ({
			nodeId: `n${flow * perFlow + node}`,
			nodeType: "events_simple",
			friendlyName: `Refund ${flow * perFlow + node}`,
		})),
	}));
}

describe("Event start picker", () => {
	test("searches across flows and pages while keeping the chosen start visible below the filter", async () => {
		const view = await mount();
		expect(view.boardCalls).toEqual([]);
		expect(view.container.textContent).toContain("3 in 2 flows");
		await typeInto(
			byRole("textbox", "Search start nodes, pages and flows"),
			"refund",
		);
		expect(queryByRole("radio", "Order intake: Receive order")).toBeNull();
		expect(view.container.textContent).toContain(
			"Chosen Order intake › Receive order",
		);
		await click(byRole("radio", "Returns: Refund approved"));
		expect(view.selections[0]).toMatchObject({
			boardId: "returns",
			nodeId: "refund",
			nodeType: "events_simple",
			name: "Refund approved",
			boardExecutionMode: IExecutionMode.Hybrid,
		});
		await typeInto(
			byRole("textbox", "Search start nodes, pages and flows"),
			"",
		);
		await chooseFlow("Order intake");
		expect(queryByRole("radio", "Returns: Refund approved")).toBeNull();
		expect(view.container.textContent).toContain(
			"Chosen Returns › Refund approved",
		);
		await click(byRole("radio", "Order intake: Order desk"));
		expect(view.selections.at(-1)).toMatchObject({
			boardId: "orders",
			pageId: "desk",
			name: "Order desk",
		});
		expect(view.selections.at(-1)?.nodeId).toBeUndefined();
	});

	test("a failed legacy flow read stays visible and can be retried", async () => {
		let fail = true;
		const legacy = { ...summaries[0], entryNodes: undefined };
		const view = await mount({
			summaries: [legacy, summaries[1]],
			getBoard: async () => {
				if (fail) throw new Error("Read failed");
				return graph("Receive order");
			},
		});
		expect(view.container.textContent).toContain(
			"Could not load start nodes in Order intake.",
		);
		expect(view.container.textContent).toContain("2 loaded");
		expect(queryByRole("radio", "Returns: Refund approved")).not.toBeNull();
		fail = false;
		await click(byRole("button", "Retry"));
		for (let i = 0; i < 3; i++) await settle();
		expect(queryByRole("alert")).toBeNull();
		expect(queryByRole("radio", "Order intake: Receive order")).not.toBeNull();
		expect(view.boardCalls).toHaveLength(2);
	});

	test("uses the selected flow's pinned graph instead of its latest summary", async () => {
		const view = await mount({
			selected: {
				boardId: "orders",
				nodeId: "receive",
				boardVersion: [0, 9, 0],
			},
			getBoard: async () => graph("Legacy intake"),
		});
		expect(view.boardCalls).toEqual([["app", "orders", [0, 9, 0]]]);
		expect(queryByRole("radio", "Order intake: Receive order")).toBeNull();
		const selected = byRole("radio", "Order intake: Legacy intake");
		expect(selected.getAttribute("aria-checked")).toBe("true");
		expect(view.container.textContent).toContain(
			"Chosen Order intake › Legacy intake",
		);
	});

	test("custom start nodes remain available without a built-in event mapping", async () => {
		const custom = {
			...summaries[0],
			entryNodes: [
				{
					nodeId: "custom",
					nodeType: "vendor_custom_start",
					friendlyName: "Sensor reading",
				},
			],
		};
		const view = await mount({ summaries: [custom], pages: [] });
		await click(byRole("radio", "Order intake: Sensor reading"));
		expect(view.selections[0]).toMatchObject({
			boardId: "orders",
			nodeId: "custom",
			nodeType: "vendor_custom_start",
			name: "Sensor reading",
		});
		expect(view.container.textContent).toContain("Custom start");
	});

	test("unavailable pages remain listed and cannot be chosen", async () => {
		const view = await mount({ pages: [{ ...page, unavailable: true }] });
		const choice = byRole(
			"radio",
			"Order intake: Order desk",
		) as HTMLInputElement;
		expect(choice.disabled).toBe(true);
		expect(view.container.textContent).toContain("Page unavailable");
		await click(choice);
		expect(view.selections).toEqual([]);
	});

	test("counts name the flow plural correctly", async () => {
		const view = await mount({ summaries: [summaries[0]], pages: [] });
		expect(view.container.textContent).toContain("1 in 1 flow");
		expect(view.container.textContent).not.toContain("1 flows");
	});

	test("items without a friendly name sort by name then id, and duplicates in one flow get an id suffix", async () => {
		const bare = (nodeId: string) => ({
			nodeId,
			nodeType: "events_simple",
			friendlyName: "",
		});
		const flow = {
			...summaries[0],
			entryNodes: [bare("zz-b2b2b2"), bare("aa-a1a1a1")],
		};
		const other = {
			...summaries[1],
			entryNodes: [bare("only-one")],
		};
		await mount({
			summaries: [flow, other],
			pages: [],
			selected: { boardId: "none" },
		});
		const names = radios().map((radio) => radio.getAttribute("aria-label"));
		expect(names).toEqual([
			"Order intake: Simple · a1a1a1",
			"Order intake: Simple · b2b2b2",
			"Returns: Simple",
		]);
	});

	test("5,000 start nodes in 300 flows render a capped DOM and stay selectable", async () => {
		const view = await mount({
			summaries: volume(300, 17),
			pages: [],
			selected: { boardId: "none" },
		});
		expect(view.container.textContent).toContain("5100 in 300 flows");
		expect(radios().length).toBeLessThanOrEqual(150);
		expect(radios().length).toBeGreaterThan(0);
		expect(view.container.textContent).toContain("more flows not shown");
		expect(view.boardCalls).toEqual([]);
		await typeInto(search(), "refund 17");
		expect(radios().length).toBeLessThanOrEqual(150);
		expect(radios()[0]?.getAttribute("aria-label")).toContain("Refund 17");
		await click(radios()[0] as HTMLElement);
		expect(view.selections[0]).toMatchObject({ name: "Refund 17" });
		expect(radios()[0]?.getAttribute("aria-checked")).toBe("true");
	});

	test("typing re-renders only the rows whose props changed", async () => {
		const renders = new Map<string, number>();
		await mount({
			summaries: volume(300, 17),
			pages: [],
			selected: { boardId: "none" },
			onRowRender: (key) => renders.set(key, (renders.get(key) ?? 0) + 1),
		});
		expect(renders.size).toBeLessThanOrEqual(150);
		await typeInto(search(), "refund 17");
		const shown = radios().map((radio) => {
			const [flow, name] = (radio.getAttribute("aria-label") ?? "").split(": ");
			return `node:flow-${flow?.replace("Flow ", "")}:n${name?.replace("Refund ", "")}`;
		});
		const before = new Map(renders);
		await typeInto(search(), "refund 170");
		const survivors = radios().filter((radio) =>
			shown.some((key) => radio.getAttribute("aria-label") === labelOf(key)),
		);
		expect(survivors.length).toBeGreaterThan(0);
		const rerendered = shown.reduce(
			(sum, key) => sum + ((renders.get(key) ?? 0) - (before.get(key) ?? 0)),
			0,
		);
		expect(rerendered).toBeLessThanOrEqual(2);
	});

	test("arrow keys move focus without selecting", async () => {
		const view = await mount({
			summaries: volume(2, 5),
			pages: [],
			selected: { boardId: "none" },
		});
		const rows = radios();
		await act(async () => (rows[0] as HTMLElement).focus());
		await keyDown(rows[0] as HTMLElement, "ArrowDown");
		expect(document.activeElement).toBe(rows[1]);
		await keyDown(rows[1] as HTMLElement, "End");
		expect(document.activeElement).toBe(rows.at(-1));
		await keyDown(rows.at(-1) as HTMLElement, "Home");
		expect(document.activeElement).toBe(rows[0]);
		expect(view.selections).toEqual([]);
		expect(
			rows.some((row) => row.getAttribute("aria-checked") === "true"),
		).toBe(false);
		await click(rows[1] as HTMLElement);
		expect(view.selections).toHaveLength(1);
	});

	test("groups show 50 rows and reveal the rest on request", async () => {
		await mount({
			summaries: volume(1, 120),
			pages: [],
			selected: { boardId: "none" },
		});
		expect(radios()).toHaveLength(50);
		await click(byRole("button", "Show 50 more"));
		expect(radios()).toHaveLength(100);
		await click(byRole("button", "Show 20 more"));
		expect(radios()).toHaveLength(120);
		expect(queryByRole("button", /^Show/)).toBeNull();
	});

	test("the flow filter searches hundreds of flows", async () => {
		await mount({
			summaries: volume(300, 17),
			pages: [],
			selected: { boardId: "none" },
		});
		await click(byRole("combobox", /^Filter by flow/));
		expect(allByRole("option").length).toBeLessThanOrEqual(101);
		await typeInto(
			document.querySelector("input[placeholder='Search flows']") as Element,
			"Flow 299",
		);
		await click(byRole("option", "Flow 299"));
		expect(radios()).toHaveLength(17);
		expect(
			radios().every((radio) =>
				radio.getAttribute("aria-label")?.startsWith("Flow 299:"),
			),
		).toBe(true);
	});

	test("legacy flows load their graph on demand instead of all at once", async () => {
		const legacy = Array.from({ length: 30 }, (_, flow) => ({
			...summary(`legacy-${flow}`, `Legacy ${flow}`, "x", "x"),
			entryNodes: undefined,
		}));
		const view = await mount({
			summaries: legacy,
			pages: [],
			selected: { boardId: "legacy-0" },
			getBoard: async (_app, boardId) =>
				({ ...graph("Receive order"), id: boardId }) as IBoard,
		});
		expect(view.boardCalls).toEqual([["app", "legacy-0", undefined]]);
		expect(queryByRole("radio", "Legacy 0: Receive order")).not.toBeNull();
		await click(byRole("button", "Load start nodes in Legacy 7"));
		for (let i = 0; i < 3; i++) await settle();
		expect(view.boardCalls).toHaveLength(2);
		expect(queryByRole("radio", "Legacy 7: Receive order")).not.toBeNull();
		expect(queryByRole("button", "Load start nodes in Legacy 7")).toBeNull();
	});

	test("rows keep an inset focus ring so content-visibility clipping cannot hide it", async () => {
		await mount();
		const row = radios()[0] as HTMLElement;
		expect(row.className).toContain("[content-visibility:auto]");
		expect(row.className).toContain("focus-visible:ring-inset");
		expect(row.className).toContain("focus-visible:ring-2");
	});

	test("a legacy flow with a page still offers to load its start nodes", async () => {
		const legacy = { ...summaries[0], entryNodes: undefined };
		const view = await mount({
			summaries: [legacy],
			selected: { boardId: "none" },
		});
		expect(view.boardCalls).toEqual([]);
		expect(radios().map((radio) => radio.getAttribute("aria-label"))).toEqual([
			"Order intake: Order desk",
		]);
		await click(byRole("button", "Load start nodes in Order intake"));
		for (let i = 0; i < 3; i++) await settle();
		expect(view.boardCalls).toHaveLength(1);
		expect(queryByRole("radio", "Order intake: Receive order")).not.toBeNull();
		expect(queryByRole("radio", "Order intake: Order desk")).not.toBeNull();
		expect(queryByRole("button", /^Load start nodes/)).toBeNull();
	});

	test("a chosen unavailable page does not remove the list from the tab order", async () => {
		await mount({
			pages: [{ ...page, unavailable: true }],
			selected: { boardId: "orders", pageId: "desk" },
		});
		expect(queryByRole("radio", "Order intake: Order desk")).not.toBeNull();
		const tabbable = radios().filter((radio) => radio.tabIndex === 0);
		expect(tabbable).toHaveLength(1);
		expect((tabbable[0] as HTMLButtonElement).disabled).toBe(false);
	});

	test("the flow filter announces the listbox it opens", async () => {
		await mount();
		expect(
			byRole("combobox", /^Filter by flow/).getAttribute("aria-haspopup"),
		).toBe("listbox");
	});

	test("expanded groups consume the shared row budget and reset when the search or flow changes", async () => {
		await mount({
			summaries: volume(4, 120),
			pages: [],
			selected: { boardId: "none" },
		});
		const inFlow = (flow: number) =>
			radios().filter((radio) =>
				radio.getAttribute("aria-label")?.startsWith(`Flow ${flow}:`),
			).length;
		expect(radios()).toHaveLength(150);
		await click(byRole("button", "Show 50 more"));
		expect(inFlow(0)).toBe(100);
		expect(radios()).toHaveLength(150);
		expect(inFlow(2)).toBe(0);
		await typeInto(search(), "Refund");
		await typeInto(search(), "");
		expect(inFlow(0)).toBe(50);
		await click(byRole("button", "Show 50 more"));
		expect(inFlow(0)).toBe(100);
		await chooseFlow("Flow 0");
		expect(radios()).toHaveLength(50);
	});
});
