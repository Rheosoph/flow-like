import { describe, expect, test } from "bun:test";
import {
	type IBoard,
	type ILayer,
	ILayerType,
	type INode,
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../schema/flow/board";
import {
	anchorPins,
	buildSuggestionContext,
	extractBoardFacts,
	schemaTitle,
} from "./extract";
import { contextOf, expandTransition } from "./facts";
import {
	type BoardFacts,
	MAX_BAG,
	MAX_PRED,
	type TransitionFact,
} from "./types";

interface NodeSpec {
	name?: string;
	x?: number;
	y?: number;
	start?: boolean;
	event?: boolean;
	execIn?: boolean;
	execOuts?: string[];
	dataIns?: string[];
	dataOuts?: string[];
	layer?: string;
	schema?: string;
}

function makePin(
	id: string,
	name: string,
	pinType: IPinType,
	dataType: IVariableType,
	index: number,
	schema?: string,
): IPin {
	return {
		connected_to: [],
		data_type: dataType,
		depends_on: [],
		description: "",
		friendly_name: name,
		id,
		index,
		name,
		pin_type: pinType,
		schema,
		value_type: IValueType.Normal,
	};
}

class BoardBuilder {
	readonly nodes: Record<string, INode> = {};
	readonly layers: Record<string, ILayer> = {};
	readonly refs: Record<string, string> = {};

	node(id: string, spec: NodeSpec = {}): INode {
		const node: INode = {
			id,
			name: spec.name ?? id,
			category: "Tests",
			description: "",
			friendly_name: id,
			coordinates: [spec.x ?? 0, spec.y ?? 0, 0],
			start: spec.start ?? false,
			event_callback: spec.event ?? false,
			layer: spec.layer,
			pins: {},
		};
		const add = (
			name: string,
			pinType: IPinType,
			dataType: IVariableType,
			index: number,
		) => {
			const pinId = `${id}:${name}`;
			node.pins[pinId] = makePin(
				pinId,
				name,
				pinType,
				dataType,
				index,
				dataType === IVariableType.Execution ? undefined : spec.schema,
			);
		};
		let input = 1;
		let output = 1;
		if (spec.execIn ?? true) {
			add("exec_in", IPinType.Input, IVariableType.Execution, input++);
		}
		for (const name of spec.execOuts ?? ["exec_out"]) {
			add(name, IPinType.Output, IVariableType.Execution, output++);
		}
		for (const name of spec.dataIns ?? []) {
			add(name, IPinType.Input, IVariableType.String, input++);
		}
		for (const name of spec.dataOuts ?? []) {
			add(name, IPinType.Output, IVariableType.String, output++);
		}
		this.nodes[id] = node;
		return node;
	}

	pure(id: string, spec: NodeSpec = {}): INode {
		return this.node(id, {
			execIn: false,
			execOuts: [],
			dataIns: ["in"],
			dataOuts: ["out"],
			...spec,
		});
	}

	reroute(id: string, exec = false): INode {
		const type = exec ? IVariableType.Execution : IVariableType.String;
		const node = this.node(id, {
			name: "reroute",
			execIn: false,
			execOuts: [],
		});
		node.pins[`${id}:in`] = makePin(`${id}:in`, "in", IPinType.Input, type, 1);
		node.pins[`${id}:out`] = makePin(
			`${id}:out`,
			"out",
			IPinType.Output,
			type,
			1,
		);
		return node;
	}

	layer(id: string, type: ILayerType, pinNames: string[]): ILayer {
		const layer: ILayer = {
			id,
			name: id,
			type,
			comments: {},
			coordinates: [0, 0, 0],
			nodes: {},
			pins: {},
			variables: {},
		};
		pinNames.forEach((name, index) => {
			const pinId = `${id}:${name}`;
			layer.pins[pinId] = makePin(
				pinId,
				name,
				IPinType.Input,
				IVariableType.String,
				index + 1,
			);
		});
		this.layers[id] = layer;
		return layer;
	}

	connect(from: string, to: string): void {
		const pin = this.findPin(from);
		pin.connected_to = [...pin.connected_to, to];
	}

	wire(fromNode: string, fromPin: string, toNode: string, toPin: string) {
		this.connect(`${fromNode}:${fromPin}`, `${toNode}:${toPin}`);
	}

	exec(fromNode: string, toNode: string, fromPin = "exec_out") {
		this.wire(fromNode, fromPin, toNode, "exec_in");
	}

	build(): IBoard {
		return {
			nodes: this.nodes,
			layers: this.layers,
			refs: this.refs,
			comments: {},
		} as IBoard;
	}

	private findPin(pinId: string): IPin {
		for (const node of Object.values(this.nodes)) {
			if (node.pins[pinId]) return node.pins[pinId];
		}
		for (const layer of Object.values(this.layers)) {
			if (layer.pins[pinId]) return layer.pins[pinId];
			for (const node of Object.values(layer.nodes)) {
				if (node.pins[pinId]) return node.pins[pinId];
			}
		}
		throw new Error(`Test pin ${pinId} does not exist`);
	}
}

function expanded(facts: BoardFacts): TransitionFact[] {
	return facts.transitions.map((transition) =>
		expandTransition(facts, transition),
	);
}

function edgesOf(facts: BoardFacts): string[] {
	return expanded(facts).map(
		(fact) => `${fact.src}.${fact.srcPin}->${fact.dst}.${fact.dstPin}`,
	);
}

function construction(facts: BoardFacts): string[] {
	return facts.types;
}

describe("extractBoardFacts wiring", () => {
	test("folds exec and data reroutes, never emits reroute endpoints", () => {
		const graph = new BoardBuilder();
		graph.node("a", { start: true, execIn: false, dataOuts: ["value"] });
		graph.node("b", { x: 200, dataIns: ["text"] });
		graph.reroute("r1", true);
		graph.reroute("r2");
		graph.reroute("r3");
		graph.connect("a:exec_out", "r1:in");
		graph.connect("r1:out", "b:exec_in");
		graph.connect("a:value", "r2:in");
		graph.connect("r2:out", "r3:in");
		graph.connect("r3:out", "b:text");
		const facts = extractBoardFacts(graph.build());
		expect(edgesOf(facts).sort()).toEqual([
			"a.exec_out->b.exec_in",
			"a.value->b.text",
		]);
		expect(facts.types).toEqual(["a", "b"]);
		expect(
			expanded(facts)
				.map((fact) => fact.kind)
				.sort(),
		).toEqual(["data", "exec"]);
		expect(
			facts.outcomes.every((row) => facts.types[row.src] !== "reroute"),
		).toBe(true);
	});

	test("follows Collapsed and Macro boundary pins", () => {
		const graph = new BoardBuilder();
		graph.node("a", { start: true, execIn: false, dataOuts: ["value"] });
		graph.node("b", { x: 200, dataIns: ["text"], layer: "collapsed" });
		graph.node("c", { x: 400, dataIns: ["text"], layer: "macro" });
		graph.layer("collapsed", ILayerType.Collapsed, ["gate"]);
		graph.layer("macro", ILayerType.Macro, ["gate"]);
		graph.connect("a:value", "collapsed:gate");
		graph.connect("collapsed:gate", "b:text");
		graph.connect("a:exec_out", "macro:gate");
		graph.connect("macro:gate", "c:exec_in");
		expect(edgesOf(extractBoardFacts(graph.build())).sort()).toEqual([
			"a.exec_out->c.exec_in",
			"a.value->b.text",
		]);
	});

	test("Function and Module layer pins are real boundaries", () => {
		const graph = new BoardBuilder();
		graph.node("a", { start: true, execIn: false, dataOuts: ["value"] });
		graph.node("b", { x: 200, dataIns: ["text"], layer: "fn" });
		graph.node("c", { x: 400, layer: "module" });
		graph.layer("fn", ILayerType.Function, ["gate"]);
		graph.layer("module", ILayerType.Module, ["gate"]);
		graph.connect("a:value", "fn:gate");
		graph.connect("fn:gate", "b:text");
		graph.connect("a:exec_out", "module:gate");
		graph.connect("module:gate", "c:exec_in");
		const facts = extractBoardFacts(graph.build());
		expect(facts.transitions).toEqual([]);
		const aRows = facts.outcomes.filter((row) => facts.types[row.src] === "a");
		expect(aRows.map((row) => [row.srcPin, row.connected])).toEqual([
			["exec_out", false],
			["value", false],
		]);
	});

	test("includes legacy nodes that only live in layer.nodes", () => {
		const graph = new BoardBuilder();
		graph.node("a", { start: true, execIn: false });
		const layer = graph.layer("legacy", ILayerType.Collapsed, []);
		const legacy = new BoardBuilder().node("legacy_node", { x: 300 });
		layer.nodes[legacy.id] = legacy;
		graph.exec("a", "legacy_node");
		expect(edgesOf(extractBoardFacts(graph.build()))).toEqual([
			"a.exec_out->legacy_node.exec_in",
		]);
	});

	test("deduplicates per source node, source pin and target node", () => {
		const graph = new BoardBuilder();
		graph.node("event", { start: true, execIn: false });
		graph.pure("value1", { name: "value", x: -100, dataOuts: ["out"] });
		graph.pure("value2", { name: "value", x: -100, y: 100 });
		graph.node("sink", { x: 200, dataIns: ["left", "right"] });
		graph.reroute("r1", true);
		graph.reroute("r2", true);
		graph.exec("event", "sink");
		graph.connect("event:exec_out", "r1:in");
		graph.connect("r1:out", "sink:exec_in");
		graph.connect("event:exec_out", "r2:in");
		graph.connect("r2:out", "sink:exec_in");
		graph.wire("value1", "out", "sink", "left");
		graph.wire("value1", "out", "sink", "right");
		graph.wire("value2", "out", "sink", "right");
		const facts = extractBoardFacts(graph.build());
		const edges = edgesOf(facts);
		expect(edges.filter((edge) => edge.startsWith("event."))).toEqual([
			"event.exec_out->sink.exec_in",
		]);
		expect(edges.filter((edge) => edge.startsWith("value.")).length).toBe(2);
	});

	test("records one outcome per output pin of every non-reroute node", () => {
		const graph = new BoardBuilder();
		graph.node("a", {
			start: true,
			execIn: false,
			execOuts: ["exec_out", "exec_error"],
			dataOuts: ["value", "unused"],
		});
		graph.node("b", { x: 200, dataIns: ["text"] });
		graph.reroute("r");
		graph.exec("a", "b");
		graph.connect("a:value", "r:in");
		graph.connect("r:out", "b:text");
		const facts = extractBoardFacts(graph.build());
		const rows = facts.outcomes.map(
			(row) =>
				`${facts.types[row.src]}.${row.srcPin}:${row.kind}:${row.connected}`,
		);
		expect(rows).toEqual([
			"a.exec_out:exec:true",
			"a.exec_error:exec:false",
			"a.value:data:true",
			"a.unused:data:false",
			"b.exec_out:exec:false",
		]);
	});

	test("caches the analysis per board object", () => {
		const graph = new BoardBuilder();
		graph.node("a", { start: true, execIn: false });
		const board = graph.build();
		expect(extractBoardFacts(board)).toBe(extractBoardFacts(board));
	});
});

describe("construction order", () => {
	function pipeline() {
		const graph = new BoardBuilder();
		graph.node("event", { name: "on_start", start: true, execIn: false });
		graph.node("x1", { name: "fetch", x: 100 });
		graph.node("x2", { name: "log", x: 200 });
		graph.node("x3", {
			name: "transform",
			x: 300,
			dataIns: ["left", "right"],
		});
		graph.node("x4", { name: "log", x: 400 });
		graph.node("x5", { name: "store", x: 500 });
		graph.pure("p1", { name: "zeta_source", x: 250, y: 100 });
		graph.pure("p2", { name: "alpha_source", x: 260, y: 100 });
		graph.exec("event", "x1");
		graph.exec("x1", "x2");
		graph.exec("x2", "x3");
		graph.exec("x3", "x4");
		graph.exec("x4", "x5");
		graph.wire("p1", "out", "x3", "left");
		graph.wire("p2", "out", "x3", "right");
		return graph;
	}

	test("visits exec chains from starts with providers right before their consumer", () => {
		const facts = extractBoardFacts(pipeline().build());
		expect(construction(facts)).toEqual([
			"on_start",
			"fetch",
			"log",
			"zeta_source",
			"alpha_source",
			"transform",
			"store",
		]);
	});

	test("derives pred, ups and bag from the construction order", () => {
		const facts = extractBoardFacts(pipeline().build());
		const fact = expanded(facts).find(
			(candidate) => candidate.src === "transform",
		);
		expect(fact?.dst).toBe("log");
		expect(fact?.pred).toEqual(["log", "fetch", "on_start"]);
		expect(fact?.ups).toEqual(["alpha_source", "zeta_source"]);
		const raw = facts.transitions.find(
			(transition) => facts.types[transition.src] === "transform",
		);
		expect(raw?.bag).toBe(6);
		expect(fact?.bag).toEqual([
			"transform",
			"alpha_source",
			"zeta_source",
			"log",
			"fetch",
			"on_start",
		]);
	});

	test("caps the predecessor chain and stops on exec cycles", () => {
		const graph = new BoardBuilder();
		const names = ["n0", "n1", "n2", "n3", "n4", "n5", "n6"];
		graph.node("n0", { start: true, execIn: false });
		names.slice(1).forEach((name, index) => {
			graph.node(name, { x: (index + 1) * 100 });
			graph.exec(names[index], name);
		});
		graph.node("loop_a", { x: 0, y: 500 });
		graph.node("loop_b", { x: 100, y: 500 });
		graph.exec("loop_a", "loop_b");
		graph.exec("loop_b", "loop_a");
		const facts = expanded(extractBoardFacts(graph.build()));
		const last = facts.find((fact) => fact.src === "n5");
		expect(last?.pred).toEqual(["n4", "n3", "n2", "n1"]);
		expect(last?.pred.length).toBe(MAX_PRED);
		const loop = facts.find((fact) => fact.src === "loop_b");
		expect(loop?.pred).toEqual(["loop_a"]);
		expect(facts.find((fact) => fact.src === "loop_a")?.pred).toEqual([
			"loop_b",
		]);
	});

	test("prefers success over error branches and places unwired nodes last", () => {
		const graph = new BoardBuilder();
		graph.node("orphan", { x: -500 });
		graph.node("request", {
			start: true,
			execIn: false,
			execOuts: ["exec_error", "exec_success"],
		});
		graph.node("handle_error", { x: 200, y: -100 });
		graph.node("handle_ok", { x: 200, y: 100 });
		graph.node("entry", { execIn: false, x: 50, y: 600 });
		graph.node("after_entry", { x: 150, y: 600 });
		graph.exec("request", "handle_error", "exec_error");
		graph.exec("request", "handle_ok", "exec_success");
		graph.exec("entry", "after_entry");
		expect(construction(extractBoardFacts(graph.build()))).toEqual([
			"request",
			"handle_ok",
			"handle_error",
			"entry",
			"after_entry",
			"orphan",
		]);
	});

	test("uses the earliest-placed exec source as predecessor", () => {
		const graph = new BoardBuilder();
		graph.node("event", { name: "on_start", start: true, execIn: false });
		graph.node("first", { x: 100 });
		graph.node("late", { x: 50, y: 900, execIn: false, name: "late_trigger" });
		graph.node("join", { x: 300 });
		graph.exec("event", "first");
		graph.exec("first", "join");
		graph.exec("late", "join");
		const fact = expanded(extractBoardFacts(graph.build())).find(
			(candidate) => candidate.src === "join",
		);
		expect(fact).toBeUndefined();
		const context = buildSuggestionContext(
			graph.build(),
			"join",
			"join:exec_out",
		);
		expect(context?.pred).toEqual(["first", "on_start"]);
	});
});

describe("buildSuggestionContext", () => {
	test("matches the expanded training fact apart from the whole-board bag", () => {
		const graph = new BoardBuilder();
		graph.node("event", { name: "on_start", start: true, execIn: false });
		graph.node("x1", { name: "fetch", x: 100, dataOuts: ["body"] });
		graph.node("x2", {
			name: "parse",
			x: 200,
			dataIns: ["text"],
			dataOuts: ["json"],
			schema: "schema-ref",
		});
		graph.node("x3", { name: "store", x: 300, dataIns: ["value"] });
		graph.pure("p", { name: "config", x: 150, y: 100 });
		graph.refs["schema-ref"] = JSON.stringify({ title: "Payload" });
		graph.exec("event", "x1");
		graph.exec("x1", "x2");
		graph.exec("x2", "x3");
		graph.wire("x1", "body", "x2", "text");
		graph.wire("p", "out", "x2", "text");
		graph.wire("x2", "json", "x3", "value");
		const board = graph.build();
		const facts = extractBoardFacts(board);
		const pins: [string, string][] = [
			["x1", "x1:body"],
			["x2", "x2:json"],
			["x2", "x2:exec_out"],
			["p", "p:out"],
		];
		for (const [nodeId, pinId] of pins) {
			const context = buildSuggestionContext(board, nodeId, pinId);
			const pin = board.nodes[nodeId].pins[pinId];
			const fact = expanded(facts).find(
				(candidate) =>
					candidate.src === board.nodes[nodeId].name &&
					candidate.srcPin === pin.name,
			);
			if (!fact || !context) throw new Error(`Missing fact for ${pinId}`);
			const { bag: factBag, ...factRest } = contextOf(fact);
			const { bag, ...rest } = context;
			expect(rest).toEqual(factRest);
			expect(bag).toEqual([...facts.types].reverse().slice(0, MAX_BAG));
			expect(factBag.length).toBeLessThanOrEqual(bag.length);
		}
		expect(buildSuggestionContext(board, "x2", "x2:json")?.schema).toBe(
			"Payload",
		);
	});

	test("rejects input pins, reroutes and unknown nodes", () => {
		const graph = new BoardBuilder();
		graph.node("a", { start: true, execIn: false });
		graph.node("b", { x: 100 });
		graph.reroute("r", true);
		graph.exec("a", "b");
		const board = graph.build();
		expect(buildSuggestionContext(board, "b", "b:exec_in")).toBeUndefined();
		expect(buildSuggestionContext(board, "r", "r:out")).toBeUndefined();
		expect(buildSuggestionContext(board, "missing", "x")).toBeUndefined();
		expect(buildSuggestionContext(board, "b", "b:exec_out")).toMatchObject({
			kind: "exec",
			src: "b",
			srcPin: "exec_out",
			dataType: "Execution",
			valueType: "Normal",
			pred: ["a"],
			ups: [],
			bag: ["b", "a"],
		});
	});
});

describe("anchorPins", () => {
	test("orders unconnected exec pins by priority, then data pins by index", () => {
		const graph = new BoardBuilder();
		graph.node("n", {
			execOuts: ["exec_error", "custom", "exec_success", "exec_out", "done"],
			dataOuts: ["second", "first"],
		});
		graph.node("m", { x: 200 });
		graph.exec("n", "m", "done");
		const board = graph.build();
		board.nodes.n.pins["n:second"].index = 9;
		board.nodes.n.pins["n:first"].index = 8;
		expect(anchorPins(board, "n")).toEqual([
			{ pinId: "n:exec_out", kind: "exec" },
			{ pinId: "n:exec_success", kind: "exec" },
			{ pinId: "n:custom", kind: "exec" },
			{ pinId: "n:exec_error", kind: "exec" },
			{ pinId: "n:first", kind: "data" },
			{ pinId: "n:second", kind: "data" },
		]);
		expect(anchorPins(board, "missing")).toEqual([]);
	});
});

describe("schemaTitle", () => {
	test("resolves refs and reads a string title without throwing", () => {
		const refs = { ref: JSON.stringify({ title: "Resolved" }) };
		expect(schemaTitle(refs, "ref")).toBe("Resolved");
		expect(schemaTitle(refs, JSON.stringify({ title: "Inline" }))).toBe(
			"Inline",
		);
		expect(schemaTitle(refs, JSON.stringify({ title: 3 }))).toBe("");
		expect(schemaTitle(refs, "{not json")).toBe("");
		expect(schemaTitle(refs, "constructor")).toBe("");
		expect(schemaTitle(undefined, null)).toBe("");
		expect(schemaTitle({}, "null")).toBe("");
	});
});

describe("performance", () => {
	function largeBoard(size: number): IBoard {
		const graph = new BoardBuilder();
		for (let index = 0; index < size; index++) {
			const exec = index % 3 !== 2;
			if (exec) {
				graph.node(`n${index}`, {
					name: `type_${index % 90}`,
					start: index === 0,
					execIn: index !== 0,
					x: index * 10,
					y: (index % 7) * 40,
					dataIns: ["a", "b"],
					dataOuts: ["result"],
				});
			} else {
				graph.pure(`n${index}`, {
					name: `pure_${index % 40}`,
					x: index * 10,
					y: 300,
				});
			}
		}
		let previousExec = "n0";
		for (let index = 1; index < size; index++) {
			if (index % 3 === 2) {
				graph.wire(`n${index}`, "out", `n${index - 1}`, "a");
				continue;
			}
			graph.exec(previousExec, `n${index}`);
			graph.wire(previousExec, "result", `n${index}`, "b");
			previousExec = `n${index}`;
		}
		return graph.build();
	}

	test("extracts a 1000-node board and serves cached contexts quickly", () => {
		const board = largeBoard(1000);
		const started = performance.now();
		const facts = extractBoardFacts(board);
		const extractMs = performance.now() - started;
		expect(facts.transitions.length).toBeGreaterThan(1000);
		const contextStarted = performance.now();
		for (let index = 0; index < 1000; index += 10) {
			buildSuggestionContext(board, `n${index}`, `n${index}:exec_out`);
		}
		const contextMs = (performance.now() - contextStarted) / 100;
		expect(extractMs).toBeLessThan(250);
		expect(contextMs).toBeLessThan(5);
	});
});
