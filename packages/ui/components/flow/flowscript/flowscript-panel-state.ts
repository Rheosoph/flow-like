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
	| { kind: "scoped"; nodeIds: string[] }
	/** One virtual file of a modular board: `"main"` or a module layer id. */
	| { kind: "file"; file: string };

/** What the panel knows about the board's files when it resolves its scope. */
export interface FlowScriptFileScopeInput {
	/** The board carries at least one module layer, so it has more than one file. */
	hasModules: boolean;
	/** `getFlowScriptFile` exists on the backend. */
	backendSupportsFiles: boolean;
	/** The file the canvas is on: `"main"` or a module layer id. */
	file: string;
}

/**
 * Selection-scoped editing needs `getFlowScriptScoped` on the backend, per-file editing needs
 * `getFlowScriptFile`; without the respective method the request silently degrades to the
 * full-board render, so the panel never shows a banner (or a file tab) it cannot honor on apply.
 *
 * A selection scope OVERRIDES the file mode while it is active: the selection is the narrower,
 * explicitly requested view. Exiting it drops back to the file the canvas is on. A board without
 * modules has exactly one file and behaves as it always did — `full`.
 */
export function resolveFlowScriptScope(
	requestedNodeIds: readonly string[] | undefined,
	backendSupportsScope: boolean,
	files?: FlowScriptFileScopeInput,
): FlowScriptScopeMode {
	if (backendSupportsScope && requestedNodeIds && requestedNodeIds.length > 0) {
		return { kind: "scoped", nodeIds: [...requestedNodeIds] };
	}
	if (files?.hasModules && files.backendSupportsFiles) {
		return { kind: "file", file: files.file };
	}
	return { kind: "full" };
}

/** Set equality on scope node ids — order and duplicates are irrelevant. */
export function sameScopeNodeIds(
	a: readonly string[],
	b: readonly string[],
): boolean {
	const setA = new Set(a);
	const setB = new Set(b);
	if (setA.size !== setB.size) return false;
	for (const id of setA) if (!setB.has(id)) return false;
	return true;
}

/**
 * Joining a peer's shared scope validates their node ids against the LOCAL
 * board first: unknown ids (deleted nodes, other boards) are dropped, and an
 * all-unknown scope yields an empty list so the caller can refuse to open a
 * session on a selection that no longer exists.
 */
export function resolveJoinableScopeNodeIds(
	requestedNodeIds: readonly string[],
	isKnownNodeId: (nodeId: string) => boolean,
): string[] {
	return [...new Set(requestedNodeIds)].filter(isKnownNodeId);
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
