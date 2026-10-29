const HEADER = "x-flowlike-benchmark-request-id";
const UUID =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

/** Retain only the proxy's UUID, never arbitrary response header contents. */
export function benchmarkRequestId(headers) {
	if (!headers || typeof headers !== "object" || Array.isArray(headers)) return;
	const matches = Object.entries(headers).filter(
		([name]) => name.toLowerCase() === HEADER,
	);
	if (matches.length !== 1) return;
	const value = matches[0][1];
	return typeof value === "string" && UUID.test(value) ? value : undefined;
}
