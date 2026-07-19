import type React from "react";
import type { IBoard, ILogMetadata } from "../../../lib";
import type { FlowIrCommitToken } from "../../../lib/schema/copilot";
import type {
	BoardCommand,
	PlanStep,
	Suggestion,
} from "../../../lib/schema/flow/copilot";
import type { INode } from "../../../lib/schema/flow/node";
import type { IApplyFlowIrCommitResponse } from "../../../state/backend-state/board-state";

export type LoadingPhase =
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

export type Mode = "chat" | "autocomplete" | "panel" | "embedded";

export interface CopilotMessage {
	role: "user" | "assistant";
	content: string;
	agentType?: "Explain" | "Edit";
	executedCommands?: BoardCommand[];
	planSteps?: PlanStep[];
}

export interface FlowScriptApplyResultLike {
	commands?: unknown[];
	board_commands?: BoardCommand[];
	diagnostics?: string[];
	final_board_node_count?: number;
}

export interface FlowScriptApplyOptions {
	allowDeletions?: boolean;
	suppressBlockedToast?: boolean;
}

export interface FlowCopilotProps {
	appId?: string;
	board: IBoard | null | undefined;
	catalogNodes?: INode[];
	selectedNodeIds: string[];
	onAcceptSuggestion: (suggestion: Suggestion) => void;
	onExecuteCommands?: (commands: BoardCommand[]) => void | Promise<void>;
	onApplyFlowScript?: (
		flowscript: string,
		options?: FlowScriptApplyOptions,
	) =>
		| undefined
		| FlowScriptApplyResultLike
		| Promise<undefined | FlowScriptApplyResultLike>;
	onApplyFlowIrCommit?: (
		token: FlowIrCommitToken,
	) => Promise<IApplyFlowIrCommitResponse>;
	onGhostNodesChange?: (suggestions: Suggestion[]) => void;
	onClearRunContext?: () => void;
	onClose?: () => void;
	onWorkspaceVisibleChange?: (visible: boolean) => void;
	mode?: Mode;
	embedded?: boolean;
	runContext?: ILogMetadata;
	onFocusNode?: (nodeId: string) => void;
	onSelectNodes?: (nodeIds: string[]) => void;
	initialPrompt?: string;
}

export const LOADING_PHASES: Record<LoadingPhase, LoadingPhaseInfo> = {
	initializing: {
		label: "Starting up",
		icon: null,
		color: "text-muted-foreground",
	},
	analyzing: {
		label: "Analyzing flow",
		icon: null,
		color: "text-blue-500",
	},
	searching: {
		label: "Searching catalog",
		icon: null,
		color: "text-violet-500",
	},
	reasoning: {
		label: "Reasoning",
		icon: null,
		color: "text-amber-500",
	},
	generating: {
		label: "Generating",
		icon: null,
		color: "text-emerald-500",
	},
	finalizing: {
		label: "Finalizing",
		icon: null,
		color: "text-primary",
	},
};
