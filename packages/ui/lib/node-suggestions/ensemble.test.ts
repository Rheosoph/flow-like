import { describe, expect, test } from "bun:test";
import { suggest } from "./ensemble";
import { NgramModel } from "./ngram";
import type {
	BoardFacts,
	RankedCandidate,
	SuggestionContext,
	SuggestionModel,
} from "./types";

function chainFacts(edges: [string, string][], dstPin = "exec_in"): BoardFacts {
	const types = [...new Set(edges.flat())];
	return {
		types,
		transitions: edges.map(([src, dst]) => ({
			kind: "exec",
			src: types.indexOf(src),
			srcPin: "exec_out",
			dataType: "Execution",
			valueType: "Normal",
			schema: "",
			pred: [],
			ups: [],
			bag: types.length,
			dst: types.indexOf(dst),
			dstPin,
		})),
		outcomes: edges.map(([src], index) => ({
			kind: "exec",
			src: types.indexOf(src),
			srcPin: "exec_out",
			connected: index % 4 !== 0,
		})),
	};
}

function context(src: string): SuggestionContext {
	return {
		kind: "exec",
		src,
		srcPin: "exec_out",
		dataType: "Execution",
		valueType: "Normal",
		schema: "",
		pred: [],
		ups: [],
		bag: [],
	};
}

function fixedModel(candidates: RankedCandidate[]): SuggestionModel {
	return {
		rank: (_context, limit) => candidates.slice(0, limit),
		serialize: () => "",
	};
}

const ngram = NgramModel.train([
	chainFacts([
		["fetch", "parse"],
		["fetch", "parse"],
		["parse", "store"],
	]),
	chainFacts([
		["fetch", "parse"],
		["fetch", "log"],
	]),
	chainFacts([["fetch", "log"]], "trigger"),
	chainFacts([
		["loop", "loop"],
		["loop", "print"],
	]),
	chainFacts([["loop", "loop"]]),
]);

function renormalized(candidates: RankedCandidate[]): Map<string, number> {
	const total = candidates.reduce((sum, c) => sum + c.probability, 0);
	return new Map(candidates.map((c) => [c.type, c.probability / total]));
}

describe("suggest", () => {
	test("renormalizes the n-gram top list without a neural model", () => {
		const query = context("fetch");
		const result = suggest({ ngram }, query);
		const ranked = ngram.rank(query, 60);
		const expected = renormalized(ranked);
		expect(result.candidates.map((candidate) => candidate.type)).toEqual(
			ranked.slice(0, 10).map((candidate) => candidate.type),
		);
		for (const candidate of result.candidates) {
			expect(candidate.probability).toBeCloseTo(
				expected.get(candidate.type) ?? 0,
				12,
			);
		}
		expect(result.candidates[0].pin).toBe("exec_in");
		expect(result.candidates.find((c) => c.type === "log")?.pin).toBe(
			ngram.predictPin(query, "log"),
		);
	});

	test("mixes 0.7 n-gram with 0.3 neural in log space and keeps neural-only types", () => {
		const query = context("fetch");
		const ngramLog = new Map(
			ngram
				.rank(query, 60)
				.map((candidate) => [candidate.type, Math.log(candidate.probability)]),
		);
		const ngramFloor = Math.min(...ngramLog.values()) - 1;
		const neural = fixedModel([
			{ type: "log", probability: 0.8 },
			{ type: "brand_new", probability: 0.2 },
		]);
		const neuralFloor = Math.log(0.2);
		const score = (type: string) =>
			0.3 *
				(type === "log"
					? Math.log(0.8)
					: type === "brand_new"
						? Math.log(0.2)
						: neuralFloor) +
			0.7 * (ngramLog.get(type) ?? ngramFloor);
		const types = [...new Set([...ngramLog.keys(), "log", "brand_new"])];
		const total = types.reduce((sum, type) => sum + Math.exp(score(type)), 0);
		const result = suggest({ ngram, neural }, query, 60);
		const probability = (type: string) =>
			result.candidates.find((candidate) => candidate.type === type)
				?.probability;
		for (const type of ["parse", "log", "brand_new"]) {
			expect(probability(type)).toBeCloseTo(Math.exp(score(type)) / total, 12);
		}
		expect(
			result.candidates.find((candidate) => candidate.type === "brand_new")
				?.pin,
		).toBeNull();
	});

	test("ranks a self-type top-1 second", () => {
		const query = context("loop");
		expect(ngram.rank(query, 1)[0].type).toBe("loop");
		const result = suggest({ ngram }, query);
		expect(result.candidates[0].type).not.toBe("loop");
		expect(result.candidates[1].type).toBe("loop");
		const only = suggest(
			{
				ngram: NgramModel.train([]),
				neural: fixedModel([{ type: "loop", probability: 1 }]),
			},
			query,
		);
		expect(only.candidates.map((candidate) => candidate.type)).toEqual([
			"loop",
		]);
	});

	test("gates confidence on the chance the pin stays unconnected", () => {
		const query = context("fetch");
		const result = suggest({ ngram }, query, 3);
		expect(result.candidates.length).toBe(3);
		expect(result.pUnconnected).toBe(ngram.pUnconnected(query));
		expect(result.confidence).toBeCloseTo(
			(1 - result.pUnconnected) * result.candidates[0].probability,
			12,
		);
		const empty = suggest({ ngram: NgramModel.train([]) }, query);
		expect(empty.candidates).toEqual([]);
		expect(empty.confidence).toBe(0);
	});
});
