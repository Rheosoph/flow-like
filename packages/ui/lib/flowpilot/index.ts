/**
 * FlowPilot A2UI Generation Utilities
 * Export all FlowPilot-related lib utilities
 */

export {
	A2UI_SYSTEM_PROMPT,
	COMPONENT_SELECTION_GUIDANCE,
	STYLE_SUGGESTION_PROMPT,
	FEW_SHOT_EXAMPLES,
	A2UI_EDIT_PROMPT,
	buildA2UIPrompt,
	parseA2UIResponse,
} from "./a2ui-prompts";

export {
	useA2UIGeneration,
	streamA2UIGeneration,
} from "./use-a2ui-generation";
export type {
	A2UIGenerationState,
	UseA2UIGenerationOptions,
	StreamingGenerationOptions,
} from "./use-a2ui-generation";

export { buildChatMessageError } from "./chat-error";
export type {
	ChatErrorKind,
	IChatErrorAction,
	IChatMessageError,
} from "./chat-error";
