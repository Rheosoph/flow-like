/**
 * Config sections that own their vertical space instead of scrolling the page:
 * storage browsers, Data Studio, the app stylesheet editor and the Devices area
 * render their own scroll containers, so the layout must hand them a
 * flex-sized slot rather than an auto-height one — a code editor sized `h-full`
 * inside an auto-height slot collapses to nothing.
 *
 * Matched per path segment — a substring check on `/storage` silently misses
 * `/user-storage`, which collapses that page's height.
 */
const FULL_HEIGHT_SEGMENTS = new Set([
	"storage",
	"user-storage",
	"explore",
	"setup",
	"appearance",
	"devices",
]);

export function configRouteFillsHeight(route?: string | null): boolean {
	if (!route) return false;
	return route.split("/").some((segment) => FULL_HEIGHT_SEGMENTS.has(segment));
}

/**
 * The deploy wizard shows its step next to the "This deploy" summary, which
 * needs the whole card: the Devices section opens maximized while it runs.
 */
export function configRouteOpensMaximized(
	route?: string | null,
	flow?: string | null,
): boolean {
	return flow === "deploy" && !!route?.split("/").includes("devices");
}
