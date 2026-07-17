export type ExecutionElements = Record<string, unknown>;

interface ExecutionElementsResponse {
	elements: ExecutionElements;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Unwraps the API response while retaining compatibility with hubs that still
 * return the element map directly.
 */
export function executionElementsFromResponse(
	response: ExecutionElementsResponse | ExecutionElements | unknown,
): ExecutionElements {
	if (!isRecord(response)) return {};

	if ("elements" in response) {
		return isRecord(response.elements) ? response.elements : {};
	}

	return response;
}
