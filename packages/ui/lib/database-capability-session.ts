import type { IDatabaseSchemaField } from "../state/backend-state/db-state";

export interface PendingDatabaseSchema {
	appId: string;
	tableName: string;
	userScoped: boolean;
	ifNotExists: boolean;
	fields: IDatabaseSchemaField[];
	requestedAtMs: number;
}

const MAX_PENDING_SCHEMAS = 128;

const appsWithoutExplicitSchemaCreate = new Set<string>();
const pendingSchemas = new Map<string, PendingDatabaseSchema>();

function pendingSchemaKey(
	appId: string,
	tableName: string,
	userScoped: boolean,
) {
	return `${appId}\u0000${userScoped ? "user" : "project"}\u0000${tableName}`;
}

/**
 * A single POST 405 proves explicit schema creation is unavailable for this frontend session.
 * Remembering that capability avoids repeated requests and approvals during a rolling deploy.
 */
export function markExplicitSchemaCreateUnavailable(appId: string) {
	appsWithoutExplicitSchemaCreate.add(appId);
}

export function isExplicitSchemaCreateUnavailable(appId: string) {
	return appsWithoutExplicitSchemaCreate.has(appId);
}

/** Retain every requested schema in memory so no build intent is lost while the API is stale. */
export function retainPendingDatabaseSchema(
	schema: Omit<PendingDatabaseSchema, "requestedAtMs">,
) {
	const key = pendingSchemaKey(
		schema.appId,
		schema.tableName,
		schema.userScoped,
	);
	pendingSchemas.delete(key);
	pendingSchemas.set(key, {
		...schema,
		fields: schema.fields.map((field) => ({ ...field })),
		requestedAtMs: Date.now(),
	});
	while (pendingSchemas.size > MAX_PENDING_SCHEMAS) {
		const oldest = pendingSchemas.keys().next().value;
		if (typeof oldest !== "string") break;
		pendingSchemas.delete(oldest);
	}
	let appPendingSchemaCount = 0;
	for (const pending of pendingSchemas.values()) {
		if (pending.appId === schema.appId) appPendingSchemaCount += 1;
	}
	return appPendingSchemaCount;
}

export function getPendingDatabaseSchemas(): PendingDatabaseSchema[] {
	return Array.from(pendingSchemas.values(), (schema) => ({
		...schema,
		fields: schema.fields.map((field) => ({ ...field })),
	}));
}

export function shouldSkipUnavailableCreateTableApproval(
	toolName: string,
	args: Record<string, unknown>,
	defaultAppId?: string,
) {
	const requestedAppId = args.app_id ?? args.appId ?? defaultAppId;
	return (
		typeof requestedAppId === "string" &&
		appsWithoutExplicitSchemaCreate.has(requestedAppId) &&
		toolName === "database_tool" &&
		args.operation === "create_table"
	);
}

/** Test-only reset. Production deliberately keeps the latch until the frontend reloads. */
export function resetDatabaseCapabilitySessionForTests() {
	appsWithoutExplicitSchemaCreate.clear();
	pendingSchemas.clear();
}
