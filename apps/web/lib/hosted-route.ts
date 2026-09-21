export type HostedKind = "c" | "f" | "u";

export function isHostedFrontendPath(pathname: string | null): boolean {
	return pathname !== null && /^\/(c|f|u)(?:\/|$)/.test(pathname);
}

export interface HostedTarget {
	kind: HostedKind;
	slug: string;
	variant?: string;
}

/** Query URLs work on static hosts without a rewrite rule. */
export function parseHostedTarget(url: URL): HostedTarget | null {
	const match = /^\/(c|f|u)(?:\/([^/]+))?\/?$/.exec(url.pathname);
	if (!match) return null;
	let slug: string;
	try {
		slug = match[2]
			? decodeURIComponent(match[2])
			: (url.searchParams.get("event") ?? "");
	} catch {
		return null;
	}
	if (!/^[a-zA-Z0-9_-]{1,128}$/.test(slug)) return null;
	const variant = url.searchParams.get("__variant");
	return {
		kind: match[1] as HostedKind,
		slug,
		...(variant ? { variant } : {}),
	};
}

/** Auth can only return to a hosted surface on this same origin. */
export function hostedReturnPath(value: unknown): string | null {
	if (
		typeof value !== "string" ||
		!value.startsWith("/") ||
		value.includes("\\") ||
		Array.from(value).some(
			(character) =>
				character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127,
		)
	)
		return null;
	const origin = "https://hosted.invalid";
	try {
		const url = new URL(value, origin);
		if (url.origin !== origin || !parseHostedTarget(url)) return null;
		return url.pathname + url.search + url.hash;
	} catch {
		return null;
	}
}

export function hostedApiPath(target: HostedTarget): string {
	return `frontend/${target.kind}/${encodeURIComponent(target.slug)}`;
}

export function hostedNavigationPath(
	route: string,
	routes: readonly { path: string; event_id: string; kind: HostedKind }[],
	queryParams: Record<string, string> = {},
	queryRouting = false,
): string {
	const origin = "https://hosted.invalid";
	const url = new URL(route, origin);
	if (url.origin !== origin || !/^https?:$/.test(url.protocol))
		throw new Error("This navigation does not point to a published app route.");
	const path =
		url.pathname === "/use"
			? (url.searchParams.get("route") ?? "/")
			: url.pathname;
	const normalizePath = (value: string) => value.replace(/\/+$/, "") || "/";
	const target = routes.find(
		(candidate) => normalizePath(candidate.path) === normalizePath(path),
	);
	if (!target)
		throw new Error("This page has not been published for this interface.");
	const params = new URLSearchParams(url.search);
	for (const reserved of ["id", "event", "eventId", "route", "sessionId"])
		params.delete(reserved);
	for (const [key, value] of Object.entries(queryParams))
		params.set(key, value);
	const pathname = queryRouting
		? `/${target.kind}`
		: `/${target.kind}/${encodeURIComponent(target.event_id)}`;
	if (queryRouting) params.set("event", target.event_id);
	return pathname + (params.size ? `?${params}` : "") + url.hash;
}

let sessionId: string | undefined;
export function hostedSessionId(): string {
	if (sessionId) return sessionId;
	try {
		const stored = sessionStorage.getItem("flow-like-hosted-session");
		if (stored && /^[a-zA-Z0-9_-]{16,128}$/.test(stored)) {
			sessionId = stored;
			return stored;
		}
	} catch {}
	sessionId = crypto.randomUUID();
	try {
		sessionStorage.setItem("flow-like-hosted-session", sessionId);
	} catch {}
	return sessionId;
}
