import type React from "react";
import type { IBoard } from "../../lib";
import type { A2UIPlanStep } from "../../lib/schema/a2ui/copilot";
import type {
	CanvasSettings,
	FlowIrCommitToken,
} from "../../lib/schema/copilot";
import type {
	BoardCommand,
	PlanStep,
	Suggestion,
} from "../../lib/schema/flow/copilot";
import type { INode } from "../../lib/schema/flow/node";
import type { IApplyFlowIrCommitResponse } from "../../state/backend-state/board-state";
import type { SurfaceComponent } from "../a2ui/types";

/**
 * Agent mode determines what the copilot operates on:
 * - "board": Flow board operations (adding nodes, connections, etc.)
 * - "ui": A2UI surface operations (creating/modifying UI components)
 * - "both": Can operate on both (future capability)
 */
export type AgentMode = "board" | "ui" | "both";

/**
 * AI Provider type for FlowPilot
 * - "bits": Use configured model bits from user profile
 * - "github-copilot": Use the GitHub Copilot SDK directly in the desktop app
 * - "codex": Use the Codex CLI through the shared FlowPilot MCP tool surface
 * - "claude-code": Use Claude Code through the shared FlowPilot MCP tool surface
 * - "copilot": Legacy alias for "github-copilot"
 */
export type AIProvider =
	| "bits"
	| "github-copilot"
	| "codex"
	| "claude-code"
	| "copilot";

export type NormalizedAIProvider =
	| "bits"
	| "github-copilot"
	| "codex"
	| "claude-code";

export type AgentBackendProvider = Exclude<NormalizedAIProvider, "bits">;

export const AGENT_BACKEND_PROVIDERS: AgentBackendProvider[] = [
	"github-copilot",
	"codex",
	"claude-code",
];

export function normalizeAIProvider(
	provider?: AIProvider,
): NormalizedAIProvider {
	if (!provider) return "bits";
	if (provider === "copilot") return "github-copilot";
	return provider;
}

export function isAgentBackendProvider(
	provider: AIProvider | NormalizedAIProvider,
): provider is AgentBackendProvider {
	return normalizeAIProvider(provider as AIProvider) !== "bits";
}

export function flowPilotModelIdForProvider(
	provider: AIProvider | NormalizedAIProvider,
	modelId?: string,
): string | undefined {
	if (!modelId) return undefined;

	const normalized = normalizeAIProvider(provider as AIProvider);
	switch (normalized) {
		case "bits":
			return modelId;
		case "github-copilot":
			return `github-copilot:${modelId}`;
		case "codex":
			return `codex:${modelId}`;
		case "claude-code":
			return `claude-code:${modelId}`;
	}
}

/**
 * Copilot model information from the SDK
 */
export interface CopilotReasoningEffort {
	/** Provider-native value forwarded unchanged to the selected backend. */
	id: string;
	/** Human-readable label supplied or normalized by the backend. */
	name: string;
	/** Optional provider-supplied explanation of the trade-off. */
	description?: string;
}

export interface CopilotModel {
	/** Model ID */
	id: string;
	/** Model display name */
	name: string;
	/** Reasoning levels advertised dynamically for this exact model. */
	supportedReasoningEfforts?: CopilotReasoningEffort[];
	/** Provider-advertised default; omitting the request setting still defers to it. */
	defaultReasoningEffort?: string;
}

/**
 * Copilot authentication status
 */
export interface CopilotAuthStatus {
	/** Whether the user is authenticated with GitHub Copilot */
	authenticated: boolean;
	/** GitHub username if authenticated */
	login?: string;
	/** Backend-specific status message */
	message?: string;
}

/**
 * Copilot connection configuration
 */
export interface CopilotConnectionConfig {
	/** Use stdio connection (local mode) or TCP/remote */
	useStdio: boolean;
	/** Server URL for remote/web mode */
	serverUrl?: string;
	/** Agent backend to start */
	backend?: AgentBackendProvider;
}

/**
 * Specialized agent type for Copilot
 */
export type CopilotAgentType = "general" | "frontend" | "backend";

/**
 * Loading phases that represent the AI's current activity
 */
export type LoadingPhase =
	| "idle"
	| "initializing"
	| "analyzing"
	| "searching"
	| "reasoning"
	| "generating"
	| "finalizing";

export interface LoadingPhaseInfo {
	label: string;
	icon: React.ReactNode;
	color: string;
}

export const LOADING_PHASES: Record<LoadingPhase, LoadingPhaseInfo> = {
	idle: {
		label: "Ready",
		icon: null,
		color: "text-muted-foreground",
	},
	initializing: {
		label: "Starting up",
		icon: null,
		color: "text-blue-500",
	},
	analyzing: {
		label: "Analyzing...",
		icon: null,
		color: "text-violet-500",
	},
	searching: {
		label: "Searching...",
		icon: null,
		color: "text-cyan-500",
	},
	reasoning: {
		label: "Thinking...",
		icon: null,
		color: "text-amber-500",
	},
	generating: {
		label: "Generating...",
		icon: null,
		color: "text-pink-500",
	},
	finalizing: {
		label: "Finalizing...",
		icon: null,
		color: "text-green-500",
	},
};

/**
 * Image attachment interface used across both modes
 */
export interface AttachedImage {
	/** Base64-encoded image data (without data URL prefix) */
	data: string;
	/** MIME type (e.g., "image/png", "image/jpeg") */
	mediaType: string;
	/** Data URL for preview display */
	preview: string;
}

/**
 * Unified plan step that works for both board and UI modes
 */
export type UnifiedPlanStep = PlanStep | A2UIPlanStep;

export type FlowPilotProcessEventKind =
	| "tool"
	| "progress"
	| "workspace"
	| "commands"
	| "components";

export type FlowPilotProcessEventStatus = "running" | "done" | "error" | "info";

export interface FlowPilotProcessEvent {
	id: string;
	kind: FlowPilotProcessEventKind;
	status: FlowPilotProcessEventStatus;
	title: string;
	summary?: string;
	toolName?: string;
	details?: string;
	resultPreview?: string;
	workspaceBefore?: string;
	workspaceAfter?: string;
	commands?: BoardCommand[];
	componentCount?: number;
	createdAt: number;
	updatedAt?: number;
}

export interface FlowScriptApplyResultLike {
	commands?: unknown[];
	board_commands?: BoardCommand[];
	/** Non-blocking source repairs that require a canonical FlowScript readback. */
	corrections?: string[];
	diagnostics?: string[];
	final_board_node_count?: number;
}

export interface FlowScriptApplyOptions {
	allowDeletions?: boolean;
	suppressBlockedToast?: boolean;
}

/**
 * Unified message format for the copilot chat
 */
export interface CopilotMessage {
	role: "user" | "assistant";
	content: string;
	images?: AttachedImage[];
	/** Plan steps associated with this message */
	planSteps?: UnifiedPlanStep[];
	/** Context node IDs (board mode) */
	contextNodeIds?: string[];
	/** Applied components (UI mode) */
	appliedComponents?: SurfaceComponent[];
	/** Executed board commands (board mode) */
	executedCommands?: BoardCommand[];
	/** Last FlowScript draft/workspace produced by the workflow agent */
	flowscriptWorkspace?: string;
	/** Live process timeline for tool calls, FlowScript edits, and queued changes */
	processEvents?: FlowPilotProcessEvent[];
}

/**
 * Props for the unified FlowPilot component
 */
export interface FlowPilotProps {
	/** The agent mode determines what the copilot operates on */
	agentMode: AgentMode;

	/** Title to display in the header (defaults to "FlowPilot") */
	title?: string;

	/** Custom class name for styling */
	className?: string;

	/** Callback when close button is clicked */
	onClose?: () => void;

	/** Notifies parent shells when the FlowScript workspace pane is visible. */
	onWorkspaceVisibleChange?: (visible: boolean) => void;

	// === Provider Props ===

	/** Force a specific AI provider (if not set, shows provider selector) */
	forceProvider?: AIProvider;

	/** Default provider to use (defaults to "bits" for backward compatibility) */
	defaultProvider?: AIProvider;

	// === Board Mode Props ===

	/** The board to operate on (required for board mode) */
	board?: IBoard | null;

	/** Current app id, used by FlowPilot runtime tools for database/storage/event access. */
	appId?: string;

	/** App-scoped catalog nodes visible to the board, including installed package nodes. */
	catalogNodes?: INode[];

	/** Selected node IDs for context (board mode) */
	selectedNodeIds?: string[];

	/** Callback when a suggestion is accepted (board mode) */
	onAcceptSuggestion?: (suggestion: Suggestion) => void;

	/** Callback when commands should be executed (board mode) */
	onExecuteCommands?: (commands: BoardCommand[]) => void | Promise<void>;

	/** Callback when the FlowScript workspace should be reconciled and applied server-side. */
	onApplyFlowScript?: (
		flowscript: string,
		options?: FlowScriptApplyOptions,
	) =>
		| undefined
		| FlowScriptApplyResultLike
		| Promise<undefined | FlowScriptApplyResultLike>;

	/** Atomically apply the exact retained compiled workflow batch and record undo/refetch state. */
	onApplyFlowIrCommit?: (
		token: FlowIrCommitToken,
		deliveryId?: string,
		historyMode?: "append" | "invalidate",
	) => Promise<IApplyFlowIrCommitResponse>;

	/** Callback to focus on a specific node (board mode) */
	onFocusNode?: (nodeId: string) => void;

	/** Callback to select nodes (board mode) */
	onSelectNodes?: (nodeIds: string[]) => void;

	/** Run context for log analysis (board mode) */
	runContext?: {
		run_id: string;
		app_id: string;
		board_id: string;
		event_id?: string;
	};

	/** Initial prompt to auto-submit (board mode) */
	initialPrompt?: string;

	// === UI Mode Props ===

	/** Current UI components on the surface (UI mode) */
	currentComponents?: SurfaceComponent[];

	/** The surface's live canvasSettings, customCss included (UI mode). Supplying it is what lets
	 * the copilot edit an existing stylesheet instead of replacing one it cannot see. */
	currentCanvasSettings?: CanvasSettings;

	/** Selected component IDs (UI mode) */
	selectedComponentIds?: string[];

	/** Callback when components are generated (UI mode) */
	onComponentsGenerated?: (components: SurfaceComponent[]) => void;

	/** Callback when components should be applied (UI mode) */
	onApplyComponents?: (
		components: SurfaceComponent[],
		canvasSettings?: CanvasSettings,
	) => void;

	// === Screenshot Props ===

	/** Custom function to capture a screenshot. If provided, shows split send button.
	 * Should return base64 data URL of the screenshot, or null if capture failed. */
	captureScreenshot?: () => Promise<string | null>;
}

/**
 * Internal state interface for the FlowPilot component
 */
export interface FlowPilotState {
	messages: CopilotMessage[];
	input: string;
	loading: boolean;
	loadingPhase: LoadingPhase;
	loadingStartTime: number | null;
	elapsedSeconds: number;
	tokenCount: number;
	planSteps: UnifiedPlanStep[];
	attachedImages: AttachedImage[];
	userScrolledUp: boolean;
	selectedModelId: string;
	/** Current AI provider */
	provider: AIProvider;

	// Board-specific state
	pendingCommands: BoardCommand[];
	suggestions: Suggestion[];
	currentToolCall: string | null;

	// UI-specific state
	pendingComponents: SurfaceComponent[];
}
