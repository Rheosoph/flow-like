export function camelToSnakeCase(key: string): string {
	return key.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

export function toSnakeCaseKeys(value: unknown): unknown {
	if (Array.isArray(value)) {
		return value.map(toSnakeCaseKeys);
	}

	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value as Record<string, unknown>).map(([key, nested]) => [
				camelToSnakeCase(key),
				toSnakeCaseKeys(nested),
			]),
		);
	}

	return value;
}
