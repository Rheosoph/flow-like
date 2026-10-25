import { normalizeGeometryValue } from "./geometry";
import type { IBoard, IVariable } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";
import { IValueType, IVariableType } from "./schema/flow/pin";
import { convertJsonToUint8Array, parseUint8ArrayToJson } from "./uint8";

/**
 * A variable whose value is supplied per run rather than stored in the flow.
 * Secrets count: they are stripped from the board and have to come from
 * somewhere at run time just the same.
 */
export function isRuntimeConfigured(variable: IVariable): boolean {
	return variable.runtime_configured || variable.secret;
}

/**
 * A variable the app operator may set from the configuration surface. The
 * `editable` half is load-bearing: `UpsertVariableCommand` rejects a locked
 * variable in both validate and execute, so an editor that ignores it offers a
 * write the engine will refuse.
 */
export function isSharedConfigurable(variable: IVariable): boolean {
	return variable.exposed && variable.editable;
}

/**
 * Whether a variable currently holds a usable value. Booleans, numbers and
 * structured values count as configured as soon as they decode; strings
 * (including paths) must be non-empty.
 *
 * This is the single definition of "configured" for the whole product — the
 * row indicator, the readiness meter, the save guard and the pre-run gate all
 * call it. Splitting it is how an empty string ends up creating a stored row
 * that satisfies the gate and then runs the flow with nothing in it.
 */
export function isRuntimeVariableConfigured(
	variable: IVariable,
	refs?: Record<string, string>,
): boolean {
	const decoded = parseUint8ArrayToJson(variable.default_value);
	if (decoded === undefined || decoded === null) return false;
	if (variable.data_type === IVariableType.Geometry) {
		try {
			normalizeGeometryValue(decoded, {
				schema: variable.schema,
				refs,
				valueType: variable.value_type,
			});
			return true;
		} catch {
			return false;
		}
	}
	if (typeof decoded === "string") return decoded.trim().length > 0;
	return true;
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
 * Check if a board has any nodes that require offline-only execution. Servers refuse such a
 * board whole, so layer nodes count as much as root ones.
 */
export function hasOfflineOnlyNodes(board: IBoard): boolean {
	const isOfflineOnly = (node: INode) => Boolean(node.only_offline);
	return (
		Object.values(board.nodes).some(isOfflineOnly) ||
		Object.values(board.layers ?? {}).some((layer) =>
			Object.values(layer.nodes ?? {}).some(isOfflineOnly),
		)
	);
}

/**
 * Get all nodes that require offline-only execution
 */
export function getOfflineOnlyNodes(board: IBoard) {
	return Object.values(board.nodes).filter((node) => node.only_offline);
}

/**
 * Resolve the initial value of a runtime variable from a saved value or its
 * board default. A plain Boolean with no value is seeded to `false` so the
 * rendered switch (which always shows a definite on/off state) matches the
 * stored value instead of being counted as "not configured".
 */
export function seedRuntimeVariable(
	variable: IVariable,
	existingBytes?: number[] | null,
): IVariable {
	let bytes = existingBytes ?? variable.default_value ?? null;
	if (
		(!bytes || bytes.length === 0) &&
		variable.value_type === IValueType.Normal &&
		variable.data_type === IVariableType.Boolean
	) {
		bytes = convertJsonToUint8Array(false) ?? null;
	}
	return { ...variable, default_value: bytes };
}
