function asRecord(value: unknown): Record<string, unknown> | null {
	return value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: null;
}

export function getModelCapability(
	parameters: unknown,
	field: string,
): number | null {
	const params = asRecord(parameters);
	const unavailable = params?.classification_unavailable_fields;
	// The catalog stores zero for unavailable fields to satisfy the numeric schema.
	if (Array.isArray(unavailable) && unavailable.includes(field)) return null;
	const value = asRecord(params?.model_classification)?.[field];
	return typeof value === "number" &&
		Number.isFinite(value) &&
		value >= 0 &&
		value <= 1
		? value
		: null;
}

/** Sort descending, leaving unknown capabilities after measured zero scores. */
export function compareModelCapability(
	aParameters: unknown,
	bParameters: unknown,
	field: string,
): number {
	const a = getModelCapability(aParameters, field);
	const b = getModelCapability(bParameters, field);
	if (a === null) return b === null ? 0 : 1;
	if (b === null) return -1;
	return b - a;
}

export function meetsModelCapabilityMinimum(
	parameters: unknown,
	field: string,
	minimum: number,
): boolean {
	if (minimum <= 0) return true;
	const value = getModelCapability(parameters, field);
	return value !== null && value >= minimum;
}
