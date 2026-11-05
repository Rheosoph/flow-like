import type {
	IBot,
	IBotMoveIssue,
	IUpdateBotRequest,
} from "../../state/backend-state/bot-state";
import { BOT_SERVER_LIMIT_REACHED } from "./editor/bot-limits";
import {
	describeBotLookRequirement,
	getBotLookRequirements,
} from "./editor/bot-look";

export const BOT_MODEL_UNAVAILABLE = "BOT_MODEL_UNAVAILABLE";
export const BOT_LOOK_NOT_OWNED = "BOT_LOOK_NOT_OWNED";
/** A secret this device's key no longer opens; `ref_id` is its name. */
export const BOT_SECRET_UNREADABLE = "BOT_SECRET_UNREADABLE";
/** The bot kept changing on this computer while it moved. */
export const LOCAL_CHANGED = "LOCAL_CHANGED";
/** This computer's keychain is locked or refused: unlock it and try again; nothing to remove. */
export const SECRET_KEY = "SECRET_KEY";

/** Refusals no setting can fix: the bot keeps living on this computer. */
const STAYS_HERE: ReadonlySet<string> = new Set([
	"BOT_LIMIT_REACHED",
	BOT_SERVER_LIMIT_REACHED,
]);

export function staysOnThisComputer(issue: IBotMoveIssue): boolean {
	return STAYS_HERE.has(issue.code);
}

/** The bot's models, the primary first, each once. */
function botModelIds(bot: IBot): string[] {
	const ids = [bot.model.primary, ...bot.model.fallbacks];
	return [...new Set(ids.filter((id): id is string => Boolean(id)))];
}

/**
 * What the account must offer that the server doesn't check: every model the bot names and
 * every pack its look uses. A look issue's `message` lists the items.
 */
export function clientMoveIssues(
	bot: IBot,
	modelIds: ReadonlySet<string>,
): IBotMoveIssue[] {
	const issues: IBotMoveIssue[] = botModelIds(bot)
		.filter((id) => !modelIds.has(id))
		.map((id) => ({
			part: "model",
			code: BOT_MODEL_UNAVAILABLE,
			message: `Model ${id} isn't available to this account`,
			ref_id: id,
		}));
	const look = getBotLookRequirements(bot.appearance);
	if (look.length > 0) {
		issues.push({
			part: "look",
			code: BOT_LOOK_NOT_OWNED,
			message: look
				.map((requirement) => describeBotLookRequirement(requirement))
				.join(", "),
			ref_id: null,
		});
	}
	return issues;
}

/** Server and client issues together, each once. */
export function mergeMoveIssues(
	...lists: readonly (readonly IBotMoveIssue[])[]
): IBotMoveIssue[] {
	const seen = new Set<string>();
	return lists.flat().filter((issue) => {
		const key = `${issue.part}\u0000${issue.code}\u0000${issue.ref_id ?? ""}`;
		if (seen.has(key)) return false;
		seen.add(key);
		return true;
	});
}

export type MoveFix =
	| { kind: "detach_app"; appId: string }
	| { kind: "use_profile" }
	| { kind: "drop_profile_apps" }
	| { kind: "automatic_model" }
	| { kind: "drop_fallback"; modelId: string }
	| { kind: "remove_secret"; name: string }
	| { kind: "open_settings" };

function modelFixes(modelId: string | null, bot: IBot): MoveFix[] {
	if (modelId && modelId === bot.model.primary)
		return [{ kind: "automatic_model" }, { kind: "open_settings" }];
	if (modelId && bot.model.fallbacks.includes(modelId))
		return [{ kind: "drop_fallback", modelId }, { kind: "open_settings" }];
	return [{ kind: "open_settings" }];
}

/** What the person can do about an issue right in the review; nothing when the bot stays here. */
export function moveFixes(issue: IBotMoveIssue, bot: IBot): MoveFix[] {
	if (staysOnThisComputer(issue)) return [];
	const ref = issue.ref_id;
	switch (issue.code) {
		case "BOT_APP_UNAVAILABLE":
			return ref
				? [{ kind: "detach_app", appId: ref }]
				: [{ kind: "open_settings" }];
		case "BOT_PROFILE_UNAVAILABLE":
			return [{ kind: "use_profile" }, { kind: "drop_profile_apps" }];
		case BOT_MODEL_UNAVAILABLE:
			return modelFixes(ref, bot);
		case BOT_SECRET_UNREADABLE:
			return ref
				? [{ kind: "remove_secret", name: ref }]
				: [{ kind: "open_settings" }];
		default:
			return [{ kind: "open_settings" }];
	}
}

/** The bot update a fix makes; `null` for fixes that aren't a bot update. */
export function moveFixRequest(
	fix: MoveFix,
	bot: IBot,
	profileId: string | null,
): IUpdateBotRequest | null {
	switch (fix.kind) {
		case "use_profile":
			return profileId ? { profile_id: profileId } : null;
		case "drop_profile_apps":
			return {
				capabilities: bot.capabilities.filter(
					(capability) => capability !== "profile_apps",
				),
				profile_id: null,
			};
		case "automatic_model":
			return { model: { primary: null, fallbacks: bot.model.fallbacks } };
		case "drop_fallback":
			return {
				model: {
					primary: bot.model.primary,
					fallbacks: bot.model.fallbacks.filter((id) => id !== fix.modelId),
				},
			};
		default:
			return null;
	}
}
