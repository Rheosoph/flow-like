import type { IProfile } from "../types";

/**
 * Resolve the API origin (scheme + host, no trailing slash, no `/api/v1`)
 * the app is talking to. Single source of truth for anywhere the UI needs
 * to construct a URL served by the backend — API calls, sink triggers,
 * health checks, etc.
 *
 * Precedence: NEXT_PUBLIC_API_URL env override → profile.hub → hardcoded
 * default. `profile.secure` decides the protocol when the value is a bare
 * host (no scheme).
 */
export function getApiOrigin(profile?: Partial<IProfile> | null): string {
	const envOverride =
		typeof process !== "undefined" ? process.env?.NEXT_PUBLIC_API_URL : null;
	let raw =
		envOverride ??
		(profile?.hub as string | undefined) ??
		"api.flow-like.com";

	while (raw.endsWith("/")) {
		raw = raw.slice(0, -1);
	}
	if (raw.startsWith("http://") || raw.startsWith("https://")) {
		return raw;
	}
	const protocol = profile?.secure === false ? "http" : "https";
	return `${protocol}://${raw}`;
}
