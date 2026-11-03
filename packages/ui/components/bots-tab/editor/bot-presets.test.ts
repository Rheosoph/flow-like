import { describe, expect, test } from "bun:test";
import bots from "../../../../locales/locales/en/bots.json";
import {
	BOT_CAPABILITIES,
	isBotCapabilityAvailable,
} from "../../../state/backend-state/bot-state";
import { resolveBotAppearance } from "../../bots/appearance";
import { defaultBotCatalog } from "../../bots/catalog";
import { orderBotCapabilities } from "./bot-draft";
import { getBotLookRequirements } from "./bot-look";
import { BOT_PRESETS } from "./bot-presets";

function lookup(key: string): unknown {
	return key
		.split(".")
		.reduce<unknown>(
			(node, part) =>
				node && typeof node === "object"
					? (node as Record<string, unknown>)[part]
					: undefined,
			bots,
		);
}

describe("BOT_PRESETS", () => {
	test("has about eight presets with unique ids", () => {
		const ids = BOT_PRESETS.map((preset) => preset.id);
		expect(ids.length).toBeGreaterThanOrEqual(8);
		expect(new Set(ids).size).toBe(ids.length);
	});

	test.each(BOT_PRESETS.map((preset) => [preset.id, preset] as const))(
		"%s is complete and saveable",
		(_, preset) => {
			for (const key of [
				preset.nameKey,
				preset.descriptionKey,
				preset.instructionsKey,
			])
				expect(typeof lookup(key)).toBe("string");

			const { appearance } = preset;
			expect(Object.hasOwn(defaultBotCatalog.shapes, appearance.kind)).toBe(
				true,
			);
			const resolved = resolveBotAppearance(appearance, defaultBotCatalog);
			expect(resolved.kind).toBe(appearance.kind);
			expect(resolved.equipment).toEqual(appearance.equipment ?? {});
			expect(resolved.skills).toEqual([...(appearance.skills ?? [])]);
			expect(getBotLookRequirements(appearance)).toEqual([]);

			for (const capability of preset.capabilities) {
				expect(BOT_CAPABILITIES).toContain(capability);
				expect(isBotCapabilityAvailable(capability, preset.location)).toBe(
					true,
				);
			}
			expect(orderBotCapabilities(preset.capabilities)).toEqual([
				...preset.capabilities,
			]);
			expect(Object.keys(preset.keywords.en).length).toBeGreaterThan(5);
			expect(Object.keys(preset.keywords.de).length).toBeGreaterThan(5);
		},
	);
});
