/**
 * Routes that own the whole window and draw their own navigation.
 *
 * The board editor is a VS Code-style shell with its own activity rail, so the
 * global sidebar's icon rail lands flush against it — two bordered icon columns
 * of comparable weight with nothing saying which scope each belongs to, and the
 * global `SidebarRail` hit strip covering the first 8px of every board rail
 * button. Such a route unmounts the global sidebar rather than collapsing it:
 * `setOpen` writes `sidebar_state` to localStorage unconditionally, so
 * auto-collapsing here would rewrite the user's preference for every other
 * route.
 *
 * Matched per path segment. `startsWith("/flow")` would also strip the chrome
 * from a future `/flow-templates`.
 */
const WINDOW_OWNING_ROUTES = new Set(["/flow"]);

export function ownsWindowChrome(route?: string | null): boolean {
	if (!route) return false;
	const path = route.split("?")[0].replace(/\/+$/, "") || "/";
	return WINDOW_OWNING_ROUTES.has(path);
}
