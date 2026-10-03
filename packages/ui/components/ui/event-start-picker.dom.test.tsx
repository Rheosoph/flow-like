import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
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
	byRole,
	click,
	installDom,
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
		await typeInto(byRole("combobox", "Filter by flow"), "orders");
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
		const selected = byRole(
			"radio",
			"Order intake: Legacy intake",
		) as HTMLInputElement;
		expect(selected.checked).toBe(true);
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
});
