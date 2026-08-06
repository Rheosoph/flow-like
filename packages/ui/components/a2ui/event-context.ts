const MAX_DEPTH = 4;
const MAX_ARRAY_ITEMS = 200;
const MAX_OBJECT_KEYS = 64;

function isPlainish(value: object): boolean {
	if (typeof Node !== "undefined" && value instanceof Node) return false;
	if (typeof Event !== "undefined" && value instanceof Event) return false;
	if ("$$typeof" in value) return false;
	return true;
}

/**
 * Reduce a renderer payload to something a board run can receive.
 *
 * Chart and table libraries hand their click callbacks live objects: DOM
 * nodes, React elements, scales, and parent back-references. Those either
 * throw on structured clone or serialize into megabytes, so events carry a
 * bounded, JSON-safe projection instead of the raw datum.
 */
export function toEventContextValue(value: unknown, depth = 0): unknown {
	if (value === null || value === undefined) return null;

	const type = typeof value;
	if (type === "string" || type === "boolean") return value;
	if (type === "number") return Number.isFinite(value as number) ? value : null;
	if (type === "bigint") return String(value);
	if (type === "function" || type === "symbol") return undefined;

	if (value instanceof Date) return value.toISOString();
	if (depth >= MAX_DEPTH) return undefined;

	if (Array.isArray(value)) {
		const items: unknown[] = [];
		for (const item of value.slice(0, MAX_ARRAY_ITEMS)) {
			items.push(toEventContextValue(item, depth + 1) ?? null);
		}
		return items;
	}

	if (type !== "object" || !isPlainish(value as object)) return undefined;

	const source = value as Record<string, unknown>;
	const result: Record<string, unknown> = {};
	let keys = 0;

	for (const [key, childValue] of Object.entries(source)) {
		if (keys >= MAX_OBJECT_KEYS) break;
		const projected = toEventContextValue(childValue, depth + 1);
		if (projected === undefined) continue;
		result[key] = projected;
		keys += 1;
	}

	return result;
}

/** Project every entry of an event context, dropping unusable values. */
export function toEventContext(
	context: Record<string, unknown>,
): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(context)) {
		const projected = toEventContextValue(value);
		if (projected !== undefined) result[key] = projected;
	}
	return result;
}
