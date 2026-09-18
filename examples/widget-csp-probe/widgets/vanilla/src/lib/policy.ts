/** Every CSP meta in the document; the server leaves exactly one */
export function cspMetas(): Element[] {
	return [...document.querySelectorAll("meta[http-equiv]")].filter(
		(meta) =>
			meta.getAttribute("http-equiv")?.toLowerCase() ===
			"content-security-policy",
	);
}

/** The server-injected policy, or null when the document has no single CSP meta */
export function servedPolicy(): string | null {
	const metas = cspMetas();
	return metas.length === 1 ? (metas[0]?.getAttribute("content") ?? "") : null;
}

export function directiveSources(
	policy: string,
	directive: string,
): string[] | null {
	for (const part of policy.split(";")) {
		const [name, ...sources] = part.trim().split(/\s+/);
		if (name === directive) return sources;
	}
	return null;
}

/** CSP3 host matching for `scheme://host` and `scheme://*.host`: a wildcard never covers its apex */
export function sourceCovers(source: string, origin: string): boolean {
	if (source === origin) return true;
	const separator = source.indexOf("://*.");
	if (separator < 0) return false;
	const scheme = source.slice(0, separator);
	const base = source.slice(separator + "://*.".length);
	return (
		origin.startsWith(`${scheme}://`) &&
		origin.slice(scheme.length + 3).endsWith(`.${base}`)
	);
}

export function originOf(url: string): string | null {
	try {
		const parsed = new URL(url);
		return parsed.protocol === "https:" || parsed.protocol === "wss:"
			? `${parsed.protocol}//${parsed.host}`
			: null;
	} catch {
		return null;
	}
}

/** Directives of the served policy that let `origin` through */
export function servedDirectivesFor(
	origin: string,
	directives: readonly string[],
): string[] {
	const policy = servedPolicy();
	if (policy === null) return [];
	return directives.filter((directive) =>
		(directiveSources(policy, directive) ?? []).some((source) =>
			sourceCovers(source, origin),
		),
	);
}
