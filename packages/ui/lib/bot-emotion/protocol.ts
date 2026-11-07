import type { BotExpression } from "../../components/bots/types";

export const BOT_EMOTION_MODEL = "Zeolit/lettuce-thymos-26m-v1";
export const BOT_EMOTION_REVISION = "57219928da2daf26201012d0aafcc3d58dea74d0";
/** Training length of the pinned ONNX graph, including the special tokens. */
export const BOT_EMOTION_MAX_LENGTH = 96;
/** The graph returns these 28 GoEmotions labels, in this order. */
export const BOT_EMOTION_LABELS = [
	"admiration",
	"amusement",
	"anger",
	"annoyance",
	"approval",
	"caring",
	"confusion",
	"curiosity",
	"desire",
	"disappointment",
	"disapproval",
	"disgust",
	"embarrassment",
	"excitement",
	"fear",
	"gratitude",
	"grief",
	"joy",
	"love",
	"nervousness",
	"optimism",
	"pride",
	"realization",
	"relief",
	"remorse",
	"sadness",
	"surprise",
	"neutral",
] as const;
export const MAX_DRAFT_CHARS = 768;
export const MAX_CONTEXT_CHARS = 384;
export const MAX_CONTEXT_MESSAGES = 4;

export interface EmotionMessage {
	role: "user" | "assistant";
	content: string;
}

export type BotEmotionMessage = EmotionMessage;

export interface EmotionInput {
	draft: string;
	messages: readonly EmotionMessage[];
}

export type EmotionScores = Record<string, number>;
export type ExpressionScores = Record<BotExpression, number>;
export type EmotionStatus = "idle" | "loading" | "ready" | "unavailable";

export interface EmotionLoadState {
	status: EmotionStatus;
	/** Download completion in [0, 1], when the server provides a total size. */
	progress?: number;
}

export interface EmotionWorkerRequest extends EmotionInput {
	id: number;
	warmup?: boolean;
}

export type EmotionWorkerResponse =
	| { type: "progress"; progress?: number }
	| { type: "ready" }
	| { type: "result"; id: number; scores: ExpressionScores }
	| { type: "error"; id: number };

/** Read the first row of sigmoid probabilities from the emotion graph. */
export function scoresFromOutput(values: ArrayLike<number>): EmotionScores {
	const width = BOT_EMOTION_LABELS.length;
	if (values.length < width || values.length % width !== 0) {
		throw new Error(`Emotion model returned ${values.length} scores`);
	}
	const scores: EmotionScores = {};
	for (let index = 0; index < width; index++) {
		scores[BOT_EMOTION_LABELS[index]] = Number(values[index]);
	}
	return scores;
}

export function boundEmotionInput(input: EmotionInput): EmotionInput {
	return {
		draft: input.draft.trim().slice(-MAX_DRAFT_CHARS),
		messages: input.messages
			.filter((message) => message.content.trim())
			.slice(-MAX_CONTEXT_MESSAGES)
			.map((message) => ({
				role: message.role,
				content: message.content.trim().slice(-MAX_CONTEXT_CHARS),
			})),
	};
}
