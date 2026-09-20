import { describe, expect, test } from "bun:test";
import {
	COMPLEXITY_LIMITS,
	type IQualityFinding,
	analyzeBoardQuality,
	looksLikeSecretName,
	matchSecretValue,
} from "./board-quality";
import {
	type IBoard,
	type ILayer,
	ILayerType,
	type INode,
	type IPin,
	IPinType,
	IValueType,
	type IVariable,
	IVariableType,
} from "./schema/flow/board";
import { convertJsonToUint8Array } from "./uint8";

interface PinSpec {
	id: string;
	name: string;
	type?: IVariableType;
	direction: IPinType;
	value?: unknown;
	sensitive?: boolean;
}

function pin(spec: PinSpec): IPin {
	return {
		id: spec.id,
		name: spec.name,
		friendly_name: spec.name,
		description: "",
		pin_type: spec.direction,
		data_type: spec.type ?? IVariableType.String,
		value_type: IValueType.Normal,
		default_value:
			spec.value === undefined ? null : convertJsonToUint8Array(spec.value),
		depends_on: [],
		connected_to: [],
		index: 0,
		options: spec.sensitive ? { sensitive: true } : null,
		schema: null,
	};
}

const execIn = (id: string): PinSpec => ({
	id,
	name: "exec_in",
	type: IVariableType.Execution,
	direction: IPinType.Input,
});
const execOut = (id: string, name = "exec_out"): PinSpec => ({
	id,
	name,
	type: IVariableType.Execution,
	direction: IPinType.Output,
});

function node(
	id: string,
	pins: PinSpec[],
	overrides: Partial<INode> = {},
): INode {
	return {
		id,
		name: overrides.name ?? "some_node",
		friendly_name: overrides.friendly_name ?? id,
		description: "",
		category: "test",
		pins: Object.fromEntries(pins.map((spec) => [spec.id, pin(spec)])),
		...overrides,
	} as INode;
}

function layer(
	id: string,
	type: ILayerType,
	pins: PinSpec[],
	overrides: Partial<ILayer> = {},
): ILayer {
	return {
		id,
		name: overrides.name ?? id,
		type,
		coordinates: [0, 0, 0],
		nodes: {},
		pins: Object.fromEntries(pins.map((spec) => [spec.id, pin(spec)])),
		variables: {},
		comments: {},
		parent_id: null,
		...overrides,
	} as ILayer;
}

function board(
	nodes: INode[],
	layers: ILayer[] = [],
	variables: IVariable[] = [],
): IBoard {
	return {
		id: "board",
		name: "Board",
		description: "",
		nodes: Object.fromEntries(nodes.map((entry) => [entry.id, entry])),
		layers: Object.fromEntries(layers.map((entry) => [entry.id, entry])),
		variables: Object.fromEntries(variables.map((entry) => [entry.id, entry])),
		comments: {},
		refs: {},
		version: [0, 0, 1],
		viewport: [0, 0, 1],
		page_ids: [],
	} as unknown as IBoard;
}

/** Wires `from` → `to` on both ends, the way the backend stores an edge. */
function connect(
	owners: Array<INode | ILayer>,
	from: string,
	to: string,
): void {
	const find = (id: string): IPin => {
		for (const owner of owners) {
			const found = owner.pins[id];
			if (found) return found;
		}
		throw new Error(`no pin ${id}`);
	};
	find(from).connected_to.push(to);
	find(to).depends_on.push(from);
}

function ofRule(report: { findings: IQualityFinding[] }, rule: string) {
	return report.findings.filter((finding) => finding.rule === rule);
}

function startNode(id: string, out = `${id}.out`): INode {
	return node(id, [execOut(out)], { name: "events_simple", start: true });
}

describe("dead code", () => {
	test("reports impure nodes no entry point reaches and leaves reached ones alone", () => {
		const start = startNode("start");
		const reached = node("reached", [
			execIn("reached.in"),
			execOut("reached.out"),
		]);
		const orphan = node("orphan", [execIn("orphan.in"), execOut("orphan.out")]);
		connect([start, reached], "start.out", "reached.in");

		const findings = ofRule(
			analyzeBoardQuality(board([start, reached, orphan])),
			"dead-code",
		);
		expect(findings.map((finding) => finding.target.id)).toEqual(["orphan"]);
		expect(findings[0].severity).toBe("warning");
	});

	test("reports only the tail of an unused pure chain", () => {
		const producer = node("producer", [
			{ id: "producer.out", name: "value", direction: IPinType.Output },
		]);
		const tail = node("tail", [
			{ id: "tail.in", name: "value", direction: IPinType.Input },
			{ id: "tail.out", name: "value", direction: IPinType.Output },
		]);
		connect([producer, tail], "producer.out", "tail.in");

		const findings = ofRule(
			analyzeBoardQuality(board([producer, tail])),
			"dead-code",
		);
		expect(findings.map((finding) => finding.target.id)).toEqual(["tail"]);
		expect(findings[0].detail.kind).toBe("unused-result");
	});

	test("a pure node read by a reachable node is live", () => {
		const start = startNode("start");
		const sink = node("sink", [
			execIn("sink.in"),
			{ id: "sink.value", name: "value", direction: IPinType.Input },
		]);
		const producer = node("producer", [
			{ id: "producer.out", name: "value", direction: IPinType.Output },
		]);
		connect([start, sink, producer], "start.out", "sink.in");
		connect([start, sink, producer], "producer.out", "sink.value");

		expect(
			ofRule(analyzeBoardQuality(board([start, sink, producer])), "dead-code"),
		).toHaveLength(0);
	});

	test("execution relays through a collapsed layer boundary", () => {
		const collapsed = layer("collapsed", ILayerType.Collapsed, [
			execIn("collapsed.in"),
		]);
		const start = startNode("start");
		const inner = node("inner", [execIn("inner.in")], { layer: "collapsed" });
		connect([start, collapsed, inner], "start.out", "collapsed.in");
		connect([start, collapsed, inner], "collapsed.in", "inner.in");

		expect(
			ofRule(
				analyzeBoardQuality(board([start, inner], [collapsed])),
				"dead-code",
			),
		).toHaveLength(0);
	});

	test("reports a function nothing calls", () => {
		const fn = layer("fn", ILayerType.Function, [execIn("fn.in")], {
			name: "Helper",
		});
		const body = node("body", [execIn("body.in")], { layer: "fn" });
		connect([fn, body], "fn.in", "body.in");

		const findings = ofRule(
			analyzeBoardQuality(board([body], [fn])),
			"dead-code",
		);
		expect(findings).toHaveLength(1);
		expect(findings[0].target).toMatchObject({ kind: "layer", id: "fn" });
		expect(findings[0].detail.kind).toBe("uncalled-function");
	});
});

describe("missing input", () => {
	test("flags a live node input with neither a connection nor a default", () => {
		const start = startNode("start");
		const sink = node("sink", [
			execIn("sink.in"),
			{ id: "sink.url", name: "url", direction: IPinType.Input },
			{
				id: "sink.method",
				name: "method",
				direction: IPinType.Input,
				value: "GET",
			},
		]);
		connect([start, sink], "start.out", "sink.in");

		const findings = ofRule(
			analyzeBoardQuality(board([start, sink])),
			"missing-input",
		);
		expect(findings).toHaveLength(1);
		expect(findings[0].detail).toEqual({ kind: "missing-input", pin: "url" });
	});

	test("a function parameter satisfies an input inside the function", () => {
		const fn = layer("fn", ILayerType.Function, [
			execIn("fn.in"),
			{ id: "fn.param", name: "param", direction: IPinType.Input },
		]);
		const body = node(
			"body",
			[
				execIn("body.in"),
				{ id: "body.value", name: "value", direction: IPinType.Input },
			],
			{ layer: "fn" },
		);
		connect([fn, body], "fn.in", "body.in");
		connect([fn, body], "fn.param", "body.value");

		expect(
			ofRule(analyzeBoardQuality(board([body], [fn])), "missing-input"),
		).toHaveLength(0);
	});

	test("dead nodes are not double-reported", () => {
		const orphan = node("orphan", [
			execIn("orphan.in"),
			{ id: "orphan.url", name: "url", direction: IPinType.Input },
		]);
		expect(
			ofRule(analyzeBoardQuality(board([orphan])), "missing-input"),
		).toHaveLength(0);
	});
});

describe("function without return", () => {
	test("reports an entry with nothing wired, an exec output nothing reaches, and an unassigned output", () => {
		const fn = layer("fn", ILayerType.Function, [
			execIn("fn.in"),
			execOut("fn.out"),
			{ id: "fn.result", name: "result", direction: IPinType.Output },
		]);
		const kinds = ofRule(analyzeBoardQuality(board([], [fn])), "no-return").map(
			(finding) => finding.detail.kind,
		);
		expect(kinds).toEqual(
			expect.arrayContaining([
				"entry-unwired",
				"exec-never-returns",
				"output-unassigned",
			]),
		);
	});

	test("a fully wired function is clean", () => {
		const fn = layer("fn", ILayerType.Function, [
			execIn("fn.in"),
			execOut("fn.out"),
			{ id: "fn.result", name: "result", direction: IPinType.Output },
		]);
		const body = node(
			"body",
			[
				execIn("body.in"),
				execOut("body.out"),
				{ id: "body.value", name: "value", direction: IPinType.Output },
			],
			{ layer: "fn" },
		);
		connect([fn, body], "fn.in", "body.in");
		connect([fn, body], "body.out", "fn.out");
		connect([fn, body], "body.value", "fn.result");

		expect(
			ofRule(analyzeBoardQuality(board([body], [fn])), "no-return"),
		).toHaveLength(0);
	});
});

describe("cycles", () => {
	test("an execution loop with no exit is an error, with a branch inside a warning", () => {
		const start = startNode("start");
		const a = node("a", [execIn("a.in"), execOut("a.out")]);
		const b = node("b", [execIn("b.in"), execOut("b.out")]);
		connect([start, a, b], "start.out", "a.in");
		connect([start, a, b], "a.out", "b.in");
		connect([start, a, b], "b.out", "a.in");

		const closed = ofRule(analyzeBoardQuality(board([start, a, b])), "cycle");
		expect(closed).toHaveLength(1);
		expect(closed[0].severity).toBe("error");
		expect(closed[0].detail).toMatchObject({ kind: "exec-cycle", exit: false });

		const branch = node("b", [execIn("b.in"), execOut("b.out")], {
			name: "control_branch",
		});
		connect([start, a, branch], "start.out", "a.in");
		connect([start, a, branch], "a.out", "b.in");
		connect([start, a, branch], "b.out", "a.in");
		const guarded = ofRule(
			analyzeBoardQuality(board([start, a, branch])),
			"cycle",
		);
		expect(guarded[0].severity).toBe("warning");
	});

	test("a structured loop body that does not wire back is not a cycle", () => {
		const start = startNode("start");
		const loop = node(
			"loop",
			[
				execIn("loop.in"),
				execOut("loop.body", "body"),
				execOut("loop.done", "done"),
			],
			{ name: "control_for_each" },
		);
		const body = node("body", [execIn("body.in")]);
		const after = node("after", [execIn("after.in")]);
		connect([start, loop, body, after], "start.out", "loop.in");
		connect([start, loop, body, after], "loop.body", "body.in");
		connect([start, loop, body, after], "loop.done", "after.in");

		expect(
			ofRule(analyzeBoardQuality(board([start, loop, body, after])), "cycle"),
		).toHaveLength(0);
	});

	test("circular data dependencies between pure nodes are errors", () => {
		const a = node("a", [
			{ id: "a.in", name: "in", direction: IPinType.Input },
			{ id: "a.out", name: "out", direction: IPinType.Output },
		]);
		const b = node("b", [
			{ id: "b.in", name: "in", direction: IPinType.Input },
			{ id: "b.out", name: "out", direction: IPinType.Output },
		]);
		connect([a, b], "a.out", "b.in");
		connect([a, b], "b.out", "a.in");

		const findings = ofRule(analyzeBoardQuality(board([a, b])), "cycle");
		expect(findings).toHaveLength(1);
		expect(findings[0].detail.kind).toBe("data-cycle");
	});

	test("a function calling itself is flagged as recursion", () => {
		const fn = layer("fn", ILayerType.Function, [execIn("fn.in")], {
			name: "Recurse",
		});
		const call = node(
			"call",
			[
				execIn("call.in"),
				{
					id: "call.fn",
					name: "function_layer_id",
					direction: IPinType.Input,
					value: "fn",
				},
			],
			{ name: "control_call_function", layer: "fn" },
		);
		connect([fn, call], "fn.in", "call.in");

		const findings = ofRule(analyzeBoardQuality(board([call], [fn])), "cycle");
		expect(findings.map((finding) => finding.detail.kind)).toEqual([
			"recursion",
		]);
	});
});

describe("hard-coded secrets", () => {
	test("a literal on a sensitive pin is an error", () => {
		const start = startNode("start");
		const llm = node("llm", [
			execIn("llm.in"),
			{
				id: "llm.key",
				name: "api_key",
				direction: IPinType.Input,
				value: "hunter2hunter2",
				sensitive: true,
			},
		]);
		connect([start, llm], "start.out", "llm.in");

		const findings = ofRule(
			analyzeBoardQuality(board([start, llm])),
			"hardcoded-secret",
		);
		expect(findings).toHaveLength(1);
		expect(findings[0].severity).toBe("error");
		expect(findings[0].detail).toMatchObject({
			kind: "sensitive-literal",
			connected: false,
		});
	});

	test("an empty sensitive default is fine", () => {
		const llm = node("llm", [
			{
				id: "llm.key",
				name: "api_key",
				direction: IPinType.Input,
				value: "",
				sensitive: true,
			},
		]);
		expect(
			ofRule(analyzeBoardQuality(board([llm])), "hardcoded-secret"),
		).toHaveLength(0);
	});

	test("recognizes token shapes anywhere in a literal", () => {
		const http = node("http", [
			{
				id: "http.header",
				name: "headers",
				direction: IPinType.Input,
				value: "Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz0123",
			},
		]);
		const findings = ofRule(
			analyzeBoardQuality(board([http])),
			"hardcoded-secret",
		);
		expect(findings[0].detail).toMatchObject({ kind: "secret-pattern" });
	});

	test("secret-looking names with a plausible literal warn, placeholders do not", () => {
		const real = node("real", [
			{
				id: "real.pw",
				name: "password",
				direction: IPinType.Input,
				value: "correcthorse",
			},
		]);
		const placeholder = node("placeholder", [
			{
				id: "ph.pw",
				name: "password",
				direction: IPinType.Input,
				value: "<your password>",
			},
		]);
		const counter = node("counter", [
			{
				id: "c.max",
				name: "max_tokens",
				direction: IPinType.Input,
				value: "4096tokens",
			},
		]);
		const findings = ofRule(
			analyzeBoardQuality(board([real, placeholder, counter])),
			"hardcoded-secret",
		);
		expect(findings.map((finding) => finding.target.id)).toEqual(["real"]);
		expect(findings[0].severity).toBe("warning");
	});

	test("a non-secret variable that looks like a credential is reported", () => {
		const variable = {
			id: "var",
			name: "openai_api_key",
			data_type: IVariableType.String,
			value_type: IValueType.Normal,
			default_value: convertJsonToUint8Array(
				"sk-abcdefghijklmnopqrstuvwxyz0123",
			),
			secret: false,
			editable: true,
			exposed: false,
		} as IVariable;
		const findings = ofRule(
			analyzeBoardQuality(board([], [], [variable])),
			"hardcoded-secret",
		);
		expect(findings).toHaveLength(1);
		expect(findings[0].target.kind).toBe("variable");
		expect(findings[0].severity).toBe("error");

		const marked = { ...variable, secret: true };
		expect(
			ofRule(analyzeBoardQuality(board([], [], [marked])), "hardcoded-secret"),
		).toHaveLength(0);
	});

	test("name and value heuristics", () => {
		expect(looksLikeSecretName("apiKey")).toBe(true);
		expect(looksLikeSecretName("client_secret")).toBe(true);
		expect(looksLikeSecretName("access_token")).toBe(true);
		expect(looksLikeSecretName("max_tokens")).toBe(false);
		expect(looksLikeSecretName("author")).toBe(false);
		expect(looksLikeSecretName("tokenizer")).toBe(false);
		expect(matchSecretValue("AKIAIOSFODNN7EXAMPLE")).toBe("aws");
		expect(matchSecretValue("ghp_abcdefghijklmnopqrstuvwxyz")).toBe("github");
		expect(matchSecretValue("postgres://admin:pw@db.internal/app")).toBe(
			"url-credentials",
		);
		expect(matchSecretValue("https://example.com/path")).toBeUndefined();
		expect(matchSecretValue("task-abcdefghijklmnopqrstuvwxyz")).toBeUndefined();
	});
});

describe("insecure nodes", () => {
	test("low security scores sort worst first and network grants are informational", () => {
		const start = startNode("start");
		const weak = node("weak", [execIn("weak.in")], {
			scores: {
				security: 2,
				privacy: 8,
				governance: 5,
				performance: 5,
				reliability: 5,
				cost: 5,
			},
		});
		const worst = node("worst", [execIn("worst.in")], {
			scores: {
				security: 1,
				privacy: 3,
				governance: 5,
				performance: 5,
				reliability: 5,
				cost: 5,
			},
		});
		const wasm = node("wasm", [execIn("wasm.in")], {
			wasm: {
				package_id: "pkg",
				permissions: ["NetworkHttp", "Cache"] as never,
			},
		});
		for (const target of [weak, worst, wasm]) {
			connect([start, target], "start.out", `${target.id}.in`);
		}

		const findings = ofRule(
			analyzeBoardQuality(board([start, weak, worst, wasm])),
			"insecure-node",
		);
		expect(findings.map((finding) => finding.target.id)).toEqual([
			"worst",
			"weak",
			"worst",
			"wasm",
		]);
		expect(findings[0].severity).toBe("error");
		expect(findings.at(-1)?.detail).toEqual({
			kind: "wasm-network",
			permissions: ["NetworkHttp"],
		});
	});
});

describe("complexity", () => {
	test("a long event chain over the limit is flagged on its entry node", () => {
		const start = startNode("start");
		const chain: INode[] = [start];
		let previous = "start.out";
		for (let index = 0; index < COMPLEXITY_LIMITS.nodes + 2; index += 1) {
			const step = node(`n${index}`, [
				execIn(`n${index}.in`),
				execOut(`n${index}.out`),
			]);
			connect([...chain, step], previous, `n${index}.in`);
			chain.push(step);
			previous = `n${index}.out`;
		}
		const findings = ofRule(analyzeBoardQuality(board(chain)), "complexity");
		expect(findings).toHaveLength(1);
		expect(findings[0].target.id).toBe("start");
		expect(findings[0].detail).toMatchObject({
			kind: "complex",
			depth: COMPLEXITY_LIMITS.nodes + 3,
		});
	});
});

describe("marks", () => {
	test("aggregates nested findings onto every ancestor layer", () => {
		const outer = layer("outer", ILayerType.Collapsed, []);
		const inner = layer("inner", ILayerType.Collapsed, [], {
			parent_id: "outer",
		});
		const orphan = node("orphan", [execIn("orphan.in")], { layer: "inner" });

		const report = analyzeBoardQuality(board([orphan], [outer, inner]));
		expect(report.marks.orphan?.counts.warning).toBe(1);
		expect(report.marks.inner?.nested).toBe(1);
		expect(report.marks.outer?.nested).toBe(1);
		expect(report.marks.outer?.findings).toHaveLength(0);
		expect(report.marks.outer?.severity).toBe("warning");
	});

	test("a clean board has no findings and no marks", () => {
		const start = startNode("start");
		const sink = node("sink", [execIn("sink.in")]);
		connect([start, sink], "start.out", "sink.in");
		const report = analyzeBoardQuality(board([start, sink]));
		expect(report.findings).toHaveLength(0);
		expect(Object.keys(report.marks)).toHaveLength(0);
	});
});

describe("payment readiness", () => {
	test("only payment requests carry the unverified admission warning", () => {
		const payment = node("pay", [], { name: "request_payment" });
		const report = analyzeBoardQuality(board([payment, node("other", [])]));
		const findings = ofRule(report, "payment-readiness");
		expect(findings).toHaveLength(1);
		expect(findings[0].target.id).toBe("pay");
		expect(findings[0].severity).toBe("warning");
		expect(findings[0].detail).toEqual({
			kind: "payment-readiness",
			simulation: false,
		});
	});

	test("a local simulation setting is informational, not an account-ready claim", () => {
		const payment = node(
			"pay",
			[
				{
					id: "simulation",
					name: "simulation",
					direction: IPinType.Input,
					value: "paid",
				},
			],
			{ name: "request_payment" },
		);
		const finding = ofRule(
			analyzeBoardQuality(board([payment])),
			"payment-readiness",
		)[0];
		expect(finding.severity).toBe("info");
		expect(finding.detail).toEqual({
			kind: "payment-readiness",
			simulation: true,
		});
	});
});
