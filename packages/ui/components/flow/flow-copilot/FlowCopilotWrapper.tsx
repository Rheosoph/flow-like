"use client";

/**
 * FlowCopilotWrapper - Backward-compatible wrapper for the unified FlowPilot
 *
 * This wrapper maintains the original FlowCopilotProps interface while
 * internally using the new unified FlowPilot component with agentMode="board".
 *
 * This allows gradual migration to the unified component without breaking
 * existing imports and usage.
 */

import { FlowPilot } from "../../flowpilot";
import type { FlowCopilotProps } from "./types";

export function FlowCopilotWrapper({
	appId,
	board,
	catalogNodes,
	selectedNodeIds,
	onAcceptSuggestion,
	onExecuteCommands,
	onFocusNode,
	onSelectNodes,
	runContext,
	initialPrompt,
	onClose,
	onWorkspaceVisibleChange,
	mode,
	embedded,
	onGhostNodesChange,
	onClearRunContext,
}: FlowCopilotProps) {
	return (
		<FlowPilot
			agentMode="board"
			title="FlowPilot"
			appId={appId}
			board={board}
			catalogNodes={catalogNodes}
			selectedNodeIds={selectedNodeIds}
			onAcceptSuggestion={onAcceptSuggestion}
			onExecuteCommands={onExecuteCommands}
			onFocusNode={onFocusNode}
			onSelectNodes={onSelectNodes}
			runContext={
				runContext
					? {
							run_id: runContext.run_id,
							app_id: runContext.app_id,
							board_id: runContext.board_id,
							event_id: runContext.event_id,
						}
					: undefined
			}
			initialPrompt={initialPrompt}
			onClose={onClose}
			onWorkspaceVisibleChange={onWorkspaceVisibleChange}
		/>
	);
}

// Re-export for backward compatibility
export { FlowCopilotWrapper as FlowCopilot };
