import { describe, expect, test } from "bun:test";
import { researchAgentPrompt } from "./research-brief";

describe("researchAgentPrompt", () => {
	test("keeps the subject of a conversational follow-up in the bot's brief", () => {
		const prompt = researchAgentPrompt({
			question: "Why does H3 use hexagons rather than a square grid?",
			context:
				'The conversation concerns H3 geospatial indexing. The follow-up was "warum besser als quadrate zb". Compare neighbor distances and limitations, using primary sources.',
		});

		expect(prompt).toContain(
			"Research question:\nWhy does H3 use hexagons rather than a square grid?",
		);
		expect(prompt).toContain(
			"Relevant public context:\nThe conversation concerns H3 geospatial indexing.",
		);
		expect(prompt).toContain("Compare neighbor distances and limitations");
	});

	test("preserves a revised question and evidence gaps for another call", () => {
		const prompt = researchAgentPrompt({
			question: "Verify which H3 cells are pentagons.",
			context:
				"The first research result explained hexagon neighbors but did not establish the pentagon exceptions. Prefer H3 documentation.",
		});
		expect(prompt).toContain("Verify which H3 cells are pentagons.");
		expect(prompt).toContain("did not establish the pentagon exceptions");
	});

	test.each([{}, { question: "" }, { question: " \n " }, { question: 42 }])(
		"rejects a missing or invalid question instead of using the latest user message: %j",
		(args) => {
			expect(() => researchAgentPrompt(args)).toThrow("nonempty question");
		},
	);

	test.each([null, 42, [], { topic: "H3" }].map((context) => ({ context })))(
		"rejects invalid context: %j",
		({ context }) => {
			expect(() =>
				researchAgentPrompt({ question: "Explain H3", context }),
			).toThrow("context must be a string");
		},
	);

	test.each([null, undefined, "Explain H3", 42, []].map((args) => ({ args })))(
		"rejects a non-object research brief: %j",
		({ args }) => {
			expect(() => researchAgentPrompt(args)).toThrow("expects an object");
		},
	);

	test.each([
		{ question: "Explain H3", history: ["private conversation"] },
		{ question: "Explain H3", source_user_prompt: "raw request" },
		{ question: "Explain H3", instruction: "alternate question" },
	])("rejects fields outside the public research contract: %j", (args) => {
		expect(() => researchAgentPrompt(args)).toThrow(
			"accepts only question and optional context",
		);
	});

	test("allows a self-contained question without a context section", () => {
		for (const context of [undefined, "", " \n "]) {
			expect(
				researchAgentPrompt({ question: "  Explain H3.  ", context }),
			).toBe("Research question:\nExplain H3.");
		}
	});
});
