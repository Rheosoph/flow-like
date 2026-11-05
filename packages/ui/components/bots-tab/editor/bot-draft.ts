import { isEqual } from "lodash-es";
import type { IDataSensitivity } from "../../../lib/data-classification";
import {
	BOT_CAPABILITIES,
	BOT_LIMITS,
	type BotCapability,
	type BotLocation,
	type BotSystemKey,
	FLOWPILOT_CAPABILITY,
	type IBot,
	type IBotModelPolicy,
	type ICreateBotRequest,
	type IUpdateBotRequest,
	MEMORY_CAPABILITY,
	isBotCapabilityAvailable,
} from "../../../state/backend-state/bot-state";
import { defaultBotCatalog } from "../../bots/catalog";
import type { BotAppearance } from "../../bots/types";
import { describeBotLookRequirement, getBotLookRequirements } from "./bot-look";
import type { BotPreset } from "./bot-presets";

/** The draft fields that pin a media model of the bot. */
export type BotModelPin =
	| "sttModel"
	| "ttsModel"
	| "imageModel"
	| "visionModel"
	| "subtaskModel";

export interface BotDraft {
	presetId: string | null;
	name: string;
	description: string;
	instructions: string;
	appearance: BotAppearance;
	model: IBotModelPolicy;
	/** Bit id of the embedding model its memory uses; `null` lets the host choose. */
	embeddingModel: string | null;
	/** Bit ids of the speech-to-text, text-to-speech, image generation and vision models it pins. */
	sttModel: string | null;
	ttsModel: string | null;
	imageModel: string | null;
	visionModel: string | null;
	/** Bit id of the model its sub-tasks run on; `null` uses its own model. */
	subtaskModel: string | null;
	classification: IDataSensitivity;
	capabilities: BotCapability[];
	profileId: string | null;
	location: BotLocation;
	deviceId: string | null;
}

/**
 * Flux and its duplicates: the complete FlowPilot toolset, so their abilities are fixed. Flux rows
 * from before the `flowpilot` capability existed don't list it, so the built-in key counts too.
 */
export function hasFlowPilot(bot: {
	system: BotSystemKey | null;
	capabilities: readonly BotCapability[];
}): boolean {
	return (
		bot.system === "flux" || bot.capabilities.includes(FLOWPILOT_CAPABILITY)
	);
}

/** Flux keeps its host as well as its abilities; its duplicates may move. */
export function hasFixedLocation(system: BotSystemKey | null): boolean {
	return system === "flux";
}

/** The localized texts a preset puts into a draft. */
export interface BotPresetTexts {
	name: string;
	description: string;
	instructions: string;
}

export type BotDraftProblem =
	| { code: "name_required" }
	| { code: "name_too_long"; max: number }
	| { code: "description_too_long"; max: number }
	| { code: "instructions_too_long"; max: number }
	| { code: "look_locked"; items: string[] }
	| { code: "look_invalid"; items: string[] }
	| { code: "profile_required" }
	| { code: "device_required" };

const text = (value: string | null | undefined) => value?.trim() || null;

/** Switched on in every new bot, blank or from a preset. */
export const NEW_BOT_CAPABILITIES: readonly BotCapability[] = [
	MEMORY_CAPABILITY,
];

export function orderBotCapabilities(
	capabilities: Iterable<BotCapability>,
): BotCapability[] {
	const chosen = new Set(capabilities);
	return BOT_CAPABILITIES.filter((capability) => chosen.has(capability));
}

export function normalizeBotModelPolicy(
	model: IBotModelPolicy | null | undefined,
): IBotModelPolicy {
	const primary = text(model?.primary);
	const fallbacks = [...new Set(model?.fallbacks ?? [])]
		.map((id) => id.trim())
		.filter((id) => id && id !== primary)
		.slice(0, BOT_LIMITS.fallbacks);
	return { primary, fallbacks };
}

export function normalizeBotClassification(
	value: IDataSensitivity | null | undefined,
): IDataSensitivity {
	return {
		...(value?.classification ? { classification: value.classification } : {}),
		pii: value?.pii === true,
	};
}

export function createEmptyBotDraft(): BotDraft {
	return {
		presetId: null,
		name: "",
		description: "",
		instructions: "",
		appearance: { kind: defaultBotCatalog.defaultKind },
		model: { primary: null, fallbacks: [] },
		embeddingModel: null,
		sttModel: null,
		ttsModel: null,
		imageModel: null,
		visionModel: null,
		subtaskModel: null,
		classification: { pii: false },
		capabilities: [...NEW_BOT_CAPABILITIES],
		profileId: null,
		location: "server",
		deviceId: null,
	};
}

/** The draft on that managed device; unchanged without a device id. */
export function botDraftOnDevice(draft: BotDraft, deviceId?: string): BotDraft {
	if (!deviceId) return draft;
	return { ...setBotDraftLocation(draft, "device").draft, deviceId };
}

export function botDraftFromBot(bot: IBot): BotDraft {
	return {
		presetId: bot.preset_id,
		name: bot.name ?? "",
		description: bot.description ?? "",
		instructions: bot.instructions ?? "",
		appearance: structuredClone(bot.appearance),
		model: normalizeBotModelPolicy(bot.model),
		embeddingModel: text(bot.embedding_model),
		sttModel: text(bot.stt_model),
		ttsModel: text(bot.tts_model),
		imageModel: text(bot.image_model),
		visionModel: text(bot.vision_model),
		subtaskModel: text(bot.subtask_model),
		classification: normalizeBotClassification(bot.classification),
		capabilities: orderBotCapabilities(bot.capabilities),
		profileId: bot.profile_id,
		location: bot.location,
		deviceId: bot.device_id,
	};
}

export function botDraftFromPreset(
	preset: BotPreset,
	texts: BotPresetTexts,
): BotDraft {
	return {
		...createEmptyBotDraft(),
		presetId: preset.id,
		name: texts.name,
		description: texts.description,
		instructions: texts.instructions,
		appearance: structuredClone(preset.appearance),
		classification: normalizeBotClassification(preset.classification),
		capabilities: orderBotCapabilities([
			...NEW_BOT_CAPABILITIES,
			...preset.capabilities.filter((capability) =>
				isBotCapabilityAvailable(capability, preset.location),
			),
		]),
		location: preset.location,
	};
}

/** `flowpilot` is never switched on or off, and a bot that has it keeps every other ability as is. */
export function setBotDraftCapability(
	draft: BotDraft,
	capability: BotCapability,
	enabled: boolean,
): BotDraft {
	if (
		capability === FLOWPILOT_CAPABILITY ||
		draft.capabilities.includes(FLOWPILOT_CAPABILITY)
	)
		return draft;
	if (enabled && !isBotCapabilityAvailable(capability, draft.location))
		return draft;
	const next = new Set(draft.capabilities);
	if (enabled) next.add(capability);
	else next.delete(capability);
	return { ...draft, capabilities: orderBotCapabilities(next) };
}

export interface BotLocationMove {
	draft: BotDraft;
	/** Switched off because the new host can't do them. */
	removed: BotCapability[];
	/** Switched off by an earlier move and back on, because the new host can do them. */
	restored: BotCapability[];
	/** Everything switched off by moves and not back yet; pass it to the next move. */
	remembered: BotCapability[];
}

/** Moves the bot: switches off what the new host can't do and back on what an earlier move switched off. */
export function setBotDraftLocation(
	draft: BotDraft,
	location: BotLocation,
	remembered: readonly BotCapability[] = [],
): BotLocationMove {
	const allowed = (capability: BotCapability) =>
		isBotCapabilityAvailable(capability, location);
	const removed = draft.capabilities.filter(
		(capability) => !allowed(capability),
	);
	const restored = orderBotCapabilities(
		remembered.filter(
			(capability) =>
				allowed(capability) && !draft.capabilities.includes(capability),
		),
	);
	return {
		draft: {
			...draft,
			location,
			capabilities: orderBotCapabilities([
				...draft.capabilities.filter(allowed),
				...restored,
			]),
		},
		removed,
		restored,
		remembered: orderBotCapabilities([
			...remembered.filter((capability) => !allowed(capability)),
			...removed,
		]),
	};
}

/** A bot without an account runs on this computer; the desktop allows every ability. */
export function keepOnThisComputer(draft: BotDraft): BotDraft {
	if (draft.location === "desktop" && draft.deviceId === null) return draft;
	return { ...draft, location: "desktop", deviceId: null };
}

/** Every field a save may send, in the form the API compares. */
type BotFields = Required<IUpdateBotRequest>;

/** Ids only count while they are used: a profile for `profile_apps`, a device for a device bot. */
function draftFields(draft: BotDraft): BotFields {
	const ordered = orderBotCapabilities(draft.capabilities);
	return {
		name: text(draft.name),
		description: text(draft.description),
		instructions: text(draft.instructions),
		appearance: draft.appearance,
		model: normalizeBotModelPolicy(draft.model),
		embedding_model: text(draft.embeddingModel),
		stt_model: text(draft.sttModel),
		tts_model: text(draft.ttsModel),
		image_model: text(draft.imageModel),
		vision_model: text(draft.visionModel),
		subtask_model: text(draft.subtaskModel),
		classification: normalizeBotClassification(draft.classification),
		capabilities: ordered,
		profile_id: ordered.includes("profile_apps") ? draft.profileId : null,
		location: draft.location,
		device_id: draft.location === "device" ? draft.deviceId : null,
	};
}

function botFields(bot: IBot): BotFields {
	return {
		name: text(bot.name),
		description: text(bot.description),
		instructions: text(bot.instructions),
		appearance: bot.appearance,
		model: normalizeBotModelPolicy(bot.model),
		embedding_model: text(bot.embedding_model),
		stt_model: text(bot.stt_model),
		tts_model: text(bot.tts_model),
		image_model: text(bot.image_model),
		vision_model: text(bot.vision_model),
		subtask_model: text(bot.subtask_model),
		classification: normalizeBotClassification(bot.classification),
		capabilities: orderBotCapabilities(bot.capabilities),
		profile_id: bot.profile_id ?? null,
		location: bot.location,
		device_id: bot.device_id ?? null,
	};
}

/** The draft with what this bot's owner can't change put back the way the bot has it. */
function withFixedFields(bot: IBot, draft: BotDraft): BotDraft {
	const kept = hasFlowPilot(bot)
		? { ...draft, capabilities: bot.capabilities }
		: draft;
	return hasFixedLocation(bot.system)
		? { ...kept, location: bot.location, deviceId: bot.device_id }
		: kept;
}

/** The host picks the memory and pinned models unless the draft names one. */
export function buildCreateBotRequest(draft: BotDraft): ICreateBotRequest {
	const {
		embedding_model,
		stt_model,
		tts_model,
		image_model,
		vision_model,
		subtask_model,
		...fields
	} = draftFields(draft);
	return {
		...fields,
		...(embedding_model ? { embedding_model } : {}),
		...(stt_model ? { stt_model } : {}),
		...(tts_model ? { tts_model } : {}),
		...(image_model ? { image_model } : {}),
		...(vision_model ? { vision_model } : {}),
		...(subtask_model ? { subtask_model } : {}),
		name: draft.name.trim(),
		preset_id: draft.presetId,
	};
}

/**
 * Only what changed; `null` clears a text, which returns a built-in bot to its default name.
 * FlowPilot bots never send abilities, and Flux never sends its host.
 */
export function buildUpdateBotRequest(bot: IBot, draft: BotDraft) {
	const before = botFields(bot);
	const after = draftFields(withFixedFields(bot, draft));
	const changed = (Object.keys(after) as (keyof BotFields)[]).filter(
		(key) => !isEqual(before[key], after[key]),
	);
	return Object.fromEntries(
		changed.map((key) => [key, after[key]]),
	) as IUpdateBotRequest;
}

function lookProblems(appearance: BotAppearance) {
	const locked: string[] = [];
	const invalid: string[] = [];
	for (const requirement of getBotLookRequirements(appearance)) {
		const target = requirement.packIds.length ? locked : invalid;
		target.push(describeBotLookRequirement(requirement));
	}
	const problems: BotDraftProblem[] = [];
	if (locked.length) problems.push({ code: "look_locked", items: locked });
	if (invalid.length) problems.push({ code: "look_invalid", items: invalid });
	return problems;
}

function textProblems(draft: BotDraft, system: BotSystemKey | null) {
	const problems: BotDraftProblem[] = [];
	const name = draft.name.trim();
	if (!name && !system) problems.push({ code: "name_required" });
	if (name.length > BOT_LIMITS.name)
		problems.push({ code: "name_too_long", max: BOT_LIMITS.name });
	if (draft.description.trim().length > BOT_LIMITS.description)
		problems.push({
			code: "description_too_long",
			max: BOT_LIMITS.description,
		});
	if (draft.instructions.trim().length > BOT_LIMITS.instructions)
		problems.push({
			code: "instructions_too_long",
			max: BOT_LIMITS.instructions,
		});
	return problems;
}

/**
 * What blocks creating or saving. Built-in bots may go without a name (they show their default),
 * and an untouched look isn't rechecked, so an older look never blocks saving other changes.
 */
export function getBotDraftProblems(
	draft: BotDraft,
	options: { system: BotSystemKey | null; checkLook: boolean },
): BotDraftProblem[] {
	const problems = textProblems(draft, options.system);
	if (options.checkLook) problems.push(...lookProblems(draft.appearance));
	if (draft.capabilities.includes("profile_apps") && !draft.profileId)
		problems.push({ code: "profile_required" });
	if (draft.location === "device" && !draft.deviceId)
		problems.push({ code: "device_required" });
	return problems;
}
