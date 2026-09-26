import { afterEach, describe, expect, mock, test } from "bun:test";
import type { InternalNode, Node } from "@xyflow/react";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { type IBoard, ILayerType } from "../lib/schema/flow/board";
import {
	ICommandType,
	type IGenericCommand,
} from "../lib/schema/flow/board/commands/generic-command";
import {
	type INode,
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../lib/schema/flow/node";
import { useGroupSuggestions } from "./use-group-suggestions";

const window = new Window({ url: "https://localhost" });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import("react-dom/client");
const roots: ReturnType<typeof createRoot>[] = [];

afterEach(() => {
	act(() => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.innerHTML = "";
});

function pin(id: string, pinType: IPinType): IPin {
	return {
		id,
		name: id,
		friendly_name: id,
		description: "",
		index: 0,
		pin_type: pinType,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
	};
}

function node(id: string, x: number, y: number): INode {
	return {
		id,
		name: id,
		friendly_name: id,
		category: "Test",
		description: "",
		coordinates: [x, y, 0],
		pins: {
			[`${id}:in`]: pin(`${id}:in`, IPinType.Input),
			[`${id}:out`]: pin(`${id}:out`, IPinType.Output),
		},
	};
}

function connect(board: IBoard, from: string, to: string) {
	board.nodes[from].pins[`${from}:out`].connected_to.push(`${to}:in`);
	board.nodes[to].pins[`${to}:in`].depends_on.push(`${from}:out`);
}

function fixture(
	groups: ReadonlyArray<readonly [string, number]> = [
		["left", 100],
		["right", 1000],
	],
) {
	const board = {
		id: "board",
		nodes: {},
		layers: {},
		comments: {},
		variables: {},
		refs: {},
	} as IBoard;
	for (const [group, y] of groups) {
		for (const [name, x, offset] of [
			["source", -200, 0],
			["a", 100, 0],
			["b", 400, -80],
			["c", 400, 150],
			["d", 700, 0],
			["sink", 1000, 0],
		] as const) {
			const id = `${group}-${name}`;
			board.nodes[id] = node(id, x, y + offset);
		}
		board.nodes[`${group}-source`].start = true;
		board.nodes[`${group}-sink`].event_callback = true;
		for (const [from, to] of [
			["source", "a"],
			["a", "b"],
			["a", "c"],
			["b", "d"],
			["c", "d"],
			["d", "sink"],
		]) {
			connect(board, `${group}-${from}`, `${group}-${to}`);
		}
	}
	return board;
}

type Options = Parameters<typeof useGroupSuggestions>[0];

function mount(overrides: Partial<Options> = {}) {
	const board = overrides.board ?? fixture();
	const executeCommands = mock(async (commands: IGenericCommand[]) => commands);
	const onStale = mock(() => {});
	const getNodes = mock((): Node[] =>
		Object.values(board.nodes).map((entry) => ({
			id: entry.id,
			position: {
				x: entry.coordinates?.[0] ?? 0,
				y: entry.coordinates?.[1] ?? 0,
			},
			data: {},
			measured: { width: 220, height: 100 },
		})),
	);
	let options: Options = {
		board,
		readOnly: false,
		selectedNodeIds: [],
		getNodes,
		getInternalNode: () => undefined,
		executeCommands,
		onStale,
		...overrides,
	};
	let grouping!: ReturnType<typeof useGroupSuggestions>;
	function Harness() {
		grouping = useGroupSuggestions(options);
		return null;
	}
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	act(() => root.render(createElement(Harness)));
	return {
		board,
		executeCommands,
		onStale,
		getNodes,
		get grouping() {
			return grouping;
		},
		update(next: Partial<Options>) {
			options = { ...options, ...next };
			act(() => root.render(createElement(Harness)));
		},
	};
}

function deferred() {
	let resolve!: (value: unknown) => void;
	const promise = new Promise<unknown>((finish) => {
		resolve = finish;
	});
	return { promise, resolve };
}

async function settleOffer() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 210));
	});
}

describe("useGroupSuggestions", () => {
	test("does no detection or board mutation while review is off", async () => {
		const view = mount();
		const before = structuredClone(view.board);
		view.update({ selectedNodeIds: ["left-a", "left-b"] });
		await act(async () => view.grouping.collapse());
		expect(view.grouping.open).toBe(false);
		expect(view.grouping.suggestions).toEqual([]);
		expect(view.grouping.previewMemberIds.size).toBe(0);
		expect(view.getNodes).not.toHaveBeenCalled();
		expect(view.executeCommands).not.toHaveBeenCalled();
		expect(view.board).toEqual(before);
	});

	test("reviews only the multi-selection and can explicitly review the whole layer", () => {
		const selected = ["left-a", "left-b", "left-c", "left-d"];
		const view = mount({ selectedNodeIds: selected });
		act(() => view.grouping.start());
		expect(view.grouping.suggestions).toHaveLength(1);
		expect(view.grouping.suggestions[0].memberIds).toEqual(selected);
		act(() => view.grouping.start(false));
		expect(view.grouping.suggestions).toHaveLength(2);
		expect(view.executeCommands).not.toHaveBeenCalled();
	});

	test("preview, selection, skip and close leave the board unchanged", () => {
		const view = mount();
		const before = structuredClone(view.board);
		act(() => view.grouping.start());
		const first = view.grouping.suggestions[0];
		const second = view.grouping.suggestions[1];
		act(() => view.grouping.togglePreview());
		expect([...view.grouping.previewMemberIds]).toEqual(first.memberIds);
		act(() => view.grouping.select(second.id));
		expect(view.grouping.preview).toBe(true);
		expect(view.grouping.selectedId).toBe(second.id);
		expect([...view.grouping.previewMemberIds]).toEqual(second.memberIds);
		act(() => view.grouping.skip());
		expect(view.grouping.suggestions.map((entry) => entry.id)).toEqual([
			first.id,
		]);
		act(() => view.grouping.close());
		expect(view.grouping.open).toBe(false);
		expect(view.grouping.suggestions).toEqual([]);
		expect(view.executeCommands).not.toHaveBeenCalled();
		expect(view.board).toEqual(before);
	});

	test("dispatches one collapse even if invoked twice before the first render", async () => {
		const waiting = deferred();
		const executeCommands = mock(
			(_commands: IGenericCommand[]) => waiting.promise,
		);
		const view = mount({ executeCommands });
		act(() => view.grouping.start());
		const selected = view.grouping.suggestions[0];
		let first!: Promise<void>;
		let second!: Promise<void>;
		act(() => {
			first = view.grouping.collapse();
			second = view.grouping.collapse();
			view.grouping.skip();
			view.grouping.next();
			view.grouping.findMore();
		});
		expect(executeCommands).toHaveBeenCalledTimes(1);
		expect(view.grouping.busy).toBe(true);
		expect(view.grouping.suggestions).toHaveLength(2);
		const commands = executeCommands.mock.calls[0][0];
		expect(commands).toHaveLength(1);
		expect(commands[0].command_type).toBe(ICommandType.UpsertLayer);
		expect(commands[0].node_ids).toEqual(selected.memberIds);
		await act(async () => {
			waiting.resolve(commands);
			await Promise.all([first, second]);
		});
		expect(view.grouping.busy).toBe(false);
		expect(view.grouping.suggestions).toHaveLength(1);
		expect(view.grouping.suggestions[0].id).not.toBe(selected.id);
	});

	test("keeps a failed collapse available to retry for both undefined and rejected results", async () => {
		for (const failure of [
			async () => undefined,
			async () => {
				throw new Error("Command failed");
			},
		]) {
			const view = mount({ executeCommands: failure });
			act(() => view.grouping.start());
			act(() => view.grouping.togglePreview());
			const selectedId = view.grouping.selectedId;
			await act(async () => view.grouping.collapse());
			expect(view.grouping.selectedId).toBe(selectedId);
			expect(view.grouping.preview).toBe(true);
			expect(view.grouping.busy).toBe(false);
			expect(view.grouping.suggestions).toHaveLength(2);
			view.update({ executeCommands: view.executeCommands });
			await act(async () => view.grouping.collapse());
			expect(view.executeCommands).toHaveBeenCalledTimes(1);
			expect(view.grouping.suggestions).toHaveLength(1);
		}
	});

	test("rejects stale wiring at commit and refreshes review without dispatching", async () => {
		const view = mount();
		act(() => view.grouping.start());
		const candidate = view.grouping.suggestions[0];
		const prefix = candidate.memberIds[0].split("-")[0];
		const source = `${prefix}-source`;
		const target = `${prefix}-a`;
		view.board.nodes.replacement = {
			...node("replacement", -300, 0),
			start: true,
		};
		view.board.nodes[source].pins[`${source}:out`].connected_to = [];
		view.board.nodes[target].pins[`${target}:in`].depends_on = [];
		connect(view.board, "replacement", target);
		await act(async () => view.grouping.collapse());
		expect(view.executeCommands).not.toHaveBeenCalled();
		expect(view.onStale).toHaveBeenCalledTimes(1);
		expect(view.grouping.open).toBe(true);
		expect(view.grouping.preview).toBe(false);
		expect(
			view.grouping.suggestions.some((entry) => entry.id === candidate.id),
		).toBe(false);
	});

	test("revalidates a reviewed membership after graph updates without selecting new regions", () => {
		const view = mount();
		act(() => view.grouping.start());
		const [selected, remaining] = view.grouping.suggestions;
		act(() => view.grouping.togglePreview());
		const board = structuredClone(view.board);
		board.nodes = Object.fromEntries(
			Object.entries(board.nodes).filter(
				([id]) => id !== selected.memberIds[0],
			),
		);
		view.update({ board });
		expect(view.grouping.suggestions.map((entry) => entry.id)).toEqual([
			remaining.id,
		]);
		expect(view.grouping.selectedId).toBe(remaining.id);
		expect(view.grouping.preview).toBe(true);
		expect([...view.grouping.previewMemberIds]).toEqual(remaining.memberIds);
		expect(view.executeCommands).not.toHaveBeenCalled();
	});

	test("steps through suggestions in both directions and wraps", () => {
		const view = mount({
			board: fixture([
				["left", 100],
				["middle", 1000],
				["right", 1900],
			]),
		});
		act(() => view.grouping.start());
		const ids = view.grouping.suggestions.map((entry) => entry.id);
		expect(ids).toHaveLength(3);
		act(() => view.grouping.next());
		expect(view.grouping.selectedId).toBe(ids[1]);
		act(() => view.grouping.next());
		act(() => view.grouping.next());
		expect(view.grouping.selectedId).toBe(ids[0]);
		act(() => view.grouping.previous());
		expect(view.grouping.selectedId).toBe(ids[2]);
	});

	test("skip and collapse advance to the following suggestion and wrap from the last", async () => {
		const view = mount({
			board: fixture([
				["left", 100],
				["middle", 1000],
				["right", 1900],
			]),
		});
		act(() => view.grouping.start());
		const ids = view.grouping.suggestions.map((entry) => entry.id);
		act(() => view.grouping.select(ids[1]));
		act(() => view.grouping.skip());
		expect(view.grouping.selectedId).toBe(ids[2]);
		await act(async () => view.grouping.collapse());
		expect(view.grouping.selectedId).toBe(ids[0]);
		expect(view.grouping.suggestions.map((entry) => entry.id)).toEqual([
			ids[0],
		]);
		expect(view.grouping.summary).toEqual({ collapsed: 1, skipped: 1 });
	});

	test("a collapse whose board refresh lands first still advances past the collapsed group", async () => {
		const waiting = deferred();
		const view = mount({
			board: fixture([
				["left", 100],
				["middle", 1000],
				["right", 1900],
			]),
			executeCommands: () => waiting.promise,
		});
		act(() => view.grouping.start());
		const [first, collapsed, last] = view.grouping.suggestions;
		act(() => view.grouping.select(collapsed.id));
		let pending!: Promise<void>;
		act(() => {
			pending = view.grouping.collapse();
		});
		const refreshed = structuredClone(view.board);
		for (const id of collapsed.memberIds) delete refreshed.nodes[id];
		view.update({ board: refreshed });
		expect(view.grouping.selectedId).toBe(last.id);
		await act(async () => {
			waiting.resolve([]);
			await pending;
		});
		expect(view.grouping.selectedId).toBe(last.id);
		expect(view.grouping.suggestions.map((entry) => entry.id)).toEqual([
			first.id,
			last.id,
		]);
	});

	test("skipped groups stay out of later searches in the same layer only", async () => {
		const view = mount();
		act(() => view.grouping.start());
		const [first, second] = view.grouping.suggestions;
		act(() => view.grouping.skip());
		expect(view.grouping.selectedId).toBe(second.id);
		act(() => view.grouping.findMore());
		expect(view.grouping.suggestions.map((entry) => entry.id)).toEqual([
			second.id,
		]);
		expect(view.grouping.exhausted).toBe(false);
		act(() => view.grouping.skip());
		expect(view.grouping.suggestions).toEqual([]);
		expect(view.grouping.summary).toEqual({ collapsed: 0, skipped: 2 });
		expect(view.grouping.exhausted).toBe(false);
		act(() => view.grouping.findMore());
		expect(view.grouping.exhausted).toBe(true);
		expect(view.grouping.summary).toEqual({ collapsed: 0, skipped: 2 });
		act(() => view.grouping.start());
		expect(view.grouping.suggestions).toEqual([]);
		expect(view.grouping.summary).toEqual({ collapsed: 0, skipped: 0 });
		act(() => view.grouping.close());
		act(() => view.grouping.offerAfterLayout());
		await settleOffer();
		expect(view.grouping.offerCount).toBe(0);
		view.update({ currentLayer: "nested" });
		view.update({ currentLayer: undefined });
		act(() => view.grouping.start());
		expect(view.grouping.suggestions).toEqual([]);
		view.update({ board: { ...view.board, id: "other" } });
		act(() => view.grouping.start());
		expect(view.grouping.suggestions.map((entry) => entry.id).sort()).toEqual(
			[first.id, second.id].sort(),
		);
	});

	test("reports whether review was scoped to the selection", () => {
		const view = mount({
			selectedNodeIds: ["left-source", "left-sink"],
		});
		act(() => view.grouping.start());
		expect(view.grouping.scope).toBe("selection");
		expect(view.grouping.suggestions).toEqual([]);
		expect(view.grouping.exhausted).toBe(true);
		act(() => view.grouping.start(false));
		expect(view.grouping.scope).toBe("layer");
		expect(view.grouping.suggestions).toHaveLength(2);
	});

	test("read-only and board or layer changes close review and clear preview", async () => {
		for (const change of ["readOnly", "board", "layer"] as const) {
			const view = mount();
			act(() => view.grouping.start());
			act(() => view.grouping.togglePreview());
			if (change === "readOnly") view.update({ readOnly: true });
			if (change === "board")
				view.update({ board: { ...view.board, id: "other" } });
			if (change === "layer") view.update({ currentLayer: "nested" });
			expect(view.grouping.open).toBe(false);
			expect(view.grouping.previewMemberIds.size).toBe(0);
			expect(view.grouping.suggestions).toEqual([]);
			if (change === "readOnly") {
				act(() => view.grouping.start());
				await act(async () => view.grouping.collapse());
				expect(view.grouping.open).toBe(false);
			}
			expect(view.executeCommands).not.toHaveBeenCalled();
		}
	});

	test("an old pending command cannot remove a suggestion from a reopened review", async () => {
		const waiting = deferred();
		const view = mount({ executeCommands: () => waiting.promise });
		act(() => view.grouping.start());
		let pending!: Promise<void>;
		act(() => {
			pending = view.grouping.collapse();
			view.grouping.close();
		});
		act(() => view.grouping.start());
		await act(async () => {
			waiting.resolve([]);
			await pending;
		});
		expect(view.grouping.open).toBe(true);
		expect(view.grouping.suggestions).toHaveLength(2);
		expect(view.grouping.busy).toBe(false);
	});

	test("offers review after layout without opening it or applying commands", async () => {
		const view = mount({ selectedNodeIds: ["left-a", "left-b"] });
		act(() => view.grouping.offerAfterLayout());
		expect(view.grouping.offerCount).toBe(0);
		expect(view.grouping.open).toBe(false);
		await settleOffer();
		expect(view.grouping.offerCount).toBe(2);
		expect(view.executeCommands).not.toHaveBeenCalled();
		act(() => view.grouping.start(false));
		expect(view.grouping.offerCount).toBe(0);
		expect(view.grouping.suggestions).toHaveLength(2);
		act(() => view.grouping.close());
		act(() => view.grouping.offerAfterLayout());
		act(() => view.grouping.dismissOffer());
		await settleOffer();
		expect(view.grouping.offerCount).toBe(0);
	});

	test("counts the post-layout offer once instead of on every later edit", async () => {
		const view = mount();
		act(() => view.grouping.offerAfterLayout());
		view.update({ board: structuredClone(view.board) });
		await settleOffer();
		expect(view.grouping.offerCount).toBe(2);
		const searches = view.getNodes.mock.calls.length;
		expect(searches).toBe(1);
		for (let edit = 0; edit < 3; edit++) {
			view.update({ board: structuredClone(view.board) });
			await settleOffer();
		}
		expect(view.getNodes.mock.calls.length).toBe(searches);
		expect(view.grouping.offerCount).toBe(2);
	});

	test("searches from the visible region and reviews what is on screen first", () => {
		for (const [cluster, y] of [
			["left", -100],
			["right", 800],
		] as const) {
			const getFocusBounds = mock(() => ({
				x: -300,
				y,
				width: 1500,
				height: 500,
			}));
			const view = mount({ getFocusBounds });
			act(() => view.grouping.start());
			expect(getFocusBounds).toHaveBeenCalledTimes(1);
			expect(view.grouping.suggestions).toHaveLength(2);
			expect(
				view.grouping.suggestions[0].memberIds.every((id) =>
					id.startsWith(`${cluster}-`),
				),
			).toBe(true);
		}
	});

	test("cancels a delayed offer after switching scope or becoming read-only", async () => {
		const layerView = mount();
		const readOnlyView = mount();
		act(() => {
			layerView.grouping.offerAfterLayout();
			readOnlyView.grouping.offerAfterLayout();
		});
		layerView.update({ currentLayer: "nested" });
		readOnlyView.update({ readOnly: true });
		await settleOffer();
		expect(layerView.grouping.offerCount).toBe(0);
		expect(readOnlyView.grouping.offerCount).toBe(0);
		expect(layerView.getNodes).not.toHaveBeenCalled();
		expect(readOnlyView.getNodes).not.toHaveBeenCalled();
	});

	test("uses absolute handle centers for ordinary and current-layer boundary pins", () => {
		const board = fixture();
		board.layers.nested = {
			id: "nested",
			name: "Nested",
			type: ILayerType.Collapsed,
			coordinates: [0, 0, 0],
			pins: {},
			nodes: {},
			variables: {},
			comments: {},
		};
		const getInternalNode = mock((id: string) =>
			id === "ordinary" || id === "nested-return"
				? ({
						internals: {
							positionAbsolute: { x: 300, y: 400 },
							handleBounds: {
								source: [{ id: "pin", x: 10, y: 20, width: 8, height: 12 }],
							},
						},
					} as unknown as InternalNode)
				: undefined,
		);
		const view = mount({ board, currentLayer: "nested", getInternalNode });
		expect(view.grouping.getPinPosition("ordinary", "pin")).toEqual({
			x: 314,
			y: 426,
		});
		expect(view.grouping.getPinPosition("nested", "pin")).toEqual({
			x: 314,
			y: 426,
		});
		expect(view.grouping.getPinPosition("nested", "missing")).toBeUndefined();
	});
});
