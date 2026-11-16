import { expect, test } from "bun:test";
import { ADDITIONAL_EMBEDDING_RATINGS } from "./catalog-quality-embeddings";

test("additional embedding estimates have valid identities, ranges and provenance", () => {
	const slugs = new Set<string>();
	const identities = new Map<string, string>();
	for (const rating of ADDITIONAL_EMBEDDING_RATINGS) {
		expect(slugs.has(rating.slug)).toBe(false);
		slugs.add(rating.slug);
		for (const id of new Set(rating.model_ids.map((id) => id.toLowerCase()))) {
			expect(identities.has(id)).toBe(false);
			identities.set(id, rating.slug);
		}
		const estimate = rating.quality_estimate;
		expect(estimate.category).toBe("Embedding");
		expect(estimate.low).toBeGreaterThanOrEqual(0);
		expect(estimate.score).toBeGreaterThanOrEqual(estimate.low);
		expect(estimate.score).toBeLessThanOrEqual(estimate.high);
		expect(estimate.high).toBeLessThanOrEqual(100);
		expect(estimate.confidence).toBe("low");
		expect(estimate.reasoning).toContain("Editorial estimate");
		expect(estimate.sources.length).toBeGreaterThan(0);
		for (const source of estimate.sources) {
			expect(new URL(source).protocol).toBe("https:");
		}
		expect(Number.isFinite(Date.parse(estimate.updated_at))).toBe(true);
	}
});

test("local and hosted Perplexity aliases share one explicitly qualified estimate", () => {
	const rating = ADDITIONAL_EMBEDDING_RATINGS.find((entry) =>
		entry.model_ids.includes("perplexity-ai/pplx-embed-v1-0.6b"),
	);
	expect(rating?.model_ids).toContain("perplexity/pplx-embed-v1-0.6b");
	expect(rating?.quality_estimate.reasoning).toContain(
		"Local and hosted entries share this estimate",
	);
});
