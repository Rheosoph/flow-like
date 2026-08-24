export interface FlowScriptApplyState {
	readOnly: boolean;
	dirty: boolean;
	applying: boolean;
	loading: boolean;
	boardChangedBehindEdits: boolean;
	/** Statement-merge conflicts awaiting a keep-mine/take-theirs decision. */
	unresolvedConflicts?: boolean;
}

/**
 * Keep every FlowScript apply entry point behind the same state gate. In
 * particular, a dirty script based on an older board snapshot must be refreshed
 * before it can be reconciled safely, and a half-resolved statement merge must
 * never be applied over the statements still in conflict.
 */
export function canApplyFlowScript({
	readOnly,
	dirty,
	applying,
	loading,
	boardChangedBehindEdits,
	unresolvedConflicts,
}: FlowScriptApplyState): boolean {
	return (
		dirty &&
		!readOnly &&
		!applying &&
		!loading &&
		!boardChangedBehindEdits &&
		!(unresolvedConflicts ?? false)
	);
}

export type FlowScriptScopeMode =
	| { kind: "full" }
	| { kind: "scoped"; nodeIds: string[] };

/**
 * Selection-scoped editing needs `getFlowScriptScoped` on the backend; without
 * it a scope request silently degrades to the full-board render so the panel
 * never shows a scoped banner it cannot honor on apply.
 */
export function resolveFlowScriptScope(
	requestedNodeIds: readonly string[] | undefined,
	backendSupportsScope: boolean,
): FlowScriptScopeMode {
	if (
		!backendSupportsScope ||
		!requestedNodeIds ||
		requestedNodeIds.length === 0
	) {
		return { kind: "full" };
	}
	return { kind: "scoped", nodeIds: [...requestedNodeIds] };
}

/** A source-only repair still needs a canonical reload even when no board command executed. */
export function shouldReloadFlowScriptAfterApply({
	commandCount,
	correctionCount,
	diagnosticCount,
}: {
	commandCount: number;
	correctionCount: number;
	diagnosticCount: number;
}): boolean {
	return commandCount > 0 || (diagnosticCount === 0 && correctionCount > 0);
}
