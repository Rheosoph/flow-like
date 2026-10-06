/*
 * One run's intercom events, read as they arrive (executeEvent's onLiveEvents, each event once).
 * Chat events go through the chat's own processor so steps, text and files parse exactly as in
 * the chat; a2ui events never reach it (its a2ui branch answers widget queries). Everything the
 * old form guessed from "the last event" is gone: a result comes only from a result event.
 */
import { Response as ChatResponse } from "../../../../lib/llm/response";
import type { IIntercomEvent } from "../../../../lib/schema/events/intercom-event";
import type { IInteractionRequest } from "../../../../lib/schema/interaction";
import {
	type IContent,
	type IHistoryMessage,
	IRole,
} from "../../../../lib/schema/llm/history";
import type { IAttachment, IMessage } from "../../chat-default/chat-db";
import { processChatEvents } from "../../chat-default/event-processor";
import { joinContentText } from "../../chat-default/inline-segments";
import type {
	CreateRunAccumulator,
	NavigateIntent,
	RunAccumulator,
	RunOutput,
	TerminalSignals,
} from "../contracts";
import { toRunSteps } from "./steps";

type Writable<T> = { -readonly [K in keyof T]: T[K] };
type CompletedStatus = NonNullable<TerminalSignals["completedStatus"]>;
type JsonRecord = Record<string, unknown>;

/** Prefix of `completed.payload.current_step` when the executor refused the run before it started. */
const REJECTED_PREFIX = "rejected:";
const CHAT_SESSION_ID = "form-workbench";
const CHAT_EVENTS: ReadonlySet<string> = new Set([
	"chat_stream_partial",
	"chat_stream",
	"chat_out",
]);

const COMPLETED_STATUS: ReadonlyMap<string, CompletedStatus> = new Map([
	["completed", "completed"],
	["success", "completed"],
	["succeeded", "completed"],
	["cancelled", "cancelled"],
	["canceled", "cancelled"],
	["timeout", "timeout"],
	["timed_out", "timeout"],
	["failed", "failed"],
]);

/** Terminal signals of a run that sent nothing (yet). */
export const NO_TERMINAL_SIGNALS: TerminalSignals = {
	runInitiated: false,
	errorMessage: null,
	completedStatus: null,
	rejectedStage: null,
	logLevel: null,
	durationMs: null,
};

interface ReaderState {
	eventCount: number;
	sawPlan: boolean;
	directText: string;
	result: { readonly value: unknown } | null;
	readonly terminal: Writable<TerminalSignals>;
	readonly streamAttachments: Map<string, IAttachment>;
	readonly interactions: Map<string, IInteractionRequest>;
	readonly navigation: NavigateIntent[];
}

interface ChatState {
	response: ChatResponse;
	readonly message: IMessage;
	readonly attachments: Map<string, IAttachment>;
	done: boolean;
}

type EventReader = (state: ReaderState, payload: unknown) => void;

const isRecord = (value: unknown): value is JsonRecord =>
	typeof value === "object" && value !== null && !Array.isArray(value);

function finiteOrNull(value: unknown) {
	return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function urlOf(attachment: unknown) {
	if (typeof attachment === "string") return attachment || null;
	if (!isRecord(attachment) || typeof attachment.url !== "string") return null;
	return attachment.url || null;
}

/** Merge by URL: the first full record wins, a record replaces a bare URL string. */
function addAttachment(into: Map<string, IAttachment>, attachment: unknown) {
	const url = urlOf(attachment);
	if (!url) return;
	const previous = into.get(url);
	if (
		previous === undefined ||
		(typeof previous === "string" && isRecord(attachment))
	)
		into.set(url, attachment as IAttachment);
}

function readResult(state: ReaderState, payload: unknown) {
	state.result = { value: payload === undefined ? null : payload };
}

function readIntercom(state: ReaderState, payload: unknown) {
	if (!isRecord(payload) || payload.type !== "return") return;
	readResult(state, "value" in payload ? payload.value : payload);
}

function readText(state: ReaderState, payload: unknown) {
	if (typeof payload === "string") state.directText += payload;
	else if (isRecord(payload) && typeof payload.text === "string")
		state.directText += payload.text;
}

/** The first error text is the cause; an error event without text still counts (""). */
function readError(state: ReaderState, payload: unknown) {
	if (state.terminal.errorMessage) return;
	const message = isRecord(payload) ? payload.message : payload;
	state.terminal.errorMessage =
		typeof message === "string" ? message.trim() : "";
}

const completedStatusOf = (status: unknown): CompletedStatus => {
	if (typeof status !== "string") return "failed";
	return COMPLETED_STATUS.get(status.trim().toLowerCase()) ?? "failed";
};

function readCompleted(state: ReaderState, payload: unknown) {
	const record = isRecord(payload) ? payload : {};
	const step = record.current_step;
	state.terminal.completedStatus = completedStatusOf(record.status);
	state.terminal.rejectedStage =
		typeof step === "string" && step.startsWith(REJECTED_PREFIX)
			? step.slice(REJECTED_PREFIX.length)
			: null;
	state.terminal.logLevel = finiteOrNull(record.log_level);
	state.terminal.durationMs = finiteOrNull(record.duration_ms);
}

const interactionOf = (payload: unknown): IInteractionRequest | null => {
	if (!isRecord(payload) || typeof payload.id !== "string" || !payload.id)
		return null;
	return {
		...payload,
		status: typeof payload.status === "string" ? payload.status : "pending",
	} as unknown as IInteractionRequest;
};

/** By id: a later event updates the request, but an answered one never goes back to pending. */
function readInteraction(state: ReaderState, payload: unknown) {
	const incoming = interactionOf(payload);
	if (!incoming) return;
	const existing = state.interactions.get(incoming.id);
	if (
		existing &&
		existing.status !== "pending" &&
		incoming.status === "pending"
	)
		return;
	state.interactions.set(
		incoming.id,
		existing ? { ...existing, ...incoming } : incoming,
	);
}

function stringRecord(value: unknown) {
	if (!isRecord(value)) return null;
	const entries = Object.entries(value).filter(
		(entry): entry is [string, string] => typeof entry[1] === "string",
	);
	return entries.length ? Object.fromEntries(entries) : null;
}

function readA2ui(state: ReaderState, payload: unknown) {
	if (!isRecord(payload) || payload.type !== "navigateTo") return;
	if (typeof payload.route !== "string") return;
	const queryParams = stringRecord(payload.queryParams ?? payload.query_params);
	state.navigation.push({
		route: payload.route,
		replace: payload.replace === true,
		...(queryParams ? { queryParams } : {}),
	});
}

function readChatFlags(state: ReaderState, payload: unknown) {
	if (isRecord(payload) && isRecord(payload.plan)) state.sawPlan = true;
}

/** The chat processor reads attachments of partial and final events only; Push Response sends all so far. */
function readChatStream(state: ReaderState, payload: unknown) {
	readChatFlags(state, payload);
	if (!isRecord(payload) || !Array.isArray(payload.attachments)) return;
	for (const attachment of payload.attachments)
		addAttachment(state.streamAttachments, attachment);
}

const READERS: ReadonlyMap<string, EventReader> = new Map<string, EventReader>([
	[
		"run_initiated",
		(state) => {
			state.terminal.runInitiated = true;
		},
	],
	["generic_result", readResult],
	["return", readResult],
	["output", readResult],
	["intercom", readIntercom],
	["text_output", readText],
	["stream_text", readText],
	["error", readError],
	["completed", readCompleted],
	["interaction_request", readInteraction],
	["a2ui", readA2ui],
	["chat_stream_partial", readChatFlags],
	["chat_stream", readChatStream],
	["chat_out", readChatFlags],
]);

const choicesOf = (value: unknown): readonly unknown[] | null =>
	isRecord(value) && Array.isArray(value.choices) ? value.choices : null;

/**
 * The chat processor trusts the wire: a chunk needs a list of choices, a full response a message
 * in every choice. A payload that breaks this is left out instead of stopping the whole batch.
 */
function isReadableChatPayload(payload: unknown) {
	if (!isRecord(payload)) return false;
	const { chunk, response } = payload;
	const chunkChoices = chunk == null ? [] : choicesOf(chunk);
	const responseChoices = response == null ? [] : choicesOf(response);
	return (
		chunkChoices !== null &&
		responseChoices !== null &&
		chunkChoices.every(isRecord) &&
		responseChoices.every(
			(choice) => isRecord(choice) && isRecord(choice.message),
		)
	);
}

function readChat(
	chat: ChatState,
	events: readonly IIntercomEvent[],
	context: { readonly appId: string; readonly eventId: string },
) {
	try {
		const result = processChatEvents([...events], {
			intermediateResponse: chat.response,
			responseMessage: chat.message,
			attachments: chat.attachments,
			tmpLocalState: null,
			tmpGlobalState: null,
			done: chat.done,
			appId: context.appId,
			eventId: context.eventId,
			sessionId: CHAT_SESSION_ID,
		});
		chat.response = result.intermediateResponse;
		chat.done = result.done;
	} catch (error) {
		console.warn(
			"[form-workbench] chat events of a run could not be read",
			error,
		);
	}
}

/** Media content parts of the answer become files, as in the old form: [url, type when the part names none]. */
const MEDIA_PARTS: readonly ((
	part: IContent,
) => readonly [unknown, string?])[] = [
	(part) => [part.image_url?.url, part.image_url?.media_type ?? "image/*"],
	(part) => [part.audio_url, part.media_type ?? "audio/*"],
	(part) => [part.video_url, part.media_type ?? "video/*"],
	(part) => [part.document_url, part.media_type],
];

const mediaOfPart = (part: IContent): IAttachment | null => {
	for (const read of MEDIA_PARTS) {
		const [url, type] = read(part);
		if (typeof url !== "string" || !url) continue;
		return type ? { url, type } : { url };
	}
	return null;
};

function mediaOf(content: IHistoryMessage["content"]) {
	const media: IAttachment[] = [];
	if (!Array.isArray(content)) return media;
	for (const part of content) {
		const attachment = isRecord(part) ? mediaOfPart(part as IContent) : null;
		if (attachment) media.push(attachment);
	}
	return media;
}

function attachmentsOf(chat: ChatState, state: ReaderState) {
	const byUrl = new Map<string, IAttachment>();
	for (const attachment of chat.attachments.values())
		addAttachment(byUrl, attachment);
	for (const attachment of state.streamAttachments.values())
		addAttachment(byUrl, attachment);
	for (const attachment of mediaOf(chat.message.inner.content))
		addAttachment(byUrl, attachment);
	return Array.from(byUrl.values());
}

/** The model's reasoning; without a plan the chat processor parks it in a synthetic "Thinking" step. */
function reasoningOf(chat: ChatState, sawPlan: boolean) {
	const response = Array.isArray(chat.response.choices)
		? chat.response.lastMessageOfRole(IRole.Assistant)?.reasoning
		: null;
	if (typeof response === "string" && response.trim()) return response;
	const synthetic = sawPlan ? null : chat.message.plan_steps?.[0]?.reasoning;
	return synthetic?.trim() ? synthetic : null;
}

const outputOf = (chat: ChatState, state: ReaderState): RunOutput => ({
	eventCount: state.eventCount,
	steps: state.sawPlan
		? toRunSteps(chat.message.plan_steps ?? [], chat.message.current_step_id)
		: [],
	answer: joinContentText(chat.message.inner.content) + state.directText,
	reasoning: reasoningOf(chat, state.sawPlan),
	attachments: attachmentsOf(chat, state),
	result: state.result,
	interactions: Array.from(state.interactions.values()),
	terminal: { ...state.terminal },
});

class RunEventReader implements RunAccumulator {
	private readonly state: ReaderState = {
		eventCount: 0,
		sawPlan: false,
		directText: "",
		result: null,
		terminal: { ...NO_TERMINAL_SIGNALS },
		streamAttachments: new Map(),
		interactions: new Map(),
		navigation: [],
	};
	private readonly chat: ChatState;
	private snapshot: RunOutput | null = null;

	constructor(
		private readonly context: {
			readonly appId: string;
			readonly eventId: string;
		},
	) {
		this.chat = {
			response: ChatResponse.default(),
			message: {
				id: `form-run-${context.eventId}`,
				appId: context.appId,
				sessionId: CHAT_SESSION_ID,
				inner: { role: IRole.Assistant, content: "" },
				files: [],
				timestamp: 0,
			},
			attachments: new Map(),
			done: false,
		};
	}

	push(events: readonly IIntercomEvent[]): void {
		const before = this.state.eventCount;
		const chatEvents: IIntercomEvent[] = [];
		for (const event of events) {
			if (!event || typeof event.event_type !== "string") continue;
			this.state.eventCount += 1;
			READERS.get(event.event_type)?.(this.state, event.payload);
			if (
				CHAT_EVENTS.has(event.event_type) &&
				isReadableChatPayload(event.payload)
			)
				chatEvents.push(event);
		}
		if (chatEvents.length > 0) readChat(this.chat, chatEvents, this.context);
		if (this.state.eventCount !== before) this.snapshot = null;
	}

	output(): RunOutput {
		this.snapshot ??= outputOf(this.chat, this.state);
		return this.snapshot;
	}

	takeNavigation(): readonly NavigateIntent[] {
		return this.state.navigation.splice(0);
	}
}

export const createRunAccumulator: CreateRunAccumulator = (context) =>
	new RunEventReader(context);
