const STORE_ROOT = "/store";
const PACKAGES_PATH = "/store/packages";
const PROBE_ORIGIN = "https://flow-like.invalid";
const CONTROL_CHARACTER = /\p{Cc}/u;

/** A same-origin `/store` path (pathname + search) to return to, or null for anything else. */
export function safeStoreReturnPath(
	from: string | null | undefined,
): string | null {
	if (
		!from?.startsWith("/") ||
		from.startsWith("//") ||
		from.includes("\\") ||
		CONTROL_CHARACTER.test(from)
	) {
		return null;
	}
	let url: URL;
	try {
		url = new URL(from, PROBE_ORIGIN);
	} catch {
		return null;
	}
	if (url.origin !== PROBE_ORIGIN) return null;
	if (url.pathname !== STORE_ROOT && !url.pathname.startsWith(`${STORE_ROOT}/`))
		return null;
	return url.pathname + url.search;
}

export function packageStoreHref({
	id,
	tab,
	from,
}: {
	id: string;
	tab?: string | null;
	from?: string | null;
}): string {
	const params = new URLSearchParams({ id });
	if (tab) params.set("tab", tab);
	const back = safeStoreReturnPath(from);
	if (back) params.set("from", back);
	return `${PACKAGES_PATH}?${params.toString()}`;
}

export function getPackageOverviewHref(
	searchParams: Pick<URLSearchParams, "toString">,
): string {
	const params = new URLSearchParams(searchParams.toString());
	const back = safeStoreReturnPath(params.get("from"));
	if (back) return back;
	params.delete("id");
	params.delete("purchase");
	params.delete("from");
	const query = params.toString();
	return query ? `${PACKAGES_PATH}?${query}` : PACKAGES_PATH;
}
