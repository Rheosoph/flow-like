import type { IBoard, IVariable } from "../lib/schema/flow/board";

/**
 * A variable whose value is supplied per run rather than stored in the flow.
 * Secrets count: they are stripped from the board and have to come from
 * somewhere at run time just the same.
 */
export function isRuntimeConfigured(variable: IVariable): boolean {
	return variable.runtime_configured || variable.secret;
}

/**
 * A variable an event may override. Mirrors `allow_event_override` in
 * `resolve_variable_override` (packages/core/src/flow/execution.rs) — keep the
 * two in step or the UI will offer overrides the engine silently drops.
 */
export function isEventOverridable(variable: IVariable): boolean {
	return variable.exposed || isRuntimeConfigured(variable);
}

/**
 * Get all runtime-configured variables from a board (including secrets)
 */
export function getRuntimeConfiguredVariables(board: IBoard): IVariable[] {
	return Object.values(board.variables).filter(isRuntimeConfigured);
}

/**
 * Get IDs of all runtime-configured variables from a board
 */
export function getRuntimeConfiguredVariableIds(board: IBoard): string[] {
	return getRuntimeConfiguredVariables(board).map((v) => v.id);
}

/**
 * Check if a board has any nodes that require offline-only execution
 */
export function hasOfflineOnlyNodes(board: IBoard): boolean {
	return Object.values(board.nodes).some((node) => node.only_offline);
}

/**
 * Get all nodes that require offline-only execution
 */
export function getOfflineOnlyNodes(board: IBoard) {
	return Object.values(board.nodes).filter((node) => node.only_offline);
}
