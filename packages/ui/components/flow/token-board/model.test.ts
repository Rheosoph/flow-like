import { describe, expect, test } from "bun:test";
import type { IBoard, ILayer, IVariable } from "../../../lib/schema/flow/board";
import { ILayerType } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
import { convertJsonToUint8Array } from "../../../lib/uint8";
import {
	type ITokenItem,
	buildFolderTree,
	buildUsageIndex,
	folderPaths,
	matchesFunction,
	matchesVariable,
	parseTokenQuery,
	resolveVariableScope,
} from "./model";

const variable = (
	id: string,
	name: string,
	category: string | null,
	extra: Partial<IVariable> = {},
): IVariable =>
	({
		id,
		name,
		category,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		exposed: false,
		secret: false,
		editable: true,
		...extra,
	}) as IVariable;

const item = (
	id: string,
	name: string,
	category: string | null,
	uses = 0,
): ITokenItem => ({
	id,
	name,
	category,
	kind: "variable",
	variable: variable(id, name, category),
	uses,
	scope: "board",
});

const refNode = (name: string, pinName: string, ref: string) => ({
	name,
	pins: {
		p1: { name: pinName, default_value: convertJsonToUint8Array(ref) },
		p2: { name: "other", default_value: null },
	},
});

describe("buildFolderTree", () => {
	test("nests along '/' and counts the whole subtree", () => {
		const root = buildFolderTree([
			item("1", "A", "Feedback/State"),
			item("2", "B", "Feedback/State/Filters"),
			item("3", "C", "Feedback"),
			item("4", "D", null),
		]);

		expect(root.items.map((i) => i.name)).toEqual(["D"]);
		expect(root.children).toHaveLength(1);

		const feedback = root.children[0];
		expect(feedback.name).toBe("Feedback");
		expect(feedback.path).toBe("Feedback");
		expect(feedback.depth).toBe(0);
		// C, plus A and B from the levels below it.
		expect(feedback.total).toBe(3);

		const state = feedback.children[0];
		expect(state.name).toBe("State");
		expect(state.path).toBe("Feedback/State");
		expect(state.depth).toBe(1);
		expect(state.total).toBe(2);

		const filters = state.children[0];
		expect(filters.path).toBe("Feedback/State/Filters");
		expect(filters.depth).toBe(2);
		expect(filters.total).toBe(1);
	});

	test("sorts items inside a folder by name then id", () => {
		const root = buildFolderTree([
			item("z", "beta", "F"),
			item("a", "alpha", "F"),
		]);
		expect(root.children[0].items.map((i) => i.name)).toEqual([
			"alpha",
			"beta",
		]);
	});

	test("folderPaths walks every level depth-first", () => {
		const root = buildFolderTree([
			item("1", "A", "Feedback/State/Filters"),
			item("2", "B", "Submit"),
		]);
		expect(folderPaths(root)).toEqual([
			"Feedback",
			"Feedback/State",
			"Feedback/State/Filters",
			"Submit",
		]);
	});
});

describe("buildUsageIndex", () => {
	test("counts Get/Set nodes per variable and Call nodes per function", () => {
		const board = {
			nodes: {
				n1: refNode("variable_get", "var_ref", "var-a"),
				n2: refNode("variable_set", "var_ref", "var-a"),
				n3: refNode("variable_get", "var_ref", "var-b"),
				n4: refNode("control_call_function", "function_layer_id", "fn-1"),
				n5: refNode("control_call_function", "function_layer_id", "fn-1"),
				n6: { name: "some_other_node", pins: {} },
			},
		} as unknown as IBoard;

		const usage = buildUsageIndex(board);
		expect(usage.variables["var-a"]).toBe(2);
		expect(usage.variables["var-b"]).toBe(1);
		expect(usage.variables["var-c"]).toBeUndefined();
		expect(usage.functions["fn-1"]).toBe(2);
	});

	test("survives a board with no nodes", () => {
		expect(buildUsageIndex(undefined)).toEqual({
			variables: {},
			functions: {},
		});
		expect(buildUsageIndex({} as IBoard)).toEqual({
			variables: {},
			functions: {},
		});
	});
});

describe("parseTokenQuery", () => {
	test("splits predicates from free text", () => {
		const query = parseTokenQuery("type:String is:unused in:State page");
		expect(query.type).toBe("string");
		expect(query.is).toEqual(["unused"]);
		expect(query.in).toBe("state");
		expect(query.text).toEqual(["page"]);
	});

	test("an empty query has no terms", () => {
		expect(parseTokenQuery("   ")).toEqual({
			text: [],
			type: null,
			is: [],
			in: null,
		});
	});
});

describe("matchesVariable", () => {
	const target = variable("v1", "mYSEARCHCLAUSE", "Feedback/State", {
		secret: true,
	});

	test("matches a subsequence of the name", () => {
		expect(
			matchesVariable(target, parseTokenQuery("msc"), 3, "board"),
		).toBeTrue();
		expect(
			matchesVariable(target, parseTokenQuery("zzz"), 3, "board"),
		).toBeFalse();
	});

	test("applies is: and in: predicates", () => {
		expect(
			matchesVariable(target, parseTokenQuery("is:secret"), 3, "board"),
		).toBeTrue();
		expect(
			matchesVariable(target, parseTokenQuery("is:unused"), 3, "board"),
		).toBeFalse();
		expect(
			matchesVariable(target, parseTokenQuery("is:unused"), 0, "board"),
		).toBeTrue();
		expect(
			matchesVariable(target, parseTokenQuery("in:state"), 3, "board"),
		).toBeTrue();
		expect(
			matchesVariable(target, parseTokenQuery("in:submit"), 3, "board"),
		).toBeFalse();
	});

	test("an unknown predicate matches nothing rather than everything", () => {
		expect(
			matchesVariable(target, parseTokenQuery("is:banana"), 3, "board"),
		).toBeFalse();
	});
});

describe("matchesFunction", () => {
	const layer = {
		id: "fn-1",
		name: "deriveModeClause",
		category: "Query",
		type: ILayerType.Function,
		cache: { enabled: true },
	} as unknown as ILayer;

	test("filters on calls, cache and folder", () => {
		expect(matchesFunction(layer, parseTokenQuery("is:cached"), 4)).toBeTrue();
		expect(matchesFunction(layer, parseTokenQuery("is:unused"), 4)).toBeFalse();
		expect(matchesFunction(layer, parseTokenQuery("is:unused"), 0)).toBeTrue();
		expect(matchesFunction(layer, parseTokenQuery("in:query"), 4)).toBeTrue();
	});

	test("type: only ever matches the function pseudo-type", () => {
		expect(
			matchesFunction(layer, parseTokenQuery("type:function"), 4),
		).toBeTrue();
		expect(
			matchesFunction(layer, parseTokenQuery("type:string"), 4),
		).toBeFalse();
	});
});

describe("resolveVariableScope", () => {
	const local = { a: variable("a", "rowIndex", null) };
	const board = { b: variable("b", "CURRENT_SUB", null) };

	test("finds the scope that actually holds the variable", () => {
		expect(resolveVariableScope("a", local, board)).toBe("local");
		expect(resolveVariableScope("b", local, board)).toBe("board");
		expect(resolveVariableScope("zz", local, board)).toBeNull();
	});

	test("a board variable still resolves while a local scope is open", () => {
		// The regression: dropping a board variable on a folder from inside a
		// function used to be tagged local, miss the lookup and be discarded.
		expect(resolveVariableScope("b", local, board)).toBe("board");
	});

	test("survives a function layer with no locals", () => {
		expect(resolveVariableScope("b", undefined, board)).toBe("board");
		expect(resolveVariableScope("b", {}, board)).toBe("board");
	});
});
