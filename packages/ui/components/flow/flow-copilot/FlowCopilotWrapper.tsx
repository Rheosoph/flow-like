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

import { memo, useMemo } from "react";
import { FlowPilot } from "../../flowpilot";
import type { FlowCopilotProps } from "./types";

function FlowCopilotWrapperImpl({
	appId,
	board,
	catalogNodes,
	selectedNodeIds,
	onAcceptSuggestion,
	onExecuteCommands,
	onApplyFlowScript,
	onFocusNode,
	onSelectNodes,
	runContext,
	initialPrompt,
	onClose,
	onWorkspaceVisibleChange,
}: FlowCopilotProps) {
	// Stabilize the runContext object identity so the memoized FlowPilot only
	// re-renders when the underlying run actually changes. The parent passes a
	// store-backed value, so its identity is already stable between run changes.
	const stableRunContext = useMemo(
		() =>
			runContext
				? {
						run_id: runContext.run_id,
						app_id: runContext.app_id,
						board_id: runContext.board_id,
						event_id: runContext.event_id,
					}
				: undefined,
		[runContext],
	);

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
			onApplyFlowScript={onApplyFlowScript}
			onFocusNode={onFocusNode}
			onSelectNodes={onSelectNodes}
			runContext={stableRunContext}
			initialPrompt={initialPrompt}
			onClose={onClose}
			onWorkspaceVisibleChange={onWorkspaceVisibleChange}
		/>
	);
}

export const FlowCopilotWrapper = memo(FlowCopilotWrapperImpl);

// Re-export for backward compatibility
export { FlowCopilotWrapper as FlowCopilot };
