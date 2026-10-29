import { describe, expect, test } from "bun:test";
import { parsePublicModelBenchmarks } from "./public-benchmarks";

function page(rows: unknown[]) {
	const record = `a:${JSON.stringify(["$", "section", null, { initialModels: rows }])}\n`;
	const middle = Math.floor(record.length / 2);
	return [record.slice(0, middle), record.slice(middle)]
		.map(
			(chunk) =>
				`<script>self.__next_f.push(${JSON.stringify([1, chunk])})</script>`,
		)
		.join("");
}

describe("public benchmark page data", () => {
	test("reads split records, preserves zero scores, and ignores navigation duplicates", () => {
		const result = parsePublicModelBenchmarks(
			page([
				{
					slug: "model",
					name: "Model",
					automationBenchPartialScore: 0,
					openness: { opennessIndex: 83.3 },
					openSourceCategorization: "permissive",
					omniscienceAccuracy: 0.5,
					omniscienceHallucinationRate: 0,
				},
				{ slug: "model", name: "Model" },
			]),
		);
		expect(result.get("model")).toEqual({
			automationBench: 0,
			opennessIndex: 83.3,
			openSourceCategorization: "Open Weights (Permissive License)",
			omniscienceAccuracy: 0.5,
			omniscienceHallucinationRate: 0,
		});
	});

	test("reads nested evaluation data and keeps raw multilingual averages distinct", () => {
		const result = parsePublicModelBenchmarks(
			page([
				{
					slug: "model",
					name: "Model",
					omniscienceBreakdown: { accuracy: 0.4, hallucinationRate: 0.2 },
					multilingual: { average: 0.8 },
					opennessIndex: 0,
				},
			]),
		);
		expect(result.get("model")).toEqual({
			omniscienceAccuracy: 0.4,
			omniscienceHallucinationRate: 0.2,
			multilingualIndex: 0.8,
			opennessIndex: 0,
		});
	});

	test("rejects invalid scores and never executes scripts", () => {
		const result = parsePublicModelBenchmarks(
			`${page([
				{
					slug: "bad",
					name: "Bad",
					automationBenchPartialScore: 20,
					omniscienceAccuracy: -1,
					opennessIndex: 101,
				},
			])}<script>self.__next_f.push(runUntrustedCode())</script>`,
		);
		expect(result.size).toBe(0);
	});
});
