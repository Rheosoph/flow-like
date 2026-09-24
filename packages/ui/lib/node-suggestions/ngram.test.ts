import { describe, expect, test } from "bun:test";
import { NgramModel } from "./ngram";
import type {
	BoardFacts,
	EdgeKind,
	FactTransition,
	SuggestionContext,
} from "./types";

interface EdgeSpec {
	src: string;
	dst: string;
	kind?: EdgeKind;
	srcPin?: string;
	dstPin?: string;
	pred?: string[];
	ups?: string[];
}

interface OutcomeSpec {
	src: string;
	srcPin: string;
	connected: boolean;
	kind?: EdgeKind;
}

function boardFacts(
	edges: EdgeSpec[],
	options: {
		extra?: string[];
		outcomes?: OutcomeSpec[];
		repeated?: string[];
	} = {},
): BoardFacts {
	const types: string[] = [];
	const positions = new Map<string, number>();
	const index = (type: string) => {
		const found = positions.get(type);
		if (found !== undefined) return found;
		positions.set(type, types.length);
		types.push(type);
		return types.length - 1;
	};
	for (const type of options.extra ?? []) index(type);
	const transitions: FactTransition[] = edges.map((edge) => {
		const kind = edge.kind ?? "exec";
		return {
			kind,
			src: index(edge.src),
			srcPin: edge.srcPin ?? (kind === "exec" ? "exec_out" : "value"),
			dataType: kind === "exec" ? "Execution" : "String",
			valueType: "Normal",
			schema: "",
			pred: (edge.pred ?? []).map(index),
			ups: (edge.ups ?? []).map(index),
			bag: 0,
			dst: index(edge.dst),
			dstPin: edge.dstPin ?? (kind === "exec" ? "exec_in" : "input"),
		};
	});
	const outcomes = (options.outcomes ?? []).map((outcome) => ({
		kind: outcome.kind ?? "exec",
		src: index(outcome.src),
		srcPin: outcome.srcPin,
		connected: outcome.connected,
	}));
	for (const transition of transitions) transition.bag = types.length;
	return {
		types,
		transitions,
		outcomes,
		repeated: (options.repeated ?? []).map(index),
	};
}

function context(
	src: string,
	overrides: Partial<SuggestionContext> = {},
): SuggestionContext {
	const kind = overrides.kind ?? "exec";
	return {
		kind,
		src,
		srcPin: kind === "exec" ? "exec_out" : "value",
		dataType: kind === "exec" ? "Execution" : "String",
		valueType: "Normal",
		schema: "",
		pred: [],
		ups: [],
		bag: [],
		...overrides,
	};
}

function times<T>(count: number, make: (index: number) => T): T[] {
	return Array.from({ length: count }, (_, index) => make(index));
}

function top(model: NgramModel, query: SuggestionContext, limit = 5) {
	return model.rank(query, limit).map((candidate) => candidate.type);
}

describe("NgramModel.rank", () => {
	test("learns a deterministic follow-up", () => {
		const model = NgramModel.train(
			times(6, () =>
				boardFacts([
					{ src: "read_file", dst: "parse_json" },
					{ src: "parse_json", dst: "log" },
				]),
			),
		);
		const ranked = model.rank(context("read_file"), 3);
		expect(ranked[0].type).toBe("parse_json");
		expect(ranked[0].probability).toBeGreaterThan(0.5);
	});

	test("conditions on the exec predecessor", () => {
		const corpus = [
			...times(4, () =>
				boardFacts([{ src: "branch", dst: "left", pred: ["check_a"] }]),
			),
			...times(4, () =>
				boardFacts([{ src: "branch", dst: "right", pred: ["check_b"] }]),
			),
		];
		const model = NgramModel.train(corpus);
		expect(top(model, context("branch", { pred: ["check_a"] }))[0]).toBe(
			"left",
		);
		expect(top(model, context("branch", { pred: ["check_b"] }))[0]).toBe(
			"right",
		);
	});

	test("counts each context and target once per board", () => {
		const spam = boardFacts(times(10, () => ({ src: "start", dst: "noisy" })));
		const model = NgramModel.train([
			spam,
			boardFacts([{ src: "start", dst: "steady" }]),
			boardFacts([{ src: "start", dst: "steady" }]),
		]);
		expect(top(model, context("start"))[0]).toBe("steady");
	});

	test("re-ranks by upstream types", () => {
		const corpus = [
			...times(3, () =>
				boardFacts([
					{ src: "format", dst: "send_mail", ups: ["email_template"] },
				]),
			),
			...times(3, () =>
				boardFacts([{ src: "format", dst: "write_file", ups: ["file_path"] }]),
			),
			boardFacts([{ src: "format", dst: "send_mail" }]),
			boardFacts([{ src: "format", dst: "write_file" }]),
		];
		const model = NgramModel.train(corpus);
		expect(top(model, context("format", { ups: ["file_path"] }))[0]).toBe(
			"write_file",
		);
		expect(top(model, context("format", { ups: ["email_template"] }))[0]).toBe(
			"send_mail",
		);
	});

	test("re-ranks by board co-occurrence of the bag", () => {
		const corpus = [
			...times(3, () =>
				boardFacts([{ src: "set_var", dst: "http_request" }], {
					extra: ["api_key"],
				}),
			),
			...times(3, () =>
				boardFacts([{ src: "set_var", dst: "render_chart" }], {
					extra: ["chart_theme"],
				}),
			),
		];
		const model = NgramModel.train(corpus);
		expect(top(model, context("set_var", { bag: ["api_key"] }))[0]).toBe(
			"http_request",
		);
		expect(top(model, context("set_var", { bag: ["chart_theme"] }))[0]).toBe(
			"render_chart",
		);
	});

	test("backs off to popularity for unknown contexts and never throws", () => {
		const model = NgramModel.train([
			boardFacts([
				{ src: "a", dst: "popular" },
				{ src: "b", dst: "popular" },
				{ src: "c", dst: "rare" },
			]),
			boardFacts([{ src: "a", dst: "popular" }]),
		]);
		expect(top(model, context("never_seen"))[0]).toBe("popular");
		expect(
			top(model, context("a", { srcPin: "unknown_pin", pred: ["ghost"] }))[0],
		).toBe("popular");
		expect(model.rank(context("a", { kind: "data" }), 5)).toEqual([]);
		expect(model.rank(context("a"), 0)).toEqual([]);
		const broken = {
			...context("a"),
			pred: undefined,
			ups: undefined,
			bag: undefined,
		} as unknown as SuggestionContext;
		expect(() => model.rank(broken, 5)).not.toThrow();
		expect(NgramModel.train([]).rank(context("a"), 5)).toEqual([]);
	});

	test("returns sorted probabilities that sum to at most one", () => {
		const corpus = times(40, (board) =>
			boardFacts(
				times(12, (edge) => ({
					src: `t${(board + edge) % 9}`,
					dst: `t${(board * 3 + edge * 7) % 50}`,
					pred: [`t${edge % 5}`],
				})),
			),
		);
		const model = NgramModel.train(corpus);
		const ranked = model.rank(context("t1", { pred: ["t2"] }), 200);
		expect(ranked.length).toBeGreaterThan(30);
		const total = ranked.reduce((sum, item) => sum + item.probability, 0);
		expect(total).toBeLessThanOrEqual(1 + 1e-9);
		expect(total).toBeGreaterThan(0.5);
		for (let index = 1; index < ranked.length; index++) {
			expect(ranked[index - 1].probability).toBeGreaterThanOrEqual(
				ranked[index].probability,
			);
		}
		expect(new Set(ranked.map((item) => item.type)).size).toBe(ranked.length);
	});
});

describe("NgramModel.predictPin", () => {
	test("prefers the pin learned for this anchor, then the target's usual pin", () => {
		const model = NgramModel.train([
			boardFacts([
				{ kind: "data", src: "a", dst: "concat", dstPin: "right" },
				{ kind: "data", src: "b", dst: "concat", dstPin: "left" },
				{ kind: "data", src: "c", dst: "concat", dstPin: "left" },
			]),
		]);
		const data = { kind: "data" as const };
		expect(model.predictPin(context("a", data), "concat")).toBe("right");
		expect(model.predictPin(context("unknown", data), "concat")).toBe("left");
		expect(model.predictPin(context("a", data), "missing")).toBeNull();
		expect(model.predictPin(context("a"), "concat")).toBeNull();
	});
});

describe("NgramModel.pUnconnected", () => {
	test("backs off from (src, pin) to pin to kind", () => {
		const outcomes: OutcomeSpec[] = [
			...times(20, () => ({
				src: "request",
				srcPin: "exec_error",
				connected: false,
			})),
			...times(20, () => ({
				src: "request",
				srcPin: "exec_out",
				connected: true,
			})),
		];
		const model = NgramModel.train([boardFacts([], { outcomes })]);
		const error = model.pUnconnected(
			context("request", { srcPin: "exec_error" }),
		);
		const out = model.pUnconnected(context("request", { srcPin: "exec_out" }));
		const unseen = model.pUnconnected(
			context("other", { srcPin: "exec_error" }),
		);
		const unknown = model.pUnconnected(context("other", { srcPin: "nope" }));
		expect(error).toBeGreaterThan(0.85);
		expect(out).toBeLessThan(0.15);
		expect(unseen).toBeGreaterThan(0.8);
		expect(unknown).toBeCloseTo(0.5, 1);
		expect(model.pUnconnected(context("x", { kind: "data" }))).toBe(0.5);
	});
});

describe("NgramModel serialization", () => {
	function sample() {
		return NgramModel.train(
			times(12, (board) =>
				boardFacts(
					[
						...times(8, (edge) => ({
							src: `s${(board + edge) % 6}`,
							dst: `d${(board * 5 + edge) % 11}`,
							pred: edge % 2 ? [`s${edge % 3}`, `s${edge % 4}`] : [],
							ups: edge % 3 ? [`u${edge % 2}`] : [],
							dstPin: `in_${edge % 3}`,
						})),
						{ kind: "data", src: "value", dst: "concat", dstPin: "left" },
					],
					{
						extra: ["shared", `only_${board % 2}`],
						repeated: ["shared"],
						outcomes: [
							{ src: "s1", srcPin: "exec_out", connected: board % 3 === 0 },
							{ kind: "data", src: "value", srcPin: "value", connected: true },
						],
					},
				),
			),
		);
	}

	test("round-trips every prediction", () => {
		const model = sample();
		const json = model.serialize();
		const restored = NgramModel.deserialize(json);
		expect(restored.serialize()).toBe(json);
		const queries = [
			context("s1", { pred: ["s1", "s2"], ups: ["u1"], bag: ["shared"] }),
			context("s3", { bag: ["only_0", "shared"] }),
			context("value", { kind: "data" }),
			context("unknown"),
		];
		for (const query of queries) {
			expect(restored.rank(query, 20)).toEqual(model.rank(query, 20));
			expect(restored.pUnconnected(query)).toBe(model.pUnconnected(query));
			for (const type of ["d1", "d4", "concat", "missing"]) {
				expect(restored.predictPin(query, type)).toBe(
					model.predictPin(query, type),
				);
			}
		}
	});

	test("rejects other format versions", () => {
		const data = JSON.parse(sample().serialize());
		expect(() =>
			NgramModel.deserialize(JSON.stringify({ ...data, version: 999 })),
		).toThrow(/Unsupported n-gram model format/);
	});
});

describe("NgramModel training cost", () => {
	test("trains on ~40k transitions in well under a second", () => {
		const corpus = times(350, (board) => {
			const edges: EdgeSpec[] = times(115, (edge) => {
				const src = `type_${(board * 7 + edge * 13) % 400}`;
				return {
					kind: edge % 5 < 2 ? "exec" : "data",
					src,
					srcPin: `pin_${edge % 6}`,
					dst: `type_${(board * 3 + edge * 29) % 600}`,
					dstPin: `in_${edge % 4}`,
					pred: [`type_${edge % 50}`, `type_${(edge + 1) % 50}`],
					ups: [`type_${edge % 30}`, `type_${edge % 31}`],
				};
			});
			return boardFacts(edges, {
				outcomes: edges.map((edge, index) => ({
					kind: edge.kind,
					src: edge.src,
					srcPin: edge.srcPin ?? "exec_out",
					connected: index % 4 !== 0,
				})),
			});
		});
		const started = performance.now();
		const model = NgramModel.train(corpus);
		const elapsed = performance.now() - started;
		expect(
			model.rank(context("type_7", { kind: "data", srcPin: "pin_1" }), 5),
		).not.toEqual([]);
		expect(elapsed).toBeLessThan(1000);
	});
});
