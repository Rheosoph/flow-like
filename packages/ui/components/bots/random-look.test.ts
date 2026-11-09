import { describe, expect, it } from "bun:test";
import { defaultBotCatalog as catalog } from "./catalog";
import { getBotColorSelectionPaint } from "./colors";
import { BOT_LOOK_PARTS, randomizeBotLook } from "./random-look";
import type { BotAppearance } from "./types";

function seeded(seed: number) {
	let state = seed;
	return () => {
		state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
		return state / 0x100000000;
	};
}

describe("randomizeBotLook", () => {
	const look: BotAppearance = {
		kind: catalog.defaultKind,
		frame: "squircle",
		backdrop: "stars",
		emote: "heart",
		skills: ["code"],
		mute_notifications: true,
	};

	it("returns the look unchanged when every part is locked", () => {
		expect(
			randomizeBotLook({
				value: look,
				catalog,
				parts: BOT_LOOK_PARTS,
				locked: BOT_LOOK_PARTS,
				random: seeded(7),
			}),
		).toEqual(look);
	});

	it("keeps rerolled colors solid beside a kept backdrop", () => {
		for (let seed = 1; seed <= 64; seed++) {
			const next = randomizeBotLook({
				value: look,
				catalog,
				parts: ["colors"],
				random: seeded(seed),
			});
			expect(next).toMatchObject({
				kind: look.kind,
				frame: "squircle",
				backdrop: "stars",
				emote: "heart",
				skills: ["code"],
				mute_notifications: true,
			});
			for (const selection of Object.values(next.colors ?? {}))
				expect(getBotColorSelectionPaint(selection, catalog)?.kind).toBe(
					"solid",
				);
		}
	});

	it("is deterministic for a seed and omits cleared fields", () => {
		const run = () =>
			randomizeBotLook({
				value: { ...look, dark: true, variant: "gold" },
				catalog,
				parts: BOT_LOOK_PARTS,
				random: seeded(42),
			});
		const next = run();
		expect(next).toEqual(run());
		expect(next.kind).not.toBe(look.kind);
		expect("dark" in next).toBe(false);
		expect("variant" in next).toBe(false);
		expect(JSON.parse(JSON.stringify(next))).toEqual(next);
	});
});
