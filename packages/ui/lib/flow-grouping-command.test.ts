import { describe, expect, test } from "bun:test";
import { evaluateGroupCandidate } from "./flow-grouping";
import { buildGroupSuggestionCommand } from "./flow-grouping-command";
import { type IBoard, type ILayer, ILayerType } from "./schema/flow/board";
import { ICommandType } from "./schema/flow/board/commands/generic-command";
import {
	type INode,
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "./schema/flow/node";

function pin(id: string, direction: IPinType): IPin {
	return {
		id,
		name: direction === IPinType.Input ? "route_in" : "route_out",
		friendly_name: id,
		description: "",
		index: 1,
		pin_type: direction,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
	};
}

function node(id: string, layer?: string): INode {
	return {
		id,
		name: id,
		friendly_name: id,
		category: "Test",
		description: "",
		coordinates: [100, 100, 0],
		layer,
		pins: {
			[`${id}:in`]: pin(`${id}:in`, IPinType.Input),
			[`${id}:out`]: pin(`${id}:out`, IPinType.Output),
		},
	};
}

function layer(id: string): ILayer {
	return {
		id,
		name: id,
		type: ILayerType.Collapsed,
		coordinates: [0, 0, 0],
		nodes: {},
		pins: {},
		variables: {},
		comments: {},
	};
}

function connect(board: IBoard, from: string, to: string) {
	board.nodes[from].pins[`${from}:out`].connected_to.push(`${to}:in`);
	board.nodes[to].pins[`${to}:in`].depends_on.push(`${from}:out`);
}

function fixture(currentLayer?: string) {
	const board = {
		id: "board",
		nodes: Object.fromEntries(
			["source", "first", "second", "sink", "sibling"].map((id) => [
				id,
				node(id, currentLayer),
			]),
		),
		layers: currentLayer ? { [currentLayer]: layer(currentLayer) } : {},
		comments: {},
		variables: {},
		refs: {},
	} as IBoard;
	connect(board, "source", "first");
	connect(board, "source", "sibling");
	connect(board, "first", "second");
	connect(board, "second", "sink");
	const suggestion = evaluateGroupCandidate({ board, currentLayer }, [
		"first",
		"second",
	]);
	if (!suggestion) throw new Error("Expected a valid grouping candidate");
	return { board, suggestion, currentLayer };
}

describe("buildGroupSuggestionCommand", () => {
	test("creates one collapsed layer without editing the board or boundary wires", () => {
		const input = fixture();
		const before = structuredClone(input.board);
		const command = buildGroupSuggestionCommand(input);
		expect(command?.command_type).toBe(ICommandType.UpsertLayer);
		expect(command?.node_ids).toEqual(["first", "second"]);
		expect(command?.layer?.type).toBe(ILayerType.Collapsed);
		expect(command?.layer?.pins).toEqual({});
		expect(command?.layer?.nodes).toEqual({});
		expect(command?.layer?.comments).toEqual({});
		expect(command?.layer?.id).toBeString();
		expect(input.board).toEqual(before);
		expect(buildGroupSuggestionCommand(input)?.layer?.id).not.toBe(
			command?.layer?.id,
		);
	});

	test("keeps the selected region in its current parent layer", () => {
		const input = fixture("parent");
		const command = buildGroupSuggestionCommand(input);
		expect(command?.current_layer).toBe("parent");
		expect(command?.layer?.parent_id).toBe("parent");
		expect(command?.node_ids).toEqual(["first", "second"]);
	});

	test("groups collapsed and macro siblings without selecting their descendants", () => {
		for (const type of [ILayerType.Collapsed, ILayerType.Macro]) {
			const { board } = fixture();
			const first = board.nodes.first;
			board.nodes = Object.fromEntries(
				Object.entries(board.nodes).filter(([id]) => id !== "first"),
			);
			board.nodes.child = node("child", "first");
			board.layers.first = {
				...layer("first"),
				type,
				pins: first.pins,
				coordinates: first.coordinates ?? [0, 0, 0],
			};
			const suggestion = evaluateGroupCandidate({ board }, ["first", "second"]);
			if (!suggestion)
				throw new Error("Expected a candidate with an atomic layer");
			expect(
				buildGroupSuggestionCommand({ board, suggestion })?.node_ids,
			).toEqual(["first", "second"]);
			expect(board.nodes.child.layer).toBe("first");
			board.layers.other = layer("other");
			board.layers.first.parent_id = "other";
			expect(
				buildGroupSuggestionCommand({ board, suggestion }),
			).toBeUndefined();
		}
	});

	test("includes an internal reroute in the same collapse command", () => {
		const { board } = fixture();
		board.nodes.reroute = { ...node("reroute"), auto_reroute: true };
		board.nodes.first.pins["first:out"].connected_to = [];
		board.nodes.second.pins["second:in"].depends_on = [];
		connect(board, "first", "reroute");
		connect(board, "reroute", "second");
		const suggestion = evaluateGroupCandidate({ board }, [
			"first",
			"reroute",
			"second",
		]);
		if (!suggestion) throw new Error("Expected a candidate with its reroute");
		expect(
			buildGroupSuggestionCommand({ board, suggestion })?.node_ids,
		).toEqual(["first", "reroute", "second"]);
	});

	test("accepts coordinate-only changes and recomputes the current mean position", () => {
		const input = fixture();
		input.board.nodes.first.coordinates = [500, 600, 0];
		input.board.nodes.second.coordinates = [750, 700, 0];
		const current = evaluateGroupCandidate(
			{ board: input.board },
			input.suggestion.memberIds,
		);
		if (!current)
			throw new Error("Expected the moved candidate to remain valid");
		expect(current.id).toBe(input.suggestion.id);
		expect(current.anchor).toEqual({ x: 625, y: 650 });
		expect(buildGroupSuggestionCommand(input)?.layer?.coordinates).toEqual([
			625, 650, 0,
		]);
	});

	test("matches the collapse history mean including reroutes and atomic layers", () => {
		const { board } = fixture();
		board.nodes.reroute = {
			...node("reroute"),
			auto_reroute: true,
			coordinates: [200, 20, 0],
		};
		board.nodes.second.coordinates = [500, 260, 0];
		board.nodes.first.pins["first:out"].connected_to = [];
		board.nodes.second.pins["second:in"].depends_on = [];
		connect(board, "first", "reroute");
		connect(board, "reroute", "second");
		board.layers.first = {
			...layer("first"),
			coordinates: [20, 80, 0],
			pins: board.nodes.first.pins,
		};
		board.nodes = Object.fromEntries(
			Object.entries(board.nodes).filter(([id]) => id !== "first"),
		);
		board.nodes.child = {
			...node("child", "first"),
			coordinates: [10000, 10000, 0],
		};
		const suggestion = evaluateGroupCandidate(
			{
				board,
				nodeSizes: new Map([
					["first", [1000, 50]],
					["second", [40, 80]],
					["reroute", [16, 12]],
				]),
			},
			["first", "reroute", "second"],
		);
		if (!suggestion)
			throw new Error("Expected a candidate with a layer and reroute");
		expect(suggestion.anchor).toEqual({ x: 240, y: 120 });
		const command = buildGroupSuggestionCommand({ board, suggestion });
		expect(command?.node_ids).toEqual(["first", "reroute", "second"]);
		expect(command?.layer?.coordinates).toEqual([240, 120, 0]);
	});

	test("uses the preview label and falls back to Group for an empty label", () => {
		const input = fixture();
		input.suggestion.label = "  Prepare message  ";
		expect(buildGroupSuggestionCommand(input)?.layer?.name).toBe(
			"Prepare message",
		);
		input.suggestion.label = " ";
		expect(buildGroupSuggestionCommand(input)?.layer?.name).toBe("Group");
	});

	test("rejects a missing or reparented member and a missing parent", () => {
		const missing = fixture();
		missing.board.nodes = Object.fromEntries(
			Object.entries(missing.board.nodes).filter(([id]) => id !== "second"),
		);
		expect(buildGroupSuggestionCommand(missing)).toBeUndefined();
		const moved = fixture();
		moved.board.layers.other = layer("other");
		moved.board.nodes.second.layer = "other";
		expect(buildGroupSuggestionCommand(moved)).toBeUndefined();
		const parent = fixture("parent");
		parent.board.layers = {};
		expect(buildGroupSuggestionCommand(parent)).toBeUndefined();
	});

	test("rejects a changed boundary even when its port counts stay the same", () => {
		const input = fixture();
		input.board.nodes.replacement = node("replacement");
		input.board.nodes.source.pins["source:out"].connected_to = ["sibling:in"];
		input.board.nodes.first.pins["first:in"].depends_on = [];
		connect(input.board, "replacement", "first");
		const current = evaluateGroupCandidate(
			{ board: input.board },
			input.suggestion.memberIds,
		);
		expect(current?.inputCount).toBe(input.suggestion.inputCount);
		expect(current?.outputCount).toBe(input.suggestion.outputCount);
		expect(buildGroupSuggestionCommand(input)).toBeUndefined();
	});

	test("rejects a function reference newly crossing the candidate boundary", () => {
		const input = fixture();
		input.board.nodes.source.fn_refs = {
			can_reference_fns: true,
			can_be_referenced_by_fns: false,
			fn_refs: ["first"],
		};
		expect(buildGroupSuggestionCommand(input)).toBeUndefined();
	});

	test("rejects altered preview membership and unrelated comments", () => {
		const input = fixture();
		input.board.layers.other = layer("other");
		input.board.comments.note = {
			id: "note",
			comment_type: "Text" as IBoard["comments"][string]["comment_type"],
			content: "Outside the group",
			coordinates: [100, 100, 0],
			timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		};
		for (const id of ["other", "note"]) {
			expect(
				buildGroupSuggestionCommand({
					...input,
					suggestion: {
						...input.suggestion,
						memberIds: [...input.suggestion.memberIds, id],
					},
				}),
			).toBeUndefined();
		}
	});
});
