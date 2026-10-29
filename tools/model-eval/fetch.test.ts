import { expect, test } from "bun:test";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fetchModelsWithCache } from "./fetch";

test("enriches linked model pages with bounded reads and keeps failures isolated", async () => {
	const root = await mkdtemp(join(tmpdir(), "model-eval-fetch-"));
	const originalFetch = globalThis.fetch;
	const calls: string[] = [];
	let active = 0;
	let peak = 0;
	const models = Array.from({ length: 8 }, (_, index) => ({
		id: `model-${index}`,
		name: `Model ${index}`,
		slug: `model-${index}`,
		model_creator: { id: "test", slug: "test", name: "Test" },
		evaluations: {},
		pricing: {},
	}));
	const catalogEstimate = {
		...models[0],
		slug: "flow-like-estimate-specialist",
		evaluations: { intelligence_estimate: { score: 25 } },
	};
	try {
		await mkdir(join(root, "tmp"));
		await writeFile(
			join(root, "tmp/results.json"),
			JSON.stringify({
				modelCount: models.length,
				fetchedAt: "2026-01-01",
				globalMaxes: {},
				data: models.map((model) => ({
					...model,
					evaluations: {
						artificial_analysis_omniscience_accuracy: 0.1,
						artificial_analysis_omniscience_index_normalized: 0.9,
						artificial_analysis_multilingual_index_normalized: 0.95,
					},
				})),
			}),
		);
		globalThis.fetch = (async (input: string | URL | Request) => {
			const url =
				typeof input === "string"
					? input
					: input instanceof URL
						? input.href
						: input.url;
			calls.push(url);
			active++;
			peak = Math.max(peak, active);
			await new Promise((resolve) => setTimeout(resolve, 1));
			active--;
			if (url.includes("/api/v2/data/llms/models"))
				return Response.json({ data: [...models, catalogEstimate] });
			const model = models.find(
				(model) => url === `https://artificialanalysis.ai/models/${model.slug}`,
			);
			if (model?.slug === "model-1")
				return new Response("missing", { status: 404 });
			if (!model) return new Response("");
			const row = {
				...model,
				omniscienceBreakdown: { accuracy: 0.6, hallucinationRate: 0.2 },
				multilingual: { average: 0.4 },
				automationBenchPartialScore: 0.7,
			};
			const record = `a:${JSON.stringify(["$", "section", null, { model: row }])}\n`;
			return new Response(
				`<script>self.__next_f.push(${JSON.stringify([1, record])})</script>`,
			);
		}) as typeof fetch;

		const result = await fetchModelsWithCache("test-key", root, [
			...models.map((model) => model.slug),
			"model-0",
			"not-in-api",
			catalogEstimate.slug,
		]);
		expect(peak).toBeLessThanOrEqual(5);
		expect(calls.filter((url) => url.endsWith("/model-0"))).toHaveLength(1);
		expect(calls.some((url) => url.endsWith("/not-in-api"))).toBe(false);
		expect(calls.some((url) => url.includes(catalogEstimate.slug))).toBe(false);
		expect(
			result.models.some((model) => model.slug === catalogEstimate.slug),
		).toBe(false);
		const refreshed = result.models.find(
			(model) => model.slug === "model-0",
		)?.evaluations;
		if (!refreshed) throw new Error("Refreshed model is missing");
		expect(refreshed.artificial_analysis_omniscience_accuracy).toBe(0.6);
		expect(
			refreshed.artificial_analysis_omniscience_index_normalized,
		).toBeNull();
		expect(refreshed.artificial_analysis_multilingual_index).toBe(0.4);
		expect(
			refreshed.artificial_analysis_multilingual_index_normalized,
		).toBeNull();
		expect(refreshed.automationbench_aa).toBe(0.7);
		expect(
			result.models.find((model) => model.slug === "model-1")?.evaluations
				.artificial_analysis_omniscience_accuracy,
		).toBe(0.1);
	} finally {
		globalThis.fetch = originalFetch;
		await rm(root, { recursive: true, force: true });
	}
});

test("excludes catalog estimates from an AA cache fallback", async () => {
	const root = await mkdtemp(join(tmpdir(), "model-eval-cache-"));
	const originalFetch = globalThis.fetch;
	try {
		await mkdir(join(root, "tmp"));
		await writeFile(
			join(root, "tmp/results.json"),
			JSON.stringify({
				modelCount: 2,
				fetchedAt: "2026-10-27",
				data: [
					{
						slug: "measured",
						evaluations: { artificial_analysis_intelligence_index: 45 },
						pricing: {},
					},
					{
						slug: "flow-like-estimate-specialist",
						evaluations: { artificial_analysis_intelligence_index: 99 },
						pricing: {},
					},
				],
			}),
		);
		globalThis.fetch = (async (_input: string | URL | Request) =>
			new Response("offline", { status: 503 })) as typeof fetch;
		const result = await fetchModelsWithCache("test-key", root, [
			"flow-like-estimate-specialist",
		]);
		expect(result.models.map((model) => model.slug)).toEqual(["measured"]);
		expect(result.maxes.intelligence_index).toBe(45);
	} finally {
		globalThis.fetch = originalFetch;
		await rm(root, { recursive: true, force: true });
	}
});
