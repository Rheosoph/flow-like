import Dexie, { type EntityTable } from "dexie";
import type { IHistoryMessage } from "../../../lib";
import type { IAgentDebugReport } from "../../../state/global-chat/agent-debug-report";

export type IAttachment =
	| string // Simple URL variant
	| {
			// Complex variant
			url: string;
			preview_text?: string;
			thumbnail_url?: string;
			name?: string;
			size?: number;
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
	/** a2ui widgets embedded in this message (from the Push Widget node). */
	widgets?: IChatWidget[];
}

export interface ISession {
	id: string;
	appId: string;
	summarization: string;
	createdAt: number;
	updatedAt: number;
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
	messages: EntityTable<IMessage, "id">;
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

export { chatDb };
