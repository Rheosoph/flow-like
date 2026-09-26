import type { WidgetContract } from "./contract";
import type { QueryPayload, QueryResultPayload } from "./protocol";
import { validateSchema } from "./validate";

export type WidgetQueryHandler = (args: unknown) => unknown;

/**
 * Invoke one registered query handler. Read queries retain their permissive
 * behavior; mutation schemas are enforced at the side-effect boundary.
 */
export async function dispatchWidgetQuery(
	contract: WidgetContract | null | undefined,
	handlers: ReadonlyMap<string, WidgetQueryHandler>,
	payload: QueryPayload,
): Promise<QueryResultPayload> {
	const handler = handlers.get(payload.name);
	if (!handler) {
		return {
			queryId: payload.queryId,
			ok: false,
			error: `Unknown query "${payload.name}"`,
		};
	}

	const query = contract?.queries?.[payload.name];
	const mutation = query?.mutation === true;
	if (mutation) {
		const validation = validateSchema(query.argsSchema, payload.args);
		if (!validation.valid) {
			return {
				queryId: payload.queryId,
				ok: false,
				error: `Invalid arguments for mutation "${payload.name}": ${validation.errors.join("; ")}`,
			};
		}
	}

	try {
		const value = await handler(payload.args);
		if (mutation) {
			const validation = validateSchema(query.resultSchema, value);
			if (!validation.valid) {
				return {
					queryId: payload.queryId,
					ok: false,
					error: `Invalid result from mutation "${payload.name}": ${validation.errors.join("; ")}`,
				};
			}
		}
		return { queryId: payload.queryId, ok: true, value };
	} catch (error) {
		return {
			queryId: payload.queryId,
			ok: false,
			error: error instanceof Error ? error.message : String(error),
		};
	}
}
