export type BoardVersion = [number, number, number];

/**
 * Event payloads expose board versions as a loose number array. Keep version
 * pinning opt-in by accepting only a complete semantic-version tuple.
 */
export function normalizeBoardVersion(
	value: readonly number[] | null | undefined,
): BoardVersion | undefined {
	if (
		!Array.isArray(value) ||
		value.length !== 3 ||
		!value.every((part) => Number.isSafeInteger(part) && part >= 0)
	) {
		return undefined;
	}

	return [value[0], value[1], value[2]];
}

/**
 * A page event's pin only applies to executions against that event's board.
 * Actions targeting a different board continue to resolve that board's latest
 * version unless they carry their own explicit version contract.
 */
export function resolveEventBoardVersion(
	eventBoardId: string | null | undefined,
	eventBoardVersion: readonly number[] | null | undefined,
	targetBoardId: string | null | undefined,
): BoardVersion | undefined {
	if (!eventBoardId || eventBoardId !== targetBoardId) return undefined;
	return normalizeBoardVersion(eventBoardVersion);
}

/** Add a version only for an explicit pin; latest is represented by omission. */
export function withBoardVersion<T extends object>(
	payload: T,
	version: BoardVersion | undefined,
): T & { version?: BoardVersion } {
	return version ? { ...payload, version } : payload;
}
