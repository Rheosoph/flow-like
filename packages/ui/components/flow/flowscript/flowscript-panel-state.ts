export interface FlowScriptApplyState {
	readOnly: boolean;
	dirty: boolean;
	applying: boolean;
	loading: boolean;
	boardChangedBehindEdits: boolean;
}

/**
 * Keep every FlowScript apply entry point behind the same state gate. In
 * particular, a dirty script based on an older board snapshot must be refreshed
 * before it can be reconciled safely.
 */
export function canApplyFlowScript({
	readOnly,
	dirty,
	applying,
	loading,
	boardChangedBehindEdits,
}: FlowScriptApplyState): boolean {
	return (
		dirty && !readOnly && !applying && !loading && !boardChangedBehindEdits
	);
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
