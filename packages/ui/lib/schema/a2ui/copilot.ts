import type {
	CanvasSettings,
	SurfaceComponent,
} from "../../../components/a2ui/types";

/** Role in the A2UI chat conversation */
export type A2UIChatRole = "User" | "Assistant";

/** An image attachment in a chat message */
export interface A2UIChatImage {
	/** Base64-encoded image data (without data URL prefix) */
	data: string;
	/** MIME type (e.g., "image/png", "image/jpeg") */
	media_type: string;
}

/** A message in the A2UI chat history */
export interface A2UIChatMessage {
	role: A2UIChatRole;
	content: string;
	/** Optional images attached to this message (for vision models) */
	images?: A2UIChatImage[];
}

/** Status of a plan step */
export type A2UIPlanStepStatus =
	| "NotStarted"
	| "InProgress"
	| "Completed"
	| "Failed";

/** A step in the AI's plan */
export interface A2UIPlanStep {
	id: string;
	description: string;
	status: A2UIPlanStepStatus;
	tool_name?: string;
}

/** Response from the A2UI copilot */
export interface A2UICopilotResponse {
	/** Text response from the assistant */
	message: string;
	/** Generated or modified UI components */
	components: SurfaceComponent[];
	/** Optional canvas settings emitted with the generated surface */
	canvas_settings?: CanvasSettings;
	/** Optional root component id emitted with the generated surface */
	root_component_id?: string;
	/** Optional follow-up suggestions */
	suggestions: string[];
}

/** Stream event types for real-time updates */
export type A2UIStreamEvent =
	| { PlanStep: A2UIPlanStep }
	| { ComponentPreview: SurfaceComponent[] };
