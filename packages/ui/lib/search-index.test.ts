import { describe, expect, test } from "bun:test";
import { buildSearchIndex } from "./search-index";

interface Pkg {
	id: string;
	manifest: { name: string; description: string; keywords: string[] };
}

const packages: Pkg[] = [
	{
		id: "flow-like.storage",
		manifest: {
			name: "Storage Nodes",
			description: "Read and write files",
			keywords: ["files", "s3"],
		},
	},
	{
		id: "acme.gpt",
		manifest: {
			name: "GPT-4o Connector",
			description: "OpenAI multimodal model access",
			keywords: ["llm", "openai"],
		},
	},
	{
		id: "acme.chartKit",
		manifest: {
			name: "chartKit",
			description: "Charts and dashboards",
			keywords: [],
		},
	},
];

const index = buildSearchIndex(packages, {
	fields: ["id", "manifest.name", "manifest.description", "manifest.keywords"],
	boost: { "manifest.name": 3 },
});

const ids = (items: Pkg[]) => items.map((item) => item.id);

describe("buildSearchIndex", () => {
	test("returns everything, in order, for an empty query", () => {
		expect(ids(index.search("   "))).toEqual(ids(packages));
	});

	test("matches partial words as the user types", () => {
		expect(ids(index.search("stor"))).toEqual(["flow-like.storage"]);
		expect(ids(index.search("multim"))).toEqual(["acme.gpt"]);
	});

	test("reads dotted paths and array fields", () => {
		expect(ids(index.search("s3"))).toEqual(["flow-like.storage"]);
		expect(ids(index.search("openai"))).toEqual(["acme.gpt"]);
	});

	test("tolerates typos", () => {
		expect(ids(index.search("sorage"))).toEqual(["flow-like.storage"]);
	});

	test("narrows on additional terms instead of widening", () => {
		expect(ids(index.search("openai model"))).toEqual(["acme.gpt"]);
		expect(index.search("openai storage")).toEqual([]);
	});

	test("splits camelCase names", () => {
		expect(ids(index.search("kit"))).toEqual(["acme.chartKit"]);
	});

	test("splits punctuated ids and names", () => {
		expect(ids(index.search("gpt-4o"))).toEqual(["acme.gpt"]);
	});

	test("survives missing and null fields", () => {
		const sparse = buildSearchIndex(
			[{ id: "a" }, { id: "b", manifest: null }] as unknown as Pkg[],
			{ fields: ["id", "manifest.name"] },
		);
		expect(ids(sparse.search("b"))).toEqual(["b"]);
	});

	test("indexes extra text", () => {
		const withExtra = buildSearchIndex(packages, {
			fields: ["id"],
			extract: (pkg) => (pkg.id === "acme.gpt" ? "favourite pinned" : ""),
		});
		expect(ids(withExtra.search("pinned"))).toEqual(["acme.gpt"]);
	});

	test("finds model bits by name, provider and id fragments", () => {
		const bits = [
			{
				id: "openai-gpt-4o",
				type: "Llm",
				meta: { en: { name: "GPT-4o", description: "Multimodal flagship" } },
				parameters: {
					provider: { provider_name: "OpenAI", model_id: "gpt-4o" },
				},
			},
			{
				id: "google-gemini-2-5-pro",
				type: "Llm",
				meta: { en: { name: "Gemini 2.5 Pro", description: "Long context" } },
				parameters: {
					provider: { provider_name: "Google", model_id: "gemini-2.5-pro" },
				},
			},
		];
		const catalog = buildSearchIndex(bits, {
			fields: [
				"meta.en.name",
				"meta.en.description",
				"id",
				"type",
				"parameters.provider.provider_name",
				"parameters.provider.model_id",
			],
			boost: { "meta.en.name": 4, id: 2 },
		});
		const found = (q: string) => catalog.search(q).map((bit) => bit.id);

		expect(found("gemin")).toEqual(["google-gemini-2-5-pro"]);
		expect(found("openai")).toEqual(["openai-gpt-4o"]);
		expect(found("gpt-4o")).toEqual(["openai-gpt-4o"]);
		expect(found("gemini 2.5")).toEqual(["google-gemini-2-5-pro"]);
		expect(found("long context")).toEqual(["google-gemini-2-5-pro"]);
	});

	test("handles an undefined collection", () => {
		const empty = buildSearchIndex<Pkg>(undefined, { fields: ["id"] });
		expect(empty.isEmpty).toBe(true);
		expect(empty.search("anything")).toEqual([]);
		expect(empty.search("")).toEqual([]);
	});
});
