import Dexie, { type EntityTable } from "dexie";
import type { IHistoryMessage } from "../../../lib";

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
}

export type PlanStepStatus = "planned" | "progress" | "done" | "failed";

export interface IPlanStep {
	id: string;
	title: string;
	description?: string;
	status: PlanStepStatus;
	reasoning?: string;
	timestamp?: number;
	startTime?: number;
	endTime?: number;
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
