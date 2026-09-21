import { normalizeRoutePath } from "./route-path";

const controls = /\p{Cc}/u;

export function isUsePathname(pathname: string | null): boolean {
	return pathname === "/use" || Boolean(pathname?.startsWith("/use/"));
}

function validateRoutePath(path: string): string {
	if (
		new TextEncoder().encode(path).length > 4096 ||
		controls.test(path) ||
		path.includes("\\") ||
		path.includes("?") ||
		path.includes("#")
	)
		throw new Error("The app route contains an invalid path.");
	const trimmed = path.trim();
	if (
		trimmed.startsWith("//") ||
		/^[a-z][a-z0-9+.-]*:/i.test(trimmed) ||
		trimmed.split("/").some((segment) => segment === "." || segment === "..")
	)
		throw new Error("Choose an internal path within this app.");
	return normalizeRoutePath(trimmed);
}

/** Decode a web app route once. Bare /use keeps its legacy Event-first behavior. */
export function readUseRoutePath(pathname: string): string | undefined {
	if (!pathname.startsWith("/use/")) return undefined;
	const segments = pathname.slice("/use/".length).split("/");
	const decoded = segments.map((segment) => {
		const value = decodeURIComponent(segment);
		if (value.includes("/"))
			throw new Error("An app route segment cannot contain an encoded slash.");
		return value;
	});
	return validateRoutePath(`/${decoded.join("/")}`);
}

function routePathname(route: string): string {
	return `/use${route.split("/").map(encodeURIComponent).join("/")}`;
}

/** Move only the shell's route parameter into the path, leaving app data intact. */
export function pathUseUrl(url: URL): string {
	const original = `${url.pathname}${url.search}${url.hash}`;
	if (!isUsePathname(url.pathname)) return original;
	const deepRoute = readUseRoutePath(url.pathname);
	const legacyRoute = url.searchParams.get("route");
	if (deepRoute === undefined && legacyRoute === null) return original;

	let pathname: string;
	try {
		pathname = routePathname(
			deepRoute ?? validateRoutePath(legacyRoute as string),
		);
	} catch (error) {
		if (deepRoute !== undefined) throw error;
		return original;
	}
	const next = new URL(url);
	next.pathname = pathname;
	if (next.searchParams.has("route")) next.searchParams.delete("route");
	return `${next.pathname}${next.search}${next.hash}`;
}
