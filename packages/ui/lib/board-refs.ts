import type { IBoard } from "./schema/flow/board";

/** `fix_refs` stores `""` like any other text, so an empty description arrives as this key. */
export const EMPTY_STRING_REF = "16248035215404677707";

/**
 * Only own keys are refs: a description or schema that happens to read
 * `constructor` or `__proto__` must not resolve to `Object.prototype` members.
 */
function ownRef(
	refs: IBoard["refs"] | undefined,
	key: string,
): string | undefined {
	return refs && Object.prototype.hasOwnProperty.call(refs, key)
		? refs[key]
		: undefined;
}

/**
 * Board node/pin descriptions and schemas are content-addressed keys into `board.refs`. Hydrated
 * catalog nodes carry plain text instead, which is returned unchanged.
 */
export function resolveBoardRef(
	value: string | null | undefined,
	refs: IBoard["refs"] | undefined,
): string {
	if (!value || value === EMPTY_STRING_REF) return "";
	return ownRef(refs, value) ?? value;
}

/** Copies every ref a clipboard payload still points to, so a paste elsewhere can resolve them. */
export function collectBoardRefs(
	values: Iterable<string | null | undefined>,
	refs: IBoard["refs"] | undefined,
): Record<string, string> {
	const collected: Record<string, string> = {};
	for (const value of values) {
		if (!value) continue;
		const resolved = ownRef(refs, value);
		if (resolved !== undefined) collected[value] = resolved;
	}
	return collected;
}
