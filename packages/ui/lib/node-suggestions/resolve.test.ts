import { describe, expect, test } from "bun:test";
import type { INode, IPin } from "../schema/flow/node";
import { IPinType, IValueType, IVariableType } from "../schema/flow/node";
import { resolveGhosts } from "./resolve";
import {
	GHOST_ALTERNATIVES,
	type RankedSuggestion,
	type SuggestionResult,
} from "./types";

const pin = (
	name: string,
	pinType: IPinType,
	dataType: IVariableType,
	index: number,
	extra: Partial<IPin> = {},
): IPin => ({
	id: `${name}-${pinType}-${index}`,
	name,
	friendly_name: name,
	description: "",
	pin_type: pinType,
	data_type: dataType,
	value_type: IValueType.Normal,
	index,
	connected_to: [],
	depends_on: [],
	...extra,
});

const node = (name: string, pins: IPin[]): INode => ({
	id: `${name}-template`,
	name,
	friendly_name: name,
	description: "",
	category: "Test",
	pins: Object.fromEntries(pins.map((entry) => [entry.id, entry])),
});

const execIn = (index = 1) =>
	pin("exec_in", IPinType.Input, IVariableType.Execution, index);
const execOut = (index = 1) =>
	pin("exec_out", IPinType.Output, IVariableType.Execution, index);

const catalog = [
	node("log_info", [
		execIn(),
		pin("message", IPinType.Input, IVariableType.String, 2),
		execOut(),
	]),
	node("string_concat", [
		pin("prefix", IPinType.Input, IVariableType.String, 1),
		pin("suffix", IPinType.Input, IVariableType.String, 2),
		pin("result", IPinType.Output, IVariableType.String, 1),
	]),
	node("math_add", [
		pin("a", IPinType.Input, IVariableType.Integer, 1),
		pin("b", IPinType.Input, IVariableType.Integer, 2),
		pin("sum", IPinType.Output, IVariableType.Integer, 1),
	]),
	node("debug_any", [pin("value", IPinType.Input, IVariableType.Generic, 1)]),
	node("delay", [execIn(), execOut()]),
	node("control_call_function", [
		execIn(1),
		pin("function_layer_id", IPinType.Input, IVariableType.String, 2),
		execOut(),
	]),
];
const catalogByName = new Map(catalog.map((entry) => [entry.name, entry]));

const anchorNode = node("source", [
	execOut(),
	pin("text", IPinType.Output, IVariableType.String, 2),
	pin("count", IPinType.Output, IVariableType.Integer, 3),
]);
const anchorPin = (name: string) => {
	const found = Object.values(anchorNode.pins).find(
		(entry) => entry.name === name,
	);
	if (!found) throw new Error(`fixture has no pin ${name}`);
	return { node: anchorNode, pin: found };
};

const result = (
	candidates: RankedSuggestion[],
	pUnconnected = 0,
): SuggestionResult => ({
	candidates,
	pUnconnected,
	confidence: candidates[0]
		? (1 - pUnconnected) * candidates[0].probability
		: 0,
});

describe("resolveGhosts", () => {
	test("uses the predicted pin when it is compatible", () => {
		const ghosts = resolveGhosts(
			result([{ type: "string_concat", pin: "suffix", probability: 0.6 }]),
			anchorPin("text"),
			catalogByName,
			{},
		);
		expect(ghosts).toHaveLength(1);
		expect(ghosts[0].node).toBe(catalogByName.get("string_concat") as INode);
		expect(ghosts[0].targetPinName).toBe("suffix");
	});

	test("falls back to the first matching pin when the prediction does not fit", () => {
		const incompatible = resolveGhosts(
			result([{ type: "log_info", pin: "exec_in", probability: 0.5 }]),
			anchorPin("text"),
			catalogByName,
			{},
		);
		expect(incompatible[0].targetPinName).toBe("message");

		const unknown = resolveGhosts(
			result([{ type: "string_concat", pin: "missing", probability: 0.5 }]),
			anchorPin("text"),
			catalogByName,
			{},
		);
		expect(unknown[0].targetPinName).toBe("prefix");

		const unpredicted = resolveGhosts(
			result([{ type: "log_info", pin: null, probability: 0.5 }]),
			anchorPin("exec_out"),
			catalogByName,
			{},
		);
		expect(unpredicted[0].targetPinName).toBe("exec_in");
	});

	test("skips candidates without a compatible pin or catalog entry", () => {
		const ghosts = resolveGhosts(
			result([
				{ type: "math_add", pin: "a", probability: 0.4 },
				{ type: "not_in_catalog", pin: null, probability: 0.3 },
				{ type: "delay", pin: "exec_in", probability: 0.2 },
				{ type: "debug_any", pin: "value", probability: 0.1 },
			]),
			anchorPin("text"),
			catalogByName,
			{},
		);
		expect(ghosts.map((ghost) => ghost.node.name)).toEqual(["debug_any"]);
	});

	test("never wires into a pin the node hides", () => {
		const ghosts = resolveGhosts(
			result([{ type: "control_call_function", pin: null, probability: 0.9 }]),
			anchorPin("text"),
			catalogByName,
			{},
		);
		expect(ghosts).toEqual([]);
	});

	test("does not mutate the catalog template", () => {
		const before = structuredClone(catalog);
		resolveGhosts(
			result([
				{ type: "log_info", pin: "message", probability: 0.5 },
				{ type: "string_concat", pin: null, probability: 0.3 },
				{ type: "debug_any", pin: null, probability: 0.2 },
			]),
			anchorPin("text"),
			catalogByName,
			{},
		);
		expect(catalog).toEqual(before);
	});

	test("confidence is the connect probability times the candidate probability", () => {
		const ghosts = resolveGhosts(
			result(
				[
					{ type: "log_info", pin: "exec_in", probability: 0.5 },
					{ type: "delay", pin: "exec_in", probability: 0.25 },
				],
				0.2,
			),
			anchorPin("exec_out"),
			catalogByName,
			{},
		);
		expect(ghosts.map((ghost) => ghost.probability)).toEqual([0.5, 0.25]);
		expect(ghosts[0].confidence).toBeCloseTo(0.4);
		expect(ghosts[1].confidence).toBeCloseTo(0.2);
	});

	test("keeps at most GHOST_ALTERNATIVES ghosts and drops repeated types", () => {
		const repeated = Array.from({ length: GHOST_ALTERNATIVES + 3 }, () => ({
			type: "debug_any",
			pin: null,
			probability: 0.01,
		}));
		expect(
			resolveGhosts(result(repeated), anchorPin("text"), catalogByName, {}),
		).toHaveLength(1);

		const many = new Map(catalogByName);
		const candidates: RankedSuggestion[] = [];
		for (let index = 0; index < GHOST_ALTERNATIVES + 2; index++) {
			const name = `logger_${index}`;
			many.set(name, node(name, [execIn()]));
			candidates.push({ type: name, pin: null, probability: 0.1 });
		}
		const ghosts = resolveGhosts(
			result(candidates),
			anchorPin("exec_out"),
			many,
			{},
		);
		expect(ghosts.map((ghost) => ghost.node.name)).toEqual(
			candidates.slice(0, GHOST_ALTERNATIVES).map((entry) => entry.type),
		);
	});
});
