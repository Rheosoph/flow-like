import Dexie, { type EntityTable } from "dexie";
import { finalizePlanSteps } from "./event-processor";
import type { IHistoryMessage } from "../../../lib";
import type { IChatMessageError } from "../../../lib/flowpilot/chat-error";
import type { IAgentDebugReport } from "../../../state/global-chat/agent-debug-report";

export type IAttachment =
	| string // Simple URL variant
	| {
			// Complex variant
			url: string;
			preview_text?: string;
			thumbnail_url?: string;
			name?: string;
			/** Nullable on the wire: `ComplexAttachment.size` is an `Option<u64>` that serialises as `null`. */
			size?: number | null;
			type?: string;
			anchor?: string;
			page?: number;
	  };

/**
 * An a2ui widget instance embedded in a chat message. `component` is the
 * self-contained `widgetInstance` component (with `inlineWidgetDef` and
 * `actionBindings`) produced by the backend Push Widget node, rendered inline
 * via A2UIRenderer.
 */
export interface IChatWidget {
	instance_id: string;
	widget_id: string;
	surface_id: string;
	component: Record<string, unknown>;
	/**
	 * Ordered a2ui update messages targeting this widget (raw wire payloads).
	 * Replayed over `component` at render time so element nodes work both
	 * before the push (attached by the backend) and after it (streamed live).
	 */
	updates?: unknown[];
	/**
	 * Originating run context, set when the widget is embedded outside its own
	 * app chat (e.g. the global chat calling an app's chat event). Widget
	 * actions execute against this board instead of the hosting chat's context.
	 */
	origin?: {
		appId: string;
		boardId?: string;
		eventId?: string;
	};
}

export type PlanStepStatus = "planned" | "progress" | "done" | "failed";

/** One planned slice of a workflow build, and whether it has reached the board yet. */
export interface IBuildLaneSegment {
	id: string;
	title: string;
	applied?: boolean;
}

/** A function the specialist could not build, committed with its interface but no logic. */
export interface IBuildLaneGap {
	function?: string;
	detail: string;
}

/**
 * A concurrent branch of one build. The data, page and workflow specialists own disjoint state and
 * run at the same time, so a flat list of rows misrepresents what is happening — this carries the
 * shape a lane needs to render as a real progress block instead of one truncated line.
 */
export interface IBuildLaneDetail {
	kind: "build_lane";
	lane: "data" | "page" | "workflow";
	/** What this lane is building: a route, a board name, the tables. */
	target?: string;
	segments?: IBuildLaneSegment[];
	segmentsApplied?: number;
	segmentsTotal?: number;
	/** Wall clock this lane earned by proving progress, in minutes. */
	earnedMinutes?: number;
	gaps?: IBuildLaneGap[];
}

export type IPlanStepDetail = IBuildLaneDetail;

export interface IPlanStep {
	id: string;
	title: string;
	description?: string;
	status: PlanStepStatus;
	reasoning?: string;
	timestamp?: number;
	startTime?: number;
	endTime?: number;
	/**
	 * Structured payload for steps that deserve more than a title/description row. Optional and
	 * additive: producers that do not set it keep the plain row, and it survives persistence and the
	 * sub-step fold because both pass step objects through untouched.
	 */
	detail?: IPlanStepDetail;
	/**
	 * Offset into the message text at the moment this step started. Lets the renderer place the
	 * step inline between the text segments it interrupted; steps without an anchor render in the
	 * legacy grouped block above the text.
	 */
	content_offset?: number;
	/** Tool that produced this step. Drives the FlowPilot orb's activity state and tool labels. */
	toolName?: string;
}

export interface IModelCallEntry {
	model: string;
	usage: {
		completion_tokens: number;
		prompt_tokens: number;
		total_tokens: number;
		cost?: number | null;
	};
	duration_ms?: number | null;
}

export interface ILLMUsageStats {
	usage: {
		completion_tokens: number;
		prompt_tokens: number;
		total_tokens: number;
		cost?: number | null;
	};
	model?: string | null;
	duration_ms?: number | null;
	iterations?: number | null;
	calls?: IModelCallEntry[];
}

export interface IChatUsageStat {
	step_name: string;
	stats: ILLMUsageStats;
}

/**
 * How one assistant turn was actually executed, stamped on the message while the run is alive.
 *
 * Nothing here is recoverable afterwards: the run record (which holds the pinned provider/model)
 * is deleted the moment the turn ends, `getGlobalChatTurnSelection` then silently falls back to
 * whatever the model picker currently shows, and `debug_report` is never even built outside dev.
 * A rating arrives minutes later, so without this field the answer to "which model produced this?"
 * is a guess. Kept flat and ids-only on purpose — it is rewritten on every ~1s checkpoint, and
 * desktop Dexie writes pay a steep serialization cost for nested values.
 */
export interface IChatRunContext {
	/** Payload version, so a stored rating stays readable when the shape grows. */
	schema: string;
	provider?: string;
	/** Raw picker model id. */
	model_id?: string;
	/** The prefixed id actually handed to the backend. */
	effective_model_id?: string;
	reasoning_effort?: string;
	auto_mode?: boolean;
	memory_enabled?: boolean;
	/** Which FlowPilot surface started the turn. */
	surface?: string;
	/** Conversation scope: platform-wide, a board copilot, or the UI builder. */
	mode?: string;
	board_app_id?: string;
	board_id?: string;
	/** The user turn this reply answers. Steering commits later user messages, so a
	 * "previous message" scan can attribute the wrong prompt. */
	user_message_id?: string;
	attachment_count?: number;
	started_at_ms?: number;
	ended_at_ms?: number;
	duration_ms?: number;
	/** "ok" | "partial" | "error" | "timeout" — mirrors the debug report's outcome. */
	outcome?: string;
	terminal_code?: string;
	error?: string;
	steer_count?: number;
	resumed?: boolean;
}

export interface IMessage {
	id: string;
	appId: string;
	sessionId: string;
	inner: IHistoryMessage;
	files: IAttachment[];
	actions?: string[];
	tools?: string[];
	explicit_name?: string;
	rating?: number;
	ratingSettings?: {
		includeChatHistory?: boolean;
		comment?: string;
		canContact?: boolean;
	};
	timestamp: number;
	plan_steps?: IPlanStep[];
	current_step_id?: string;
	usage_stats?: IChatUsageStat[];
	/** Apps this message acted on/referenced — rendered as clickable chips under the message. */
	app_refs?: string[];
	/** Persisted, bounded lifecycle report for debugging one complete agent turn. */
	debug_report?: IAgentDebugReport;
	/** How this assistant turn was executed. Global chat only; see {@link IChatRunContext}. */
	run_context?: IChatRunContext;
	/** a2ui widgets embedded in this message (from the Push Widget node). */
	widgets?: IChatWidget[];
	/**
	 * Why this turn failed, classified. Set instead of writing an error sentence into the content so
	 * the failure renders as a card with its own recovery action — and so history never carries the
	 * apology back to the model.
	 */
	error?: IChatMessageError;
	/**
	 * Write revision, bumped by {@link putChatMessage}. Dexie liveQuery
	 * re-materializes every row on any table write; an unchanged rev lets the
	 * message list hand back the previously materialized object so settled
	 * messages keep a stable identity and memoized rows don't re-render per
	 * streamed save.
	 */
	rev?: number;
}

/**
 * Persisted shape of a chat message. The message travels as one JSON string: on the desktop every
 * stored value crosses the IndexedDB shim's structural encoder, which walks nested objects at
 * roughly 50-100x the cost of `JSON.stringify`, so a flat row with a string payload is the cheap
 * representation. The indexed columns stay real so range scans, `sortBy("timestamp")` and
 * age-based cleanup never touch the payload.
 */
export interface IMessageRow {
	id: string;
	sessionId: string;
	timestamp: number;
	rev?: number;
	payload: string;
}

export function encodeMessageRow(message: IMessage): IMessageRow {
	return {
		id: message.id,
		sessionId: message.sessionId,
		timestamp: message.timestamp,
		rev: message.rev,
		payload: JSON.stringify(message),
	};
}

/** Rows written before the payload column are the nested message itself; they re-encode on their next write. */
export function decodeMessageRow(row: IMessageRow | IMessage): IMessage {
	if (typeof (row as IMessageRow).payload !== "string") return row as IMessage;
	return JSON.parse((row as IMessageRow).payload) as IMessage;
}

function bumpRev(message: IMessage): IMessageRow {
	message.rev = (message.rev ?? 0) + 1;
	return encodeMessageRow(message);
}

/** Writes a settled message. Every writer here bumps {@link IMessage.rev} so identity caching stays truthful. */
export async function putChatMessage(message: IMessage): Promise<void> {
	await chatDb.messages.put(bumpRev(message));
}

/**
 * Checkpoint of a message still streaming. Drafts have their own table so the session's message
 * query is not re-run by every streaming save; a draft only matters if its stream never completes.
 */
export async function putChatDraft(message: IMessage): Promise<void> {
	await chatDb.drafts.put(bumpRev(message));
}

/**
 * Settles a streamed message: it lands in `messages` and every draft of the session goes in the
 * same transaction. A session runs at most one stream, so any draft left at this point — including
 * one written under an id a resumed subscriber never learned — is stale.
 */
export async function finalizeChatMessage(message: IMessage): Promise<void> {
	const row = bumpRev(message);
	await chatDb.transaction("rw", chatDb.messages, chatDb.drafts, async () => {
		await chatDb.messages.put(row);
		await chatDb.drafts.where("sessionId").equals(row.sessionId).delete();
	});
}

/** The session's in-flight message, if a streaming checkpoint has been written for it. */
export async function latestChatDraft(
	sessionId: string,
): Promise<IMessage | undefined> {
	const rows = await chatDb.drafts
		.where("sessionId")
		.equals(sessionId)
		.toArray();
	if (rows.length === 0) return undefined;
	const newest = rows.reduce((a, b) => (b.timestamp > a.timestamp ? b : a));
	return decodeMessageRow(newest);
}

/**
 * Promotes drafts left behind by a crash or reload mid-stream into settled messages. Only called
 * when the execution engine holds no stream for the session, so nothing is still writing them; a
 * draft whose message was already settled is dropped rather than rolled back over it.
 */
export async function promoteChatDrafts(sessionId: string): Promise<number> {
	const drafts = await chatDb.drafts
		.where("sessionId")
		.equals(sessionId)
		.toArray();
	if (drafts.length === 0) return 0;
	await chatDb.transaction("rw", chatDb.messages, chatDb.drafts, async () => {
		for (const row of drafts) {
			const settled = await chatDb.messages.where("id").equals(row.id).count();
			if (settled === 0) {
				const message = decodeMessageRow(row);
				finalizePlanSteps(message);
				await chatDb.messages.put(bumpRev(message));
			}
			await chatDb.drafts.delete(row.id);
		}
	});
	return drafts.length;
}

export interface ISession {
	id: string;
	appId: string;
	summarization: string;
	createdAt: number;
	updatedAt: number;
	/**
	 * Epoch millis the user pinned this conversation; absent means unpinned. A timestamp rather
	 * than a boolean because booleans are not valid IndexedDB keys.
	 */
	pinnedAt?: number;
}

export interface ILocalChatState {
	id: string;
	appId: string;
	eventId: string;
	sessionId: string;
	localState: Record<string, any>;
}

export interface IGlobalState {
	id: string;
	appId: string;
	eventId: string;
	globalState: Record<string, any>;
}

const chatDb = new Dexie("Chat-History") as Dexie & {
	sessions: EntityTable<ISession, "id">;
	messages: EntityTable<IMessageRow, "id">;
	drafts: EntityTable<IMessageRow, "id">;
	localStage: EntityTable<ILocalChatState, "id">;
	globalState: EntityTable<IGlobalState, "id">;
};

// Schema declaration:
chatDb.version(3).stores({
	sessions: "id, appId, updatedAt, [updatedAt+appId]",
	messages: "id, sessionId",
	localStage: "sessionId, appId, eventId, [sessionId+eventId], timestamp",
	globalState: "appId, eventId, [appId+eventId]",
});

chatDb.version(4).stores({
	sessions: "id, appId, updatedAt, [updatedAt+appId], pinnedAt",
	messages: "id, sessionId",
	localStage: "sessionId, appId, eventId, [sessionId+eventId], timestamp",
	globalState: "appId, eventId, [appId+eventId]",
});

chatDb.version(5).stores({
	sessions: "id, appId, updatedAt, [updatedAt+appId], pinnedAt",
	messages: "id, sessionId",
	drafts: "id, sessionId",
	localStage: "sessionId, appId, eventId, [sessionId+eventId], timestamp",
	globalState: "appId, eventId, [appId+eventId]",
});

export { chatDb };
