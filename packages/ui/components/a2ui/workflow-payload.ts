/**
 * Remove transient local upload previews without changing JSON semantics.
 * Undefined object fields are omitted (as JSON.stringify does), while undefined
 * array slots become null so indexes remain stable.
 */
export function compactWorkflowPayload(value: unknown): unknown {
	if (value === undefined) return undefined;
	if (value === null) return null;

	if (Array.isArray(value)) {
		return value.map((item) => compactWorkflowPayload(item) ?? null);
	}

	if (typeof value === "object") {
		const source = value as Record<string, unknown>;
		const isUploadValue =
			typeof source.name === "string" &&
			typeof source.size === "number" &&
			typeof source.type === "string";
		const compacted: Record<string, unknown> = {};

		for (const [key, childValue] of Object.entries(source)) {
			if (key === "dataUrl" && isUploadValue) continue;
			const nextValue = compactWorkflowPayload(childValue);
			if (nextValue !== undefined) compacted[key] = nextValue;
		}

		return compacted;
	}

	return value;
}
