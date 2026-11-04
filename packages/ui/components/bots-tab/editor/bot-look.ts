import { defaultBotCatalog } from "../../bots/catalog";
import {
	type BotSaveRequirement,
	getBotSaveEligibility,
} from "../../bots/save-eligibility";
import type { BotAppearance, BotCatalog } from "../../bots/types";

/** All catalog cosmetics are available until purchasing exists. Invalid selections still need repair. */
export function getBotLookRequirements(
	appearance: BotAppearance,
	catalog: BotCatalog = defaultBotCatalog,
): BotSaveRequirement[] {
	return getBotSaveEligibility(appearance, catalog).requirements;
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
