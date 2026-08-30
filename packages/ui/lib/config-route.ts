/**
 * Config sections that own their vertical space instead of scrolling the page:
 * storage browsers and Data Studio render their own scroll containers, so the
 * layout must hand them a flex-sized slot rather than an auto-height one.
 *
 * Matched per path segment — a substring check on `/storage` silently misses
 * `/user-storage`, which collapses that page's height.
 */
const FULL_HEIGHT_SEGMENTS = new Set(["storage", "user-storage", "explore"]);

export function configRouteFillsHeight(route?: string | null): boolean {
	if (!route) return false;
	return route.split("/").some((segment) => FULL_HEIGHT_SEGMENTS.has(segment));
}
