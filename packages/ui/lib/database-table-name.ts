const VALID_DATABASE_TABLE_IDENTIFIER = /^[\p{L}\p{N}_.-]+$/u;

/**
 * Keep already-valid physical table identifiers byte-for-byte. Human-facing labels are normalized
 * to a stable snake_case identifier because the database API deliberately rejects spaces and other
 * punctuation. Returning this mapping to the agent lets it preserve the requested semantic name
 * without burning another specialist turn probing for a display-name feature that does not exist.
 */
export function normalizeDatabaseTableIdentifier(value: string): string {
	if (
		value.length <= 256 &&
		!value.includes("..") &&
		VALID_DATABASE_TABLE_IDENTIFIER.test(value)
	) {
		return value;
	}
	const normalized = value
		.normalize("NFKD")
		.replace(/\p{M}+/gu, "")
		.toLowerCase()
		.replace(/&/g, " and ")
		.replace(/[^\p{L}\p{N}]+/gu, "_")
		.replace(/^_+|_+$/g, "")
		.slice(0, 256);
	if (!normalized) {
		throw new Error(
			"create_table table_name must contain at least one letter or number.",
		);
	}
	return normalized;
}
