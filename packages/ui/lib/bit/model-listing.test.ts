import { describe, expect, test } from "bun:test";
import type { IBitState } from "../../state/backend-state/bit-state";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import type { IBitSearchQuery } from "../schema/hub/bit-search-query";
import {
	DEPRECATED_TAG,
	MODEL_BIT_TYPES,
	isDeprecated,
	listableModels,
	searchAllBitsOfType,
} from "./model-listing";
import { MODEL_QUALITY_CATEGORIES } from "./model-quality";

function bit(overrides: Partial<IBit> & { id: string }): IBit {
	return {
		dependencies: [],
		meta: { en: { name: overrides.id, tags: [] } },
		...overrides,
	} as unknown as IBit;
}

function named(id: string, name: string, tags: string[] = []): IBit {
	return bit({ id, meta: { en: { name, tags } } as unknown as IBit["meta"] });
}

describe("searchAllBitsOfType", () => {
	test("queries every scored model category and excludes supporting assets", async () => {
		const requested: IBitTypes[] = [];
		const state = {
			async searchBits(query: IBitSearchQuery) {
				requested.push(...(query.bit_types ?? []));
				return [];
			},
		} as unknown as IBitState;
		await searchAllBitsOfType.call(state, MODEL_BIT_TYPES);
		expect(new Set(requested)).toEqual(
			new Set([
				IBitTypes.Llm,
				IBitTypes.Vlm,
				...(Object.keys(MODEL_QUALITY_CATEGORIES) as IBitTypes[]),
			]),
		);
		expect(requested).toContain(IBitTypes.ObjectDetection);
		expect(requested).not.toContain(IBitTypes.File);
		expect(requested).not.toContain(IBitTypes.Tokenizer);
	});

	test("keeps small model families visible beside a paginated chat catalog", async () => {
		const chat = Array.from({ length: 125 }, (_, i) =>
			bit({ id: `chat-${i}`, type: IBitTypes.Llm }),
		);
		const forecasts = Array.from({ length: 6 }, (_, i) =>
			bit({ id: `forecast-${i}`, type: IBitTypes.Forecasting }),
		);
		const state = {
			async searchBits(query: IBitSearchQuery) {
				const offset = query.offset ?? 0;
				const limit = query.limit ?? 50;
				// Old hubs can use different query plans for mixed-type pages.
				// Their second page overlaps the first and loses small families.
				if ((query.bit_types?.length ?? 0) > 1) {
					return offset === 0 ? chat.slice(0, limit) : chat.slice(75);
				}
				const family =
					query.bit_types?.[0] === IBitTypes.Forecasting ? forecasts : chat;
				return family.slice(offset, offset + limit);
			},
		} as IBitState;

		const result = await searchAllBitsOfType.call(state, [
			IBitTypes.Llm,
			IBitTypes.Forecasting,
		]);
		expect(result).toHaveLength(131);
		expect(
			result.filter((model) => model.type === IBitTypes.Forecasting),
		).toEqual(forecasts);
		expect(new Set(result.map((model) => model.id)).size).toBe(131);
	});
});

describe("isDeprecated", () => {
	test("a model with no tags is offered", () => {
		expect(isDeprecated(named("a", "Granite - Nano"))).toBe(false);
	});

	test("the retirement tag withdraws a model", () => {
		expect(isDeprecated(named("a", "Jina - DE / EN", [DEPRECATED_TAG]))).toBe(
			true,
		);
	});

	test("the tag is matched regardless of case or padding", () => {
		expect(isDeprecated(named("a", "Jina", ["  Deprecated "]))).toBe(true);
	});

	test("an unrelated tag does not withdraw a model", () => {
		expect(isDeprecated(named("a", "Jina", ["deprecation-policy"]))).toBe(
			false,
		);
	});

	test("a tag on a non-English metadata row still counts", () => {
		const withGerman = bit({
			id: "a",
			meta: {
				de: { name: "Jina", tags: [DEPRECATED_TAG] },
			} as unknown as IBit["meta"],
		});
		expect(isDeprecated(withGerman)).toBe(true);
	});

	test("a bit carrying no metadata at all is not treated as retired", () => {
		expect(isDeprecated(bit({ id: "a", meta: undefined }))).toBe(false);
	});
});

describe("listableModels", () => {
	test("a retired model is withheld while its peers stay", () => {
		const bits = [
			named("current", "Granite - Nano"),
			named("retired", "Jina v2 - Code", [DEPRECATED_TAG]),
		];
		expect(listableModels(bits).map((b) => b.id)).toEqual(["current"]);
	});

	test("retirement is independent of the dependency and naming rules", () => {
		const bits = [
			named("parent", "Clip - Vision"),
			bit({
				id: "parent",
				meta: {
					en: { name: "Clip - Vision", tags: [] },
				} as unknown as IBit["meta"],
				dependencies: ["api.flow-like.com:half"],
			}),
			named("half", "text half"),
			named("unnamed", ""),
			named("retired", "Jina - EN / ZH", [DEPRECATED_TAG]),
		];
		// The component half, the nameless bit and the retired model all drop out.
		expect(new Set(listableModels(bits).map((b) => b.id))).toEqual(
			new Set(["parent"]),
		);
	});

	test("a retired model is still present in the input, so it stays resolvable", () => {
		const bits = [named("retired", "Jina - EN / ES", [DEPRECATED_TAG])];
		expect(listableModels(bits)).toHaveLength(0);
		expect(bits.find((b) => b.id === "retired")).toBeDefined();
	});
});
