import { type IContent, IRole } from "../../../lib/schema/llm/history";
import type { IMessage, IPlanStep } from "./chat-db";
import { joinContentText } from "./inline-segments";

/** Where a sent question stands while its answer has shown nothing yet. */
export type PendingAskPhase = "sending" | "waiting";

export interface PendingAsk {
	readonly phase: PendingAskPhase;
	/** Epoch ms the question was sent; the status counts from here. */
	readonly since: number;
}

/** The composer's latest send, matched to its question by text. */
export interface ChatSend {
	readonly content: string;
	readonly startedAt: number;
	readonly inFlight: boolean;
}

export type PendingAskStage = "sending" | "waiting" | "stillWaiting" | "slow";

export interface PendingAsks {
	/** Question id → its wait. */
	readonly byAsk: ReadonlyMap<string, PendingAsk>;
	/** Live answers with nothing to show; their question carries the wait instead. */
	readonly hidden: ReadonlySet<string>;
}

const SHOW_ELAPSED_MS = 2_000;
const STILL_WAITING_MS = 8_000;
const SLOW_MS = 20_000;

const ASK_RULE = "var(--fl-chat-ask-rule, var(--primary))";

export function pendingAskStage(
	phase: PendingAskPhase,
	elapsedMs: number,
): PendingAskStage {
	if (phase === "sending") return "sending";
	if (elapsedMs >= SLOW_MS) return "slow";
	if (elapsedMs >= STILL_WAITING_MS) return "stillWaiting";
	return "waiting";
}

/** `null` until the wait is long enough to be worth a number. */
export function formatPendingElapsed(elapsedMs: number): string | null {
	if (elapsedMs < SHOW_ELAPSED_MS) return null;
	const seconds = Math.floor(elapsedMs / 1000);
	if (seconds < 60) return `${seconds}s`;
	return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, "0")}s`;
}

/** The question's rule, dimmed while the send has not been handed off yet. */
export function askRuleColor(pending: PendingAsk | undefined) {
	return pending?.phase === "sending"
		? `color-mix(in oklch, ${ASK_RULE} 45%, transparent)`
		: ASK_RULE;
}

/** The placeholder step some producers open before a real one exists; it renders nothing. */
export function isEmptyFallbackStep(step: IPlanStep) {
	return (
		step.id === "step-0" &&
		step.title === "Thinking" &&
		!step.description?.trim() &&
		!step.reasoning?.trim()
	);
}

const VISIBLE_LISTS = ["widgets", "files", "app_refs", "usage_stats"] as const;
const MEDIA_FIELDS = [
	"image_url",
	"audio_url",
	"video_url",
	"document_url",
] as const;

function isRealStep(step: IPlanStep) {
	return !isEmptyFallbackStep(step);
}

function hasVisibleExtras(message: IMessage) {
	const hasList = (key: (typeof VISIBLE_LISTS)[number]) =>
		(message[key]?.length ?? 0) > 0;
	return (
		Boolean(message.error) ||
		VISIBLE_LISTS.some(hasList) ||
		(message.plan_steps ?? []).some(isRealStep)
	);
}

function isVisiblePart(part: IContent) {
	const hasMedia = (field: (typeof MEDIA_FIELDS)[number]) =>
		Boolean(part[field]);
	return Boolean(part.text?.trim()) || MEDIA_FIELDS.some(hasMedia);
}

function hasVisibleContent(content: IMessage["inner"]["content"]) {
	if (typeof content === "string") return content.trim() !== "";
	return Array.isArray(content) && content.some(isVisiblePart);
}

/**
 * True while a streaming answer has nothing a reader could see: no text or reasoning, no step,
 * widget, attachment, error, usage or host content.
 */
export function isAwaitingFirstEvent(
	message: IMessage,
	hasHostContent: boolean,
) {
	return (
		message.inner.role === IRole.Assistant &&
		!hasHostContent &&
		!hasVisibleExtras(message) &&
		!hasVisibleContent(message.inner.content)
	);
}

function isSendOf(ask: IMessage, send: ChatSend | null): send is ChatSend {
	return (
		send !== null &&
		ask.timestamp >= send.startedAt &&
		joinContentText(ask.inner.content) === send.content
	);
}

/**
 * Which questions still wait for their answer's first visible event. A live answer belongs to
 * the latest question sent at or before it; while it is empty it is hidden and that question
 * shows the wait. A send still in flight without any answer yet is "sending".
 */
export function resolvePendingAsks(
	transcript: readonly IMessage[],
	live: readonly IMessage[],
	send: ChatSend | null,
	hasHostContent: (messageId: string) => boolean,
): PendingAsks {
	const asks = transcript.filter(
		(message) => message.inner.role === IRole.User,
	);
	const byAsk = new Map<string, PendingAsk>();
	const hidden = new Set<string>();
	const answered = new Set<string>();

	for (const answer of live) {
		if (answer.inner.role !== IRole.Assistant) continue;
		const ask = asks.findLast((entry) => entry.timestamp <= answer.timestamp);
		if (!ask) continue;
		answered.add(ask.id);
		if (!isAwaitingFirstEvent(answer, hasHostContent(answer.id))) continue;
		hidden.add(answer.id);
		byAsk.set(ask.id, {
			phase: "waiting",
			since: isSendOf(ask, send) ? send.startedAt : ask.timestamp,
		});
	}

	const latest = asks.at(-1);
	if (
		latest &&
		send?.inFlight &&
		!answered.has(latest.id) &&
		isSendOf(latest, send)
	) {
		byAsk.set(latest.id, { phase: "sending", since: send.startedAt });
	}

	return { byAsk, hidden };
}
