import { expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import { isBotDecisionModel } from "./decision-model-options";

function bit(provider: string, type = IBitTypes.SystemOne): IBit {
	return {
		id: "decision",
		type,
		parameters: { provider: { provider_name: provider } },
	} as IBit;
}

test.each(["server", "device"] as const)(
	"%s bots only offer supported hosted System One models",
	(location) => {
		for (const provider of [
			"hosted:openrouter",
			"hosted:typesafe",
			"hosted:cloudflare",
			"hosted:systemone_compatible",
		])
			expect(isBotDecisionModel(bit(provider), location)).toBe(true);
		for (const provider of [
			"local",
			"device",
			"custom:systemone",
			"custom:typesafe",
			"custom:openrouter",
			"hosted:openai",
		])
			expect(isBotDecisionModel(bit(provider), location)).toBe(false);
		expect(
			isBotDecisionModel(bit("hosted:openrouter", IBitTypes.Llm), location),
		).toBe(false);
	},
);

test("desktop bots also offer local, device and custom decision models", () => {
	for (const provider of [
		"local",
		"device",
		"custom:systemone",
		"custom:typesafe",
		"custom:openrouter",
		"hosted:typesafe",
	])
		expect(isBotDecisionModel(bit(provider), "desktop")).toBe(true);
});
