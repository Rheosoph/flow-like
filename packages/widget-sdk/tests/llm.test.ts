import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { LlmHistory, LlmResponse, LlmResponseChunk } from "../src";
import { isLlmKind, llmSchema } from "../src/llm";
import { LLM_SCHEMAS } from "../src/llm-schemas";
import { validateSchema } from "../src/validate";

const marker = (kind: string) => ({
	type: "object",
	"x-flow-like-type": "llm",
	"x-llm": kind,
});

// Shapes as flow_like_model_provider serializes them (Response::from_text, ResponseChunk::from_text, History::new).
const response: LlmResponse = {
	choices: [
		{
			index: 0,
			finish_reason: "stop",
			message: {
				role: "assistant",
				content: "Over the Pacific.",
				content_parts: [
					{
						type: "image_url",
						image_url: { url: "https://example.test/iss.png" },
					},
				],
				tool_calls: [
					{
						id: "call-1",
						type: "function",
						function: { name: "fly", arguments: "{}" },
					},
				],
			},
		},
	],
	model: "gpt",
	usage: {
		completion_tokens: 3,
		prompt_tokens: 9,
		total_tokens: 12,
		cost: null,
	},
};
const chunk: LlmResponseChunk = {
	id: "chunk-1",
	choices: [{ index: 0, delta: { role: "assistant", content: "Over" } }],
};
const history: LlmHistory = {
	model: "",
	messages: [
		{ role: "user", content: [{ type: "text", text: "Where is the ISS?" }] },
		{ role: "assistant", content: "Over the Pacific." },
	],
	preset: null,
	stream: false,
	tools: [
		{
			type: "function",
			function: {
				name: "fly",
				parameters: {
					type: "object",
					properties: {
						target: { type: "object", properties: { id: { type: "string" } } },
					},
					required: ["target"],
				},
			},
		},
	],
};

describe("Flow-Like LLM type markers", () => {
	test("the SDK schemas are the generated native schemas", () => {
		for (const [kind, file] of [
			["History", "history"],
			["Response", "response"],
			["ResponseChunk", "response-chunk"],
		] as const) {
			const generated = JSON.parse(
				readFileSync(
					new URL(`../../schema/llm/${file}.json`, import.meta.url),
					"utf8",
				),
			);
			expect(LLM_SCHEMAS[kind]).toEqual(generated);
			expect(JSON.stringify(llmSchema(kind))).not.toContain("$ref");
		}
		expect(isLlmKind("Response")).toBe(true);
		expect(isLlmKind("toString")).toBe(false);
	});

	test("validates marked values against the native schemas, including recursive tool parameters", () => {
		expect(validateSchema(marker("Response"), response)).toEqual({
			valid: true,
			errors: [],
		});
		expect(validateSchema(marker("ResponseChunk"), chunk).valid).toBe(true);
		expect(validateSchema(marker("History"), history).valid).toBe(true);
		const nested = {
			type: "object",
			properties: { chunk: marker("ResponseChunk") },
			required: ["chunk"],
		};
		expect(validateSchema(nested, { chunk }).valid).toBe(true);

		const { id: _id, ...withoutId } = chunk;
		const { usage: _usage, ...withoutUsage } = response;
		for (const [kind, value] of [
			["ResponseChunk", withoutId],
			["ResponseChunk", { ...chunk, choices: "no" }],
			["Response", withoutUsage],
			[
				"Response",
				{
					...response,
					choices: [
						{ index: 0, finish_reason: "stop", message: { role: "assistant" } },
					],
				},
			],
			["History", { ...history, messages: [{ role: "robot", content: "hi" }] }],
			[
				"History",
				{
					...history,
					messages: [{ role: "user", content: [{ type: "text" }] }],
				},
			],
			["History", { messages: [] }],
		] as const) {
			expect(validateSchema(marker(kind), value).valid).toBe(false);
		}
	});

	test("rejects malformed markers instead of widening them", () => {
		for (const schema of [
			marker("Nope"),
			{ type: "object", "x-flow-like-type": "llm" },
			{ type: "object", "x-llm": "Response" },
		]) {
			const result = validateSchema(schema, response);
			expect(result.valid).toBe(false);
			expect(result.errors[0]).toContain("Invalid Flow-Like LLM type marker");
		}
	});
});
