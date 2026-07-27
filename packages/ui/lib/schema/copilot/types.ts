import type { SurfaceComponent } from "../../../components/a2ui/types";
import type { IGenericCommand } from "../flow/board/commands/generic-command";
import type { BoardCommand } from "../flow/copilot";

/** The scope of what the copilot agent can modify */
export type CopilotScope =
	| "Board"
	| "Frontend"
	| "Both"
	| "DataStudio"
	/** Read-only prior-art research; returns a foundation plan, mutates nothing. */
	| "Scout"
	/** Read-only public-web research; holds the only search/page-read tools. */
	| "Research";

/** Role in the chat conversation */
export type ChatRole = "User" | "Assistant";

/** An image attachment in a chat message */
export interface ChatImage {
	/** Base64-encoded image data (without data URL prefix) */
	data: string;
	/** MIME type (e.g., "image/png", "image/jpeg") */
	media_type: string;
}

/** A unified chat message that can contain both text and images */
export interface UnifiedChatMessage {
	role: ChatRole;
	content: string;
	/** Optional images attached to this message (for vision models) */
	images?: ChatImage[];
}

/** Context for a specific run (for log queries) */
export interface RunContext {
	run_id: string;
	app_id: string;
	board_id: string;
}

/** Basic page information for navigation actions */
export interface PageInfo {
	id: string;
	name: string;
}

/** Basic workflow event information for triggering workflows */
export interface WorkflowEventInfo {
	node_id: string;
	name: string;
}

/** Context for UI actions (pages, events, etc.) */
export interface UIActionContext {
	app_id: string;
	board_id?: string;
	pages: PageInfo[];
	workflow_events: WorkflowEventInfo[];
}

/** Frontend-owned scope injected into runtime tools used by a nested board/UI specialist. */
export interface CopilotToolContext {
	/**
	 * Optional to match the Rust `FrontendToolContext.app_id`. A delegated run can
	 * legitimately have no app in scope — the Scout starts by searching across
	 * every app the user can see, before any single one is chosen.
	 */
	appId?: string;
	boardId?: string;
	/**
	 * The overlay/ontology the current Data Studio page has selected. Injected as a DEFAULT into
	 * data-studio tool calls (the model can override it to reach another overlay/app).
	 */
	overlayId?: string;
	/** Correlates tools called inside a delegated run with its outer frontend request. */
	parentRequestId?: string;
	/**
	 * Top-level chat run that owns the delegated specialist. Travels down so the specialist's own
	 * frontend tool calls come back tagged with the reply they belong to — with several turns
	 * streaming at once the bridge cannot infer it.
	 */
	runId?: string;
	/**
	 * Stable id of the chat conversation that owns the delegated run. Scopes retained-draft and
	 * acceptance-contract identity so identical prompt text from another conversation can never
	 * resume this conversation's drafts.
	 */
	conversationId?: string;
	/** Immutable top-level user request that owns a delegated specialist run. */
	sourceUserPrompt?: string;
	/**
	 * Bounded frontend-owned database/UI/storage context gathered once before a board run. Every
	 * backend receives the same payload; board agents must not repeat this inventory pre-draft.
	 */
	boardContextManifest?: unknown;
}

/** Unified context passed to the copilot */
export interface UnifiedContext {
	scope: CopilotScope;
	run_context?: RunContext;
	action_context?: UIActionContext;
}

/** A suggestion for follow-up actions (works for both board and UI) */
export interface UnifiedSuggestion {
	label: string;
	prompt: string;
	/** Which scope this suggestion targets */
	scope?: CopilotScope;
}

/** Canvas settings for UI components */
export interface CanvasSettings {
	backgroundColor?: string;
	backgroundImage?: string;
	padding?: string;
	customCss?: string;
}

/** Unified response from the copilot agent */
export interface UnifiedCopilotResponse {
	/** The assistant's message explaining what was done or what should be done */
	message: string;

	/** Board commands to execute (for Board and Both scopes) */
	commands: BoardCommand[];

	/** UI components generated (for Frontend and Both scopes) */
	components: SurfaceComponent[];

	/** Canvas settings for UI components (includes customCss) */
	canvas_settings?: CanvasSettings;

	/** Root component ID for UI components */
	root_component_id?: string;

	/** Last FlowScript document submitted by the workflow agent */
	flowscript_workspace?: string;

	/** Exact retained compiled workflow batch awaiting Apply/Dismiss resolution. */
	flow_ir_commit?: FlowIrCommitToken;

	/** Suggested follow-up prompts */
	suggestions: UnifiedSuggestion[];

	/** The actual scope that was used (agent may decide to focus on one area) */
	active_scope: CopilotScope;
}

export interface FlowIrCommitToken {
	board_id: string;
	draft_id: string;
	revision: number;
	base_fingerprint: string;
	claim_id: string;
	/** Host-derived UI hint; native Apply re-derives and enforces this policy. */
	requires_destructive_approval?: boolean;
}

export type FlowIrCommitDisposition = "preflight" | "applied" | "dismissed";

export interface FlowIrCommitDispositionResult {
	status: "current" | "applied" | "dismissed" | "error";
	code?: string;
	message: string;
}

export type BoardEditJobPhase =
	| "preparing"
	| "awaiting_approval"
	| "applying"
	| "applied_pending_delivery"
	| "applied"
	| "denied"
	| "stale"
	| "failed"
	| "cancelled";

export interface BoardEditJobReview {
	commandCount: number;
	commandCounts: Record<string, number>;
	commandSummaries: string[];
	replacementMode: boolean;
	destructiveEffects: string[];
}

export interface BoardEditJobApproval {
	kind: "none" | "mutating" | "execute";
	title: string;
	description: string;
	sessionKey: string;
	timing?: "before_execution" | "before_apply";
}

/** Native, provider-neutral lifecycle for one exact retained compiler batch. */
export interface BoardEditJob {
	schemaVersion: "flowpilot.board-edit-job/v1";
	jobId: string;
	appId: string;
	boardId: string;
	requestId?: string;
	remoteProfileId?: string;
	remotePrincipalId?: string;
	remoteHub?: string;
	phase: BoardEditJobPhase;
	createdAtMs: number;
	updatedAtMs: number;
	expiresAtMs: number;
	token: FlowIrCommitToken;
	approval: BoardEditJobApproval;
	review: BoardEditJobReview;
	result?: {
		status: "applied" | "stale" | "error";
		code?: string;
		message: string;
		/** Native mutation already happened during an earlier apply attempt. */
		replayed?: boolean;
		commands: IGenericCommand[];
		board_commands: BoardCommand[];
		diagnostics: string[];
		final_board_node_count?: number;
	};
	error?: string;
}

export interface BoardEditJobResolution {
	job: BoardEditJob;
	transitioned: boolean;
}

export interface BoardEditJobDeliveryClaim {
	job: BoardEditJob;
	claimed: boolean;
	deliveryLeaseId?: string;
}

/** Status of a plan step */
export type PlanStepStatus = "Pending" | "InProgress" | "Completed" | "Failed";

/** A step in the AI's plan */
export interface PlanStep {
	id: string;
	description: string;
	status: PlanStepStatus;
	tool_name?: string;
}

/** Stream events for real-time updates */
export type UnifiedStreamEvent =
	| { Token: string }
	| { PlanStep: PlanStep }
	| { ToolCall: { name: string; args: string } }
	| { ToolResult: { name: string; result: string } }
	| { Thinking: string }
	| { FocusNode: { node_id: string; description: string } }
	| { ComponentPreview: SurfaceComponent[] }
	| { ScopeDecision: CopilotScope };
