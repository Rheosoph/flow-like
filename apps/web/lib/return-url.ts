const RETURN_URL_KEY = "flow-like-return-url";
const MAX_AGE_MS = 60 * 60 * 1000;

interface StoredReturnUrl {
	url: string;
	ts: number;
}

/**
 * Only same-origin relative paths may be used as post-login redirect targets —
 * anything else (absolute URLs, protocol-relative `//host`) would let a
 * crafted login round trip bounce the user to an attacker-chosen site.
 */
export function sanitizeReturnUrl(url: unknown): string | null {
	if (typeof url !== "string") return null;
	if (!url.startsWith("/") || url.startsWith("//")) return null;
	return url === "/" ? null : url;
}

export function currentRelativeUrl(): string | undefined {
	if (typeof window === "undefined") return undefined;
	return (
		sanitizeReturnUrl(
			window.location.pathname + window.location.search + window.location.hash,
		) ?? undefined
	);
}

export function saveReturnUrl(url: string): void {
	const sanitized = sanitizeReturnUrl(url);
	if (!sanitized) return;
	const record = JSON.stringify({
		url: sanitized,
		ts: Date.now(),
	} satisfies StoredReturnUrl);
	// Dual-write: localStorage survives cross-context mobile redirects,
	// sessionStorage is the fallback where localStorage is blocked.
	try {
		localStorage.setItem(RETURN_URL_KEY, record);
	} catch {}
	try {
		sessionStorage.setItem(RETURN_URL_KEY, record);
	} catch {}
}

function parseStored(raw: string | null): string | null {
	if (!raw) return null;
	try {
		const parsed = JSON.parse(raw) as Partial<StoredReturnUrl>;
		if (typeof parsed?.url === "string" && typeof parsed?.ts === "number") {
			if (Date.now() - parsed.ts > MAX_AGE_MS) return null;
			return sanitizeReturnUrl(parsed.url);
		}
		return null;
	} catch {
		// Legacy plain-string values predate the timestamped format.
		return sanitizeReturnUrl(raw);
	}
}

export function consumeReturnUrl(): string | null {
	let stored: string | null = null;
	try {
		stored = parseStored(sessionStorage.getItem(RETURN_URL_KEY));
	} catch {}
	if (!stored) {
		try {
			stored = parseStored(localStorage.getItem(RETURN_URL_KEY));
		} catch {}
	}
	clearReturnUrl();
	return stored;
}

export function clearReturnUrl(): void {
	try {
		localStorage.removeItem(RETURN_URL_KEY);
	} catch {}
	try {
		sessionStorage.removeItem(RETURN_URL_KEY);
	} catch {}
}
