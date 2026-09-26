import Dexie, { type EntityTable } from "dexie";

/**
 * A runtime variable value stored per-user, per-app.
 * These values are never sent to the server - they're only used for local execution.
 */
export interface IRuntimeVariableValue {
	/** Composite key: `${appId}:${variableId}` */
	id: string;
	appId: string;
	boardId: string;
	variableId: string;
	variableName: string;
	/** The value as a JSON-encoded byte array (matches Rust's default_value format) */
	value: number[];
	/** Whether this is a secret value (should be displayed as password field) */
	isSecret: boolean;
	updatedAt: string;
}

const runtimeVarsDB = new Dexie("RuntimeVariables") as Dexie & {
	values: EntityTable<IRuntimeVariableValue, "id">;
};

runtimeVarsDB.version(1).stores({
	values: "id, appId, boardId, variableId",
});

export { runtimeVarsDB };

/**
 * Get all runtime variable values for an app
 */
export async function getRuntimeVarsForApp(
	appId: string,
): Promise<IRuntimeVariableValue[]> {
	return runtimeVarsDB.values.where("appId").equals(appId).toArray();
}

/**
 * Get all runtime variable values for a specific board
 */
export async function getRuntimeVarsForBoard(
	appId: string,
	boardId: string,
): Promise<IRuntimeVariableValue[]> {
	return runtimeVarsDB.values.where({ appId, boardId }).toArray();
}

/**
 * Get a specific runtime variable value
 */
export async function getRuntimeVar(
	appId: string,
	variableId: string,
): Promise<IRuntimeVariableValue | undefined> {
	const id = `${appId}:${variableId}`;
	return runtimeVarsDB.values.get(id);
}

/**
 * Set a runtime variable value
 */
export async function setRuntimeVar(
	appId: string,
	boardId: string,
	variableId: string,
	variableName: string,
	value: number[],
	isSecret: boolean,
): Promise<void> {
	await setRuntimeVars(appId, boardId, [
		{ variableId, variableName, value, isSecret },
	]);
}

/** Save a batch together so a failed write preserves every previous value. */
export async function setRuntimeVars(
	appId: string,
	boardId: string,
	values: Array<
		Pick<
			IRuntimeVariableValue,
			"variableId" | "variableName" | "value" | "isSecret"
		>
	>,
): Promise<void> {
	if (values.length === 0) return;
	const updatedAt = new Date().toISOString();
	await runtimeVarsDB.transaction("rw", runtimeVarsDB.values, async () => {
		await runtimeVarsDB.values.bulkPut(
			values.map((value) => ({
				...value,
				id: `${appId}:${value.variableId}`,
				appId,
				boardId,
				updatedAt,
			})),
		);
	});
}

/**
 * Delete a runtime variable value
 */
export async function deleteRuntimeVar(
	appId: string,
	variableId: string,
): Promise<void> {
	const id = `${appId}:${variableId}`;
	await runtimeVarsDB.values.delete(id);
}

/**
 * Delete all runtime variable values for an app
 */
export async function deleteRuntimeVarsForApp(appId: string): Promise<void> {
	await runtimeVarsDB.values.where("appId").equals(appId).delete();
}

/**
 * Check if all required runtime variables for a board are configured
 */
export async function hasAllRuntimeVars(
	appId: string,
	requiredVariableIds: string[],
): Promise<boolean> {
	if (requiredVariableIds.length === 0) return true;

	const existingVars = await runtimeVarsDB.values
		.where("appId")
		.equals(appId)
		.toArray();

	const existingIds = new Set(existingVars.map((v) => v.variableId));
	return requiredVariableIds.every((id) => existingIds.has(id));
}

/**
 * Get missing runtime variable IDs for a board
 */
export async function getMissingRuntimeVars(
	appId: string,
	requiredVariableIds: string[],
): Promise<string[]> {
	if (requiredVariableIds.length === 0) return [];

	const existingVars = await runtimeVarsDB.values
		.where("appId")
		.equals(appId)
		.toArray();

	const existingIds = new Set(existingVars.map((v) => v.variableId));
	return requiredVariableIds.filter((id) => !existingIds.has(id));
}
