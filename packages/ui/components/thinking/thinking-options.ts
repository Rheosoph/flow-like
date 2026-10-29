import type { CopilotReasoningEffort } from "../flowpilot/types";

/** Glyph intensity: 0 is a grey dot, each step adds an orbit and speed, 4 is reserved for "extra high". */
export type ThinkingGlyphLevel = 0 | 1 | 2 | 3 | 4;

export interface ThinkingOption {
	/** The value sent to the backend; "" is the provider's own default. */
	readonly id: string;
	readonly label: string;
	readonly description?: string;
	readonly level: ThinkingGlyphLevel;
	readonly auto?: boolean;
}

export interface ThinkingLabels {
	readonly off: string;
	readonly low: string;
	readonly mid: string;
	readonly high: string;
	readonly extraHigh: string;
	readonly auto: string;
	readonly autoDescription: string;
	readonly autoWithDefault: (level: string) => string;
	readonly describe: (level: ThinkingGlyphLevel) => string;
}

export interface ThinkingCapableModel {
	readonly supportedReasoningEfforts?: readonly CopilotReasoningEffort[];
	readonly defaultReasoningEffort?: string;
}

/** The levels `History.thinking` understands, in the order the menu lists them. */
export const CHAT_THINKING_LEVELS = ["off", "low", "mid", "high"] as const;
export type ChatThinkingLevel = (typeof CHAT_THINKING_LEVELS)[number];

/** Used whenever no level was picked; the Chat Event node falls back to the same value. */
export const DEFAULT_CHAT_THINKING: ChatThinkingLevel = "low";

export function isChatThinkingLevel(
	value: unknown,
): value is ChatThinkingLevel {
	return (
		typeof value === "string" &&
		(CHAT_THINKING_LEVELS as readonly string[]).includes(value)
	);
}

export function chatThinkingLevel(value: unknown): ChatThinkingLevel {
	return isChatThinkingLevel(value) ? value : DEFAULT_CHAT_THINKING;
}

const EFFORT_LEVELS: ReadonlyMap<string, ThinkingGlyphLevel> = new Map<
	string,
	ThinkingGlyphLevel
>([
	["none", 0],
	["off", 0],
	["minimal", 0],
	["low", 1],
	["mid", 2],
	["medium", 2],
	["high", 3],
	["xhigh", 4],
	["extra_high", 4],
	["extra-high", 4],
	["max", 4],
]);

/** Maps a provider's native effort id onto the glyph; unknown ids sit in the middle. */
export function thinkingLevelForEffort(id: string): ThinkingGlyphLevel {
	return EFFORT_LEVELS.get(id.trim().toLowerCase()) ?? 2;
}

export function chatThinkingOptions(labels: ThinkingLabels): ThinkingOption[] {
	return CHAT_THINKING_LEVELS.map((id, index) => ({
		id,
		label: labels[id],
		description: labels.describe(index as ThinkingGlyphLevel),
		level: index as ThinkingGlyphLevel,
	}));
}

/**
 * An agent backend's own effort list, led by "Auto" (the provider's default). Empty when the
 * model advertises no configurable effort, so the caller can hide the picker.
 */
export function agentThinkingOptions(
	model: ThinkingCapableModel | undefined,
	labels: ThinkingLabels,
): ThinkingOption[] {
	const efforts = model?.supportedReasoningEfforts ?? [];
	if (efforts.length === 0) return [];
	const defaultEffort = efforts.find(
		(effort) => effort.id === model?.defaultReasoningEffort,
	);
	return [
		{
			id: "",
			label: labels.auto,
			description: defaultEffort
				? labels.autoWithDefault(defaultEffort.name || defaultEffort.id)
				: labels.autoDescription,
			level: defaultEffort ? thinkingLevelForEffort(defaultEffort.id) : 2,
			auto: true,
		},
		...efforts.map((effort) => {
			const level = thinkingLevelForEffort(effort.id);
			return {
				id: effort.id,
				label: effort.name || effort.id,
				description: effort.description || labels.describe(level),
				level,
			};
		}),
	];
}

/**
 * The level a turn is stamped with: agent backends keep their native id (nothing for "Auto",
 * where the provider chose), the native Bits path always runs a `History.thinking` level.
 */
export function turnThinking(
	isAgent: boolean,
	effort: string,
): string | undefined {
	if (isAgent) return effort || undefined;
	return chatThinkingLevel(effort);
}

/** Display name of a stamped level; an agent backend's own id is shown as a readable word. */
export function thinkingStampLabel(
	value: string,
	labels: ThinkingLabels,
): string {
	if (isChatThinkingLevel(value)) return labels[value];
	if (EXTRA_HIGH_EFFORTS.has(value.trim().toLowerCase())) {
		return labels.extraHigh;
	}
	const words = value.replace(/[-_]+/g, " ").trim();
	return words.charAt(0).toUpperCase() + words.slice(1);
}

const EXTRA_HIGH_EFFORTS = new Set(["xhigh", "extra_high", "extra-high"]);

/** Whether a stored effort still fits the current provider and model. */
export function isValidReasoningEffort(
	isAgent: boolean,
	effort: string,
	model: ThinkingCapableModel | undefined,
): boolean {
	if (!effort) return true;
	if (!isAgent) return isChatThinkingLevel(effort);
	return (model?.supportedReasoningEfforts ?? []).some(
		(option) => option.id === effort,
	);
}
