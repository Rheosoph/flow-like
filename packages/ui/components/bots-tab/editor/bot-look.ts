import { defaultBotCatalog } from "../../bots/catalog";
import {
	type BotSaveRequirement,
	getBotSaveEligibility,
} from "../../bots/save-eligibility";
import type { BotAppearance, BotCatalog } from "../../bots/types";

/**
 * Character packs every account may save. Packs can't be bought yet and every character belongs to
 * one, so the starter pack that holds the catalog's default character is the one everybody owns.
 */
export const BOT_OWNED_PACK_IDS: readonly string[] = ["basics-crew"];

export function getBotLookRequirements(
	appearance: BotAppearance,
	catalog: BotCatalog = defaultBotCatalog,
): BotSaveRequirement[] {
	return getBotSaveEligibility(appearance, catalog, BOT_OWNED_PACK_IDS)
		.requirements;
}

/** "Hoot (Office Crew)": the item and the packs, any one of which unlocks it. */
export function describeBotLookRequirement(
	requirement: BotSaveRequirement,
	catalog: BotCatalog = defaultBotCatalog,
): string {
	const packs = requirement.packIds.flatMap((id) => {
		const pack =
			requirement.kind === "color" ? catalog.colorPacks[id] : catalog.packs[id];
		return pack ? [pack.label] : [];
	});
	return packs.length
		? `${requirement.label} (${packs.join(" / ")})`
		: requirement.label;
}
