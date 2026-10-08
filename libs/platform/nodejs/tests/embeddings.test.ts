import { afterAll, afterEach, expect, spyOn, test } from "bun:test";
import { createHttpClient } from "../src/client.js";
import { createEmbeddingMethods } from "../src/embeddings.js";
import type { EmbeddingInput } from "../src/types.js";

const signedUrl =
	"https://storage.googleapis.com/bucket/demo.mp4?X-Goog-Signature=a%2Fb&X-Goog-Expires=600";
const fetchMock = spyOn(globalThis, "fetch");
const { embed } = createEmbeddingMethods(
	createHttpClient("https://flow-like.example", {
		type: "pat",
		token: "pat_test",
	}),
);

afterEach(() => fetchMock.mockReset());
afterAll(() => fetchMock.mockRestore());

const cases: { name: string; input: EmbeddingInput | EmbeddingInput[] }[] = [
	{ name: "single text", input: "query: demo" },
	{ name: "text batch", input: ["first", "second"] },
	{ name: "structured text", input: { type: "text", text: "demo" } },
	{ name: "raw base64 image", input: { type: "image", image: "aW1hZ2U=" } },
	{
		name: "audio data URL",
		input: { type: "audio", audio: "data:audio/wav;base64,YXVkaW8=" },
	},
	{ name: "signed video URL", input: { type: "video", video: signedUrl } },
	{
		name: "combined input",
		input: {
			type: "multimodal",
			text: "Demo: <|image|> <|audio|> <|video|>",
			image: "https://example.com/product.jpg",
			audio: "data:audio/wav;base64,YXVkaW8=",
			video: signedUrl,
		},
	},
	{
		name: "mixed batch",
		input: [
			"demo",
			{ type: "audio", audio: "data:audio/wav;base64,YXVkaW8=" },
			{ type: "video", video: signedUrl },
		],
	},
];

for (const { name, input } of cases) {
	test(`serializes ${name} without changing media or batch boundaries`, async () => {
		const inputs = Array.isArray(input) ? input : [input];
		const result = {
			embeddings: inputs.map((_, index) => [index, 0.5]),
			model: "embedding-bit",
			usage: { prompt_tokens: 42, total_tokens: 42 },
			usage_estimated: false,
			usage_available: true,
		};
		fetchMock.mockResolvedValueOnce(Response.json(result));
		const signal = new AbortController().signal;

		expect(
			await embed("embedding-bit", input, { embed_type: "document", signal }),
		).toEqual(result);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe("https://flow-like.example/api/v1/embeddings/embed");
		expect(options?.method).toBe("POST");
		expect(options?.signal).toBe(signal);
		expect(JSON.parse(String(options?.body))).toEqual({
			model: "embedding-bit",
			input: inputs,
			embed_type: "document",
		});
	});
}

test("keeps the query default for text requests", async () => {
	fetchMock.mockResolvedValueOnce(Response.json({ embeddings: [] }));
	await embed("embedding-bit", "query");
	expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body)).embed_type).toBe(
		"query",
	);
});
