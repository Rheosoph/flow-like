import type { BotDraftProblem } from "./bot-draft";

export const BOT_EDITOR_STEPS = [
	"describe",
	"look",
	"abilities",
	"brain",
	"location",
	"safety",
	"secrets",
] as const;
export type BotEditorStep = (typeof BOT_EDITOR_STEPS)[number];
export type BotEditorMode = "create" | "edit";

/** The Describe step's choice when the user starts without a preset. */
export const BOT_SCRATCH_CHOICE = "scratch";

/**
 * Describing only makes sense for a new bot; secrets need a bot that exists. A bot whose host is
 * fixed (Flux) has nothing to pick on "Runs on".
 */
export function getBotEditorSteps(
	mode: BotEditorMode,
	options: { fixedLocation?: boolean } = {},
): BotEditorStep[] {
	return BOT_EDITOR_STEPS.filter((step) => {
		if (step === "location") return !options.fixedLocation;
		return mode === "create" ? step !== "secrets" : step !== "describe";
	});
}

/** Where a problem is fixed; `null` for the name and description, which sit beside every step. */
export function getBotProblemStep(
	problem: BotDraftProblem,
): BotEditorStep | null {
	switch (problem.code) {
		case "look_locked":
		case "look_invalid":
			return "look";
		case "profile_required":
			return "abilities";
		case "device_required":
			return "location";
		case "instructions_too_long":
			return "brain";
		default:
			return null;
	}
}
