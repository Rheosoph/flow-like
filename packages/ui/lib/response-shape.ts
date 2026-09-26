/**
 * Guards for values that crossed a process boundary (HTTP, a restored query
 * cache). Their TypeScript types are claims, not guarantees: an outage, an older
 * hub or a stale persisted entry can hand a caller any shape.
 */

export function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The value when it is an array, otherwise an empty one. */
export function asArray<T>(value: readonly T[] | null | undefined): T[] {
	return Array.isArray(value) ? (value as T[]) : [];
}
