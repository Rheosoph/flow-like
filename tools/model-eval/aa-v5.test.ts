import { describe, expect, test } from "bun:test";
import {
	AA_MODELS_URL,
	AA_V5_VERSION,
	fetchAAV5Catalog,
	parseAAV5Page,
} from "./aa-v5";

const source = {
	sourceUrl: AA_MODELS_URL,
	fetchedAt: "2026-11-16T12:00:00.000Z",
};
const version =
	"<span>Artificial Analysis Intelligence Index v5.0.1 incorporates 9 evaluations</span>";

function row(overrides: Record<string, unknown> = {}): Record<string, unknown> {
	return {
		id: "variant-id",
		slug: "example-xhigh",
		name: "Example (xhigh)",
		release: { slug: "example", name: "Example" },
		effort: { slug: "xhigh", label: "Extra high", level: 50 },
		releaseDate: "2026-11-01",
		creator: { slug: "example-lab", name: "Example Lab" },
		isReasoning: true,
		isOpenWeights: false,
		contextWindowTokens: 128_000,
		intelligenceIndex: 42.8914543287565,
		briefcaseBreakdown: { overall: { elo: 1686.07 } },
		automationBenchPartialScore: 0.72,
		terminalBench40: 0.48,
		frontierCode20: 0.26,
		omniscience: -4.46,
		gdpPdfAllPass: 0.26,
		hleDiamond: 0.44,
		critptAgenticHard: 0.43,
		terminalBenchScience: 0.23,
		omniscienceAccuracy: 0.43,
		omniscienceHallucinationRate: 0.28,
		...overrides,
	};
}

function script(chunk: string): string {
	return `<script nonce="test">self.__next_f.push(${JSON.stringify([1, chunk])})</script>`;
}

function page(rows: unknown[], splitAt?: number): string {
	const record = `1a:${JSON.stringify({ models: rows })}\n`;
	return (
		version +
		(splitAt === undefined
			? script(record)
			: script(record.slice(0, splitAt)) + script(record.slice(splitAt)))
	);
}

describe("parseAAV5Page", () => {
	test("reassembles records split inside JSON strings and keeps exact variant evidence", () => {
		const html = page([row()], 77);
		const parsed = parseAAV5Page(html, source);
		expect(parsed.version).toBe(AA_V5_VERSION);
		expect(parsed.models).toHaveLength(1);
		expect(parsed.models[0]).toMatchObject({
			id: "variant-id",
			slug: "example-xhigh",
			name: "Example (xhigh)",
			release: { slug: "example", name: "Example" },
			effort: { slug: "xhigh", label: "Extra high", level: 50 },
			intelligenceIndex: 42.8914543287565,
			...source,
		});
		expect(parsed.models[0].evaluations).toEqual({
			briefcaseElo: 1686.07,
			automationBenchPartialScore: 0.72,
			terminalBench40: 0.48,
			frontierCode20: 0.26,
			omniscience: -4.46,
			gdpPdfAllPass: 0.26,
			hleDiamond: 0.44,
			critptAgenticHard: 0.43,
			terminalBenchScience: 0.23,
		});
	});

	test("ignores navigation duplicates regardless of their order", () => {
		const navigation = {
			slug: "example-xhigh",
			name: "Example",
			intelligenceIndex: 99,
		};
		const parsed = parseAAV5Page(
			page([navigation, row(), row(), navigation]),
			source,
		);
		expect(parsed.models).toHaveLength(1);
		expect(parsed.models[0].intelligenceIndex).toBe(42.8914543287565);
	});

	test("retains zero and null while leaving absent measurements absent", () => {
		const model = row({
			intelligenceIndex: 0,
			terminalBench40: 0,
			frontierCode20: null,
			briefcaseBreakdown: null,
			omniscienceAccuracy: 0,
		});
		model.hleDiamond = undefined;
		const parsed = parseAAV5Page(page([model]), source).models[0];
		expect(parsed.intelligenceIndex).toBe(0);
		expect(parsed.evaluations.terminalBench40).toBe(0);
		expect(parsed.evaluations.frontierCode20).toBeNull();
		expect(parsed.evaluations.briefcaseElo).toBeNull();
		expect(parsed.omniscienceAccuracy).toBe(0);
		expect(Object.hasOwn(parsed.evaluations, "hleDiamond")).toBe(false);
		expect(
			parseAAV5Page(page([row({ intelligenceIndex: null })]), source).models[0]
				.intelligenceIndex,
		).toBeNull();
		model.intelligenceIndex = undefined;
		expect(
			Object.hasOwn(
				parseAAV5Page(page([model]), source).models[0],
				"intelligenceIndex",
			),
		).toBe(false);
	});

	test("does not replace the variant effort with its release or another effort", () => {
		const models = parseAAV5Page(
			page([
				row(),
				row({ id: "low-id", slug: "example-low", effort: { slug: "low" } }),
			]),
			source,
		).models;
		expect(
			models.map((model) => [
				model.slug,
				model.release.slug,
				model.effort?.slug,
			]),
		).toEqual([
			["example-xhigh", "example", "xhigh"],
			["example-low", "example", "low"],
		]);
		const absent = row();
		absent.effort = undefined;
		expect(
			Object.hasOwn(parseAAV5Page(page([absent]), source).models[0], "effort"),
		).toBe(false);
		expect(
			parseAAV5Page(page([row({ effort: null })]), source).models[0].effort,
		).toBeNull();
	});

	test("rejects conflicting full rows and invalid numeric measurements", () => {
		expect(() =>
			parseAAV5Page(page([row(), row({ intelligenceIndex: 88 })]), source),
		).toThrow("Conflicting AA");
		expect(() =>
			parseAAV5Page(page([row({ intelligenceIndex: "42.89" })]), source),
		).toThrow("Invalid AA measurement");
		expect(() =>
			parseAAV5Page(page([row({ effort: "xhigh" })]), source),
		).toThrow("Invalid AA effort");
	});

	test("requires the visible exact version and benchmark count", () => {
		for (const html of [
			page([row()]).replace("v5.0.1", "v4.3"),
			page([row()]).replace("9 evaluations", "10 evaluations"),
			page([row()]).replace(version, ""),
			page([row()]).replace(
				version,
				`<script>${JSON.stringify(version)}</script>`,
			),
		]) {
			expect(() => parseAAV5Page(html, source)).toThrow();
		}
		expect(
			parseAAV5Page(page([row()]).replace("v5.0.1", "<b>v5.0.1</b>"), source)
				.models,
		).toHaveLength(1);
	});

	test("does not execute script expressions or accept navigation-only data", () => {
		const malicious = `<script>self.__next_f.push([1, (() => { throw new Error("executed"); })()])</script>`;
		expect(
			parseAAV5Page(page([row()]) + malicious, source).models,
		).toHaveLength(1);
		expect(() => parseAAV5Page(version + malicious, source)).toThrow(
			"no full model variants",
		);
		expect(() =>
			parseAAV5Page(
				page([{ slug: "example", name: "Example", intelligenceIndex: 99 }]),
				source,
			),
		).toThrow("no full model variants");
	});
});

describe("fetchAAV5Catalog", () => {
	test("fetches only missing exact slugs and records authoritative 404s", async () => {
		const calls: string[] = [];
		const catalog = await fetchAAV5Catalog(
			["example-xhigh", "example-low", "missing", "renamed", "example-low"],
			{
				now: () => new Date(source.fetchedAt),
				fetcher: async (url) => {
					calls.push(url);
					if (url === AA_MODELS_URL) return new Response(page([row()]));
					if (url.endsWith("/missing"))
						return new Response("Not found", { status: 404 });
					if (url.endsWith("/renamed"))
						return new Response(page([row({ slug: "renamed-new" })]));
					return new Response(
						page([row({ slug: "example-low", effort: { slug: "low" } })]),
					);
				},
			},
		);
		expect(calls).toEqual([
			AA_MODELS_URL,
			`${AA_MODELS_URL}/example-low`,
			`${AA_MODELS_URL}/missing`,
			`${AA_MODELS_URL}/renamed`,
		]);
		expect(catalog.models.map((model) => model.slug)).toEqual([
			"example-xhigh",
			"example-low",
		]);
		expect(catalog.missingSlugs).toEqual(["missing", "renamed"]);
		expect(catalog.pages[2]).toEqual({
			sourceUrl: `${AA_MODELS_URL}/missing`,
			fetchedAt: source.fetchedAt,
			status: 404,
			modelCount: 0,
		});
		expect(catalog.models[1].sourceUrl).toBe(`${AA_MODELS_URL}/example-low`);
	});

	test("fails on source outages, changed versions, and invalid requested slugs", async () => {
		await expect(
			fetchAAV5Catalog([], {
				fetcher: async () => new Response("Unavailable", { status: 503 }),
			}),
		).rejects.toThrow("503");
		await expect(
			fetchAAV5Catalog([], {
				fetcher: async () =>
					new Response(page([row()]).replace("v5.0.1", "v5.1")),
			}),
		).rejects.toThrow("Expected AA");
		await expect(
			fetchAAV5Catalog(["../other"], {
				fetcher: async () => {
					throw new Error("should not fetch");
				},
			}),
		).rejects.toThrow("Invalid AA model slug");
	});
});
