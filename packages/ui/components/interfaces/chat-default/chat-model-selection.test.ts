import "fake-indexeddb/auto";
import { describe, expect, test } from "vitest";
import { type IBit, IBitTypes } from "../../../lib/schema/bit/bit";
import {
	chatModelOptions,
	readChatModel,
	storeChatModel,
} from "./chat-model-selection";

const browser = { canHostLlamaCPP: false, canHostMLX: false };

function model(id: string, type = IBitTypes.Llm, provider = "Hosted"): IBit {
	return {
		id,
		type,
		hub: "catalog.example",
		parameters: { provider: { provider_name: provider } },
	} as IBit;
}

describe("app chat model choices", () => {
	test("offers active text and vision models that the execution host can run", () => {
		const bits = [
			model("text"),
			model("vision", IBitTypes.Vlm),
			model("local", IBitTypes.Llm, "Local"),
			model("inactive"),
			model("embedding", IBitTypes.Embedding),
		];
		expect(
			chatModelOptions(
				bits,
				["hub:text", "other-hub:vision", "hub:local", "hub:embedding"],
				browser,
				false,
			).map((bit) => bit.id),
		).toEqual(["text", "vision"]);
	});

	test("restricts choices to VLMs only when vision is required", () => {
		const bits = [model("text"), model("vision", IBitTypes.Vlm)];
		expect(
			chatModelOptions(bits, ["hub:text", "hub:vision"], browser, true).map(
				(bit) => bit.id,
			),
		).toEqual(["vision"]);
		expect(chatModelOptions(undefined, ["hub:vision"], browser, true)).toEqual(
			[],
		);
	});

	test("remembers the choice by app and clears it for Automatic", async () => {
		await storeChatModel("first-app", "vision");
		await storeChatModel("second-app", "text");
		expect(await readChatModel("first-app")).toBe("vision");
		expect(await readChatModel("second-app")).toBe("text");
		await storeChatModel("first-app", undefined);
		expect(await readChatModel("first-app")).toBeUndefined();
		expect(await readChatModel("second-app")).toBe("text");
	});
});
