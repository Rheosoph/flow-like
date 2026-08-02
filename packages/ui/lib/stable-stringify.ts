/**
 * JSON.stringify with deterministic key order, so two structurally equal objects
 * always produce the same string.
 *
 * Change detection compares a working copy against the saved record. Plain
 * JSON.stringify is insertion-order sensitive, so `{a, b}` and `{b, a}` compare
 * as different and the UI reports edits nobody made — which is especially easy
 * to hit with config blobs that get decoded, spread and re-encoded on every keystroke.
 *
 * Array order is preserved: for arrays, order is meaningful.
 */
export function stableStringify(value: unknown): string {
	return JSON.stringify(value, (_key, val) => {
		if (val && typeof val === "object" && !Array.isArray(val)) {
			return Object.fromEntries(
				Object.entries(val as Record<string, unknown>).sort(([a], [b]) =>
					a.localeCompare(b),
				),
			);
		}
		return val;
	});
}
