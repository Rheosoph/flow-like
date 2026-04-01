import { AuthError } from "./errors.js";
import type { AuthConfig } from "./types.js";

const PAT_PREFIX = "pat_";
const API_KEY_PREFIX = "flk_";

export function resolveAuth(pat?: string, apiKey?: string): AuthConfig {
	const token =
		pat ?? apiKey ?? process.env.FLOW_LIKE_PAT ?? process.env.FLOW_LIKE_API_KEY;

	if (!token) {
		throw new AuthError(
			"No authentication provided. Set FLOW_LIKE_PAT or FLOW_LIKE_API_KEY, or pass pat/apiKey in options.",
		);
	}

	if (token.startsWith(PAT_PREFIX)) {
		return { type: "pat", token };
	}

	if (token.startsWith(API_KEY_PREFIX)) {
		return { type: "api_key", token };
	}

	throw new AuthError(
		`Invalid token format. Expected prefix "${PAT_PREFIX}" (PAT) or "${API_KEY_PREFIX}" (API Key).`,
	);
}

export function buildAuthHeaders(auth: AuthConfig): Record<string, string> {
	if (auth.type === "pat") {
		return { Authorization: auth.token };
	}
	return { "X-API-Key": auth.token };
}
