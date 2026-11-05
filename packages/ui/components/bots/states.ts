import type {
	BotAgentState,
	BotEffect,
	BotEmote,
	BotExpression,
	BotMood,
	BotPose,
} from "./types";

export interface BotStatePreset {
	label: string;
	pose: BotPose;
	expression: BotExpression;
	emote: BotEmote;
	effect?: BotEffect;
}

export const BOT_MOODS = {
	"likes-you": {
		label: "Likes you",
		pose: "idle",
		expression: "happy",
		emote: "heart",
	},
	cheerful: {
		label: "Good mood",
		pose: "idle",
		expression: "happy",
		emote: "note",
	},
	love: { label: "In love", pose: "idle", expression: "love", emote: "heart" },
	wink: { label: "Winking", pose: "idle", expression: "wink", emote: "spark" },
	surprised: {
		label: "Surprised",
		pose: "idle",
		expression: "surprised",
		emote: "alert",
	},
	thinking: {
		label: "Thinking",
		pose: "right",
		expression: "curious",
		emote: "question",
	},
	sad: { label: "Sad", pose: "idle", expression: "sad", emote: "rain" },
	angry: { label: "Angry", pose: "idle", expression: "angry", emote: "anger" },
	oops: { label: "Oops", pose: "oops", expression: "oops", emote: "drop" },
	sleeping: {
		label: "Sleeping",
		pose: "sleep",
		expression: "neutral",
		emote: "zzz",
	},
} as const satisfies Record<BotMood, BotStatePreset>;

export const BOT_AGENT_STATES = {
	idle: { label: "Idle" },
	working: {
		label: "Working",
		pose: "center",
		expression: "focused",
		emote: "loading",
	},
	idea: {
		label: "Has an idea",
		pose: "center",
		expression: "surprised",
		emote: "idea",
	},
	done: { label: "Done", pose: "happy", expression: "happy", emote: "check" },
	error: { label: "Error", pose: "oops", expression: "oops", emote: "error" },
	"needs-you": {
		label: "Needs you",
		pose: "center",
		expression: "neutral",
		emote: "alert",
	},
	offline: {
		label: "Offline",
		pose: "sleep",
		expression: "neutral",
		emote: "zzz",
		effect: "offline",
	},
} as const satisfies Record<Exclude<BotAgentState, "idle">, BotStatePreset> & {
	idle: { label: string };
};

export function getBotStatePreset(
	mood?: BotMood,
	agentState?: BotAgentState,
): BotStatePreset | undefined {
	if (agentState && agentState !== "idle") return BOT_AGENT_STATES[agentState];
	return mood ? BOT_MOODS[mood] : undefined;
}
