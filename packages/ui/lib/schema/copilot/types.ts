import type { SurfaceComponent } from "../../../components/a2ui/types";
import type { BoardCommand } from "../flow/copilot";

/** The scope of what the copilot agent can modify */
export type CopilotScope = "Board" | "Frontend" | "Both";

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

	/** Suggested follow-up prompts */
	suggestions: UnifiedSuggestion[];

	/** The actual scope that was used (agent may decide to focus on one area) */
	active_scope: CopilotScope;
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
