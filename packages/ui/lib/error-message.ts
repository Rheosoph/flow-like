export function getErrorMessage(
	error: unknown,
	fallback = "Unknown error",
): string {
	if (error instanceof Error && error.message.trim()) {
		return error.message;
	}

	if (typeof error === "string") {
		const message = error.trim();
		if (message) {
			return message;
		}
	}

	if (error && typeof error === "object") {
		const record = error as Record<string, unknown>;
		for (const key of ["message", "error", "details"]) {
			const value = record[key];
			if (typeof value === "string" && value.trim()) {
				return value;
			}
		}

		if (record.cause !== undefined) {
			const causeMessage = getErrorMessage(record.cause, "");
			if (causeMessage) {
				return causeMessage;
			}
		}

		try {
			const serialized = JSON.stringify(error);
			if (serialized && serialized !== "{}") {
				return serialized;
			}
		} catch {}
	}

	return fallback;
}
