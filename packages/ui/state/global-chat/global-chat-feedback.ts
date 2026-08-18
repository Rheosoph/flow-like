import type {
	IChatRunContext,
	IChatUsageStat,
	IMessage,
} from "../../components/interfaces/chat-default/chat-db";
import { IRole } from "../../lib/schema/llm/history";
import type { IProfile } from "../../types";
import type { IApiState } from "../backend-state/api-state";

/** Authenticated, app-less FlowPilot feedback route. Matches `PUT /ai/global-chat/feedback`. */
export const FLOWPILOT_FEEDBACK_PATH = "ai/global-chat/feedback";

/** The app-chat rating scale, kept identical so both surfaces aggregate together. */
export const FLOWPILOT_RATING_POSITIVE = 5;
export const FLOWPILOT_RATING_NEGATIVE = 1;
/** A withdrawn rating. The server deletes the row rather than storing a neutral one. */
export const FLOWPILOT_RATING_WITHDRAWN = 0;

const MAX_TEXT_CHARS = 20_000;
const MAX_HISTORY_MESSAGES = 24;
const MAX_HISTORY_MESSAGE_CHARS = 4_000;
const MAX_STEP_TITLES = 40;
const MAX_STEP_TITLE_CHARS = 200;
const MAX_APP_REFS = 20;
/** Mirrors the route's own limit, so an oversized capture is trimmed here instead of 400ing there. */
const MAX_CONTEXT_BYTES = 256 * 1024;

export interface IFlowPilotFeedbackUsage {
	prompt_tokens: number;
	completion_tokens: number;
	total_tokens: number;
	cost?: number;
	/** Every distinct model that billed against this turn, parent and specialists alike. */
	models: string[];
}

export interface IFlowPilotFeedbackTranscriptEntry {
	role: string;
	content: string;
	timestamp: number;
}

/** The `context` blob stored on the feedback row and rendered by the admin surface. */
export interface IFlowPilotFeedbackContext {
	schema: string;
	message_id: string;
	conversation_id: string;
	/** The user turn this reply answered. */
	prompt: string;
	prompt_message_id?: string;
	/** The rated assistant reply. */
	response: string;
	rated_at: number;
	message_at: number;
	run_context?: IChatRunContext;
	usage?: IFlowPilotFeedbackUsage;
	/** Plan-step titles and tool names — the cheapest read on what the turn actually did. */
	steps?: string[];
	tools?: string[];
	app_refs?: string[];
	attachment_count?: number;
	can_contact?: boolean;
	/** Present only when the user ticked "include chat history" in the feedback dialog. */
	transcript?: IFlowPilotFeedbackTranscriptEntry[];
	/** True when the transcript was trimmed to fit the size budget. */
	transcript_truncated?: boolean;
}

export interface IFlowPilotFeedbackBody {
	feedback_id: string;
	rating: number;
	comment: string;
	context: IFlowPilotFeedbackContext;
}

export function messageText(message: IMessage): string {
	const content = message.inner.content;
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content
		.filter((part) => part.type === "text" && part.text)
		.map((part) => part.text)
		.join("\n");
}

function clip(value: string, max: number): string {
	if (value.length <= max) return value;
	return `${value.slice(0, max)}\n…[truncated]`;
}

function foldUsage(
	stats: IChatUsageStat[] | undefined,
): IFlowPilotFeedbackUsage | undefined {
	if (!stats || stats.length === 0) return undefined;
	const models = new Set<string>();
	let promptTokens = 0;
	let completionTokens = 0;
	let totalTokens = 0;
	let cost = 0;
	let hasCost = false;

	for (const entry of stats) {
		const usage = entry.stats.usage;
		promptTokens += usage?.prompt_tokens ?? 0;
		completionTokens += usage?.completion_tokens ?? 0;
		totalTokens += usage?.total_tokens ?? 0;
		if (typeof usage?.cost === "number") {
			cost += usage.cost;
			hasCost = true;
		}
		// The top-level `model` is only set by some backends; the per-call entries are the reliable
		// source, so collect both rather than trusting either alone.
		if (entry.stats.model) models.add(entry.stats.model);
		for (const call of entry.stats.calls ?? []) {
			if (call.model) models.add(call.model);
			if (typeof call.usage?.cost === "number") {
				cost += call.usage.cost;
				hasCost = true;
			}
		}
	}

	return {
		prompt_tokens: promptTokens,
		completion_tokens: completionTokens,
		total_tokens: totalTokens,
		cost: hasCost ? cost : undefined,
		models: [...models],
	};
}

/**
 * The user turn a reply answered.
 *
 * Prefers the id stamped at send time: steering commits extra user messages with later timestamps,
 * so scanning backwards from the reply can attribute a completely unrelated prompt. The scan is the
 * fallback for messages persisted before run contexts existed.
 */
function owningPrompt(
	message: IMessage,
	messages: readonly IMessage[],
): IMessage | undefined {
	const stampedId = message.run_context?.user_message_id;
	if (stampedId) {
		const stamped = messages.find((candidate) => candidate.id === stampedId);
		if (stamped) return stamped;
	}
	for (let index = messages.length - 1; index >= 0; index -= 1) {
		const candidate = messages[index];
		if (
			candidate.sessionId === message.sessionId &&
			candidate.inner.role === IRole.User &&
			candidate.timestamp <= message.timestamp
		) {
			return candidate;
		}
	}
	return undefined;
}

function buildTranscript(
	message: IMessage,
	messages: readonly IMessage[],
): IFlowPilotFeedbackTranscriptEntry[] {
	return messages
		.filter(
			(candidate) =>
				candidate.sessionId === message.sessionId &&
				candidate.timestamp <= message.timestamp,
		)
		.slice(-MAX_HISTORY_MESSAGES)
		.map((candidate) => ({
			role: String(candidate.inner.role),
			content: clip(messageText(candidate), MAX_HISTORY_MESSAGE_CHARS),
			timestamp: candidate.timestamp,
		}));
}

/**
 * Assemble everything a reviewer needs to judge one rated turn.
 *
 * Lives here rather than in the chat component so `/chat` and the docked overlay — which mount the
 * same body but are separate call sites in principle — can never drift into capturing different
 * things, and so it stays unit-testable without React.
 */
export function buildFlowPilotFeedbackContext(
	message: IMessage,
	messages: readonly IMessage[],
	options?: { includeTranscript?: boolean; canContact?: boolean },
): IFlowPilotFeedbackContext {
	const prompt = owningPrompt(message, messages);
	const context: IFlowPilotFeedbackContext = {
		schema: "flowpilot.feedback/v1",
		message_id: message.id,
		conversation_id: message.sessionId,
		prompt: clip(prompt ? messageText(prompt) : "", MAX_TEXT_CHARS),
		prompt_message_id: prompt?.id,
		response: clip(messageText(message), MAX_TEXT_CHARS),
		rated_at: Date.now(),
		message_at: message.timestamp,
		run_context: message.run_context,
		usage: foldUsage(message.usage_stats),
		attachment_count: prompt?.files?.length ?? 0,
	};

	const steps = (message.plan_steps ?? [])
		.map((step) => step.title)
		.filter(Boolean)
		.slice(0, MAX_STEP_TITLES)
		.map((title) => clip(title, MAX_STEP_TITLE_CHARS));
	if (steps.length > 0) context.steps = steps;

	const tools = [
		...new Set(
			(message.plan_steps ?? [])
				.map((step) => step.toolName)
				.filter((name): name is string => Boolean(name)),
		),
	].slice(0, MAX_STEP_TITLES);
	if (tools.length > 0) context.tools = tools;

	if (message.app_refs && message.app_refs.length > 0) {
		context.app_refs = message.app_refs.slice(0, MAX_APP_REFS);
	}
	if (options?.canContact) context.can_contact = true;
	if (options?.includeTranscript) {
		context.transcript = buildTranscript(message, messages);
	}

	return trimToBudget(context);
}

/**
 * The guarantee that a capture is never rejected by the route for size.
 *
 * Every field above is individually capped, so this should not fire — it exists because "should not"
 * is not "cannot": raising any one of those caps, or a message carrying more parts than expected,
 * would otherwise turn into a silent 400 that loses the rating. Sheds in order of what a reviewer
 * can least afford to lose: transcript entries first, then the transcript, then the response tail.
 */
function trimToBudget(
	context: IFlowPilotFeedbackContext,
): IFlowPilotFeedbackContext {
	const size = (value: IFlowPilotFeedbackContext) =>
		JSON.stringify(value).length;
	let candidate = context;

	while (
		size(candidate) > MAX_CONTEXT_BYTES &&
		candidate.transcript &&
		candidate.transcript.length > 0
	) {
		candidate = {
			...candidate,
			transcript: candidate.transcript.slice(1),
			transcript_truncated: true,
		};
	}

	if (size(candidate) > MAX_CONTEXT_BYTES) {
		const { transcript, ...rest } = candidate;
		candidate = { ...rest, transcript_truncated: Boolean(transcript?.length) };
	}

	if (size(candidate) > MAX_CONTEXT_BYTES) {
		const overflow = size(candidate) - MAX_CONTEXT_BYTES;
		candidate = {
			...candidate,
			response: clip(
				candidate.response,
				Math.max(0, candidate.response.length - overflow - 64),
			),
		};
	}

	return candidate;
}

export function flowPilotRatingForUi(rating: number | undefined): number {
	if (!rating) return FLOWPILOT_RATING_WITHDRAWN;
	return rating > 0 ? FLOWPILOT_RATING_POSITIVE : FLOWPILOT_RATING_NEGATIVE;
}

/**
 * Send one rating. Deliberately has no retry queue: the Dexie message row is the durable local
 * record, and the message id doubles as the idempotency key, so a later re-send simply overwrites.
 */
export async function submitFlowPilotFeedback(
	apiState: IApiState,
	profile: IProfile,
	body: IFlowPilotFeedbackBody,
): Promise<void> {
	await apiState.put(profile, FLOWPILOT_FEEDBACK_PATH, body);
}
