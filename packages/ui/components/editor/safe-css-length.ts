/** Stored layout dimensions accept lengths, never CSS expressions or declarations. */
export function safeCssLength(value: unknown): number | string | undefined {
	if (typeof value === "number")
		return Number.isFinite(value) && value >= 0 ? value : undefined;
	if (typeof value !== "string") return undefined;
	const length = value.trim();
	return /^(?:\d+(?:\.\d+)?|\.\d+)(?:px|%|em|rem|vw|vh)$/.test(length)
		? length
		: undefined;
}
