import { describe, expect, test } from "bun:test";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import { buildSearchCorpus } from "./use-history-search";

function message(
	sessionId: string,
	content: IMessage["inner"]["content"],
	timestamp: number,
): IMessage {
	return {
		id: `${sessionId}-${timestamp}`,
		appId: "global",
		sessionId,
		inner: { role: "user" as IMessage["inner"]["role"], content },
		files: [],
		timestamp,
	};
}

describe("buildSearchCorpus", () => {
	test("groups messages per conversation", () => {
		const corpus = buildSearchCorpus([
			message("a", "deploy the pipeline", 1),
			message("b", "unrelated", 2),
			message("a", "then check the logs", 3),
		]);

		expect(corpus.get("a")).toBe("then check the logs · deploy the pipeline");
		expect(corpus.get("b")).toBe("unrelated");
	});

	test("flattens structured content parts and drops non-text ones", () => {
		const corpus = buildSearchCorpus([
			message(
				"a",
				[
					{ type: "text" as never, text: "look at" },
					{ type: "image_url" as never, image_url: { url: "http://x" } },
					{ type: "text" as never, text: "this chart" },
				],
				1,
			),
		]);

		expect(corpus.get("a")).toBe("look at this chart");
	});

	test("keeps the newest messages and bounds the indexed text", () => {
		const many = Array.from({ length: 60 }, (_, i) =>
			message("a", `msg${i}`, i),
		);
		const corpus = buildSearchCorpus(many);
		const text = corpus.get("a") ?? "";

		expect(text.startsWith("msg59 · msg58")).toBe(true);
		expect(text).not.toContain("msg0 ");
		expect(text.length).toBeLessThanOrEqual(2000);
	});

	test("is safe on undefined and empty input", () => {
		expect(buildSearchCorpus(undefined).size).toBe(0);
		expect(buildSearchCorpus([]).size).toBe(0);
	});
});
