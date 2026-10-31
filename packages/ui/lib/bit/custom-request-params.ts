const UNSUPPORTED_PROVIDERS = new Set([
	"custom:claude-code",
	"custom:codex",
	"custom:github-copilot",
	"custom:microsoft-copilot",
]);

export function supportsCustomRequestParams(providerName: unknown): boolean {
	if (typeof providerName !== "string") return false;
	const name = providerName.trim().toLowerCase();
	return name.startsWith("custom:") && !UNSUPPORTED_PROVIDERS.has(name);
}

export function validateCustomRequestParams(value: unknown): string | null {
	if (value === undefined) return null;
	if (value === null || typeof value !== "object" || Array.isArray(value))
		return "Request parameters must be a JSON object.";
	if (Object.hasOwn(value, "stream"))
		return "Remove stream from request parameters. The app controls streaming mode.";
	return null;
}

export function parseCustomRequestParams(text: string): {
	value?: Record<string, unknown>;
	error: string | null;
} {
	if (!text.trim()) return { error: null };
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		return { error: "Request parameters must be valid JSON." };
	}
	const error = validateCustomRequestParams(value);
	return error
		? { error }
		: { value: value as Record<string, unknown>, error: null };
}

export function withCustomRequestParams(
	params: Record<string, unknown>,
	requestParams: Record<string, unknown> | undefined,
): Record<string, unknown> {
	const { request_params: _previous, ...rest } = params;
	return requestParams === undefined
		? rest
		: { ...rest, request_params: requestParams };
}
