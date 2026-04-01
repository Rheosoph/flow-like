/**
 * Simple route mapping: path -> eventId
 * The event determines what to display (page-target or board-target)
 */
export interface IRouteMapping {
	path: string;
	eventId: string;
}

export interface IAppRouteState {
	/** Get all route mappings for an app */
	getRoutes(appId: string, force?: boolean): Promise<IRouteMapping[]>;
	/** Get the route mapping for a specific path */
	getRouteByPath(appId: string, path: string): Promise<IRouteMapping | null>;
	/** Get the default route (path = "/") */
	getDefaultRoute(appId: string): Promise<IRouteMapping | null>;
	/** Set a route mapping (path -> eventId) */
	setRoute(
		appId: string,
		path: string,
		eventId: string,
	): Promise<IRouteMapping>;
	/** Set all route mappings in bulk */
	setRoutes(
		appId: string,
		routes: Record<string, string>,
	): Promise<IRouteMapping[]>;
	/** Delete a route mapping by path */
	deleteRouteByPath(appId: string, path: string): Promise<void>;
	/** Delete a route mapping by event ID */
	deleteRouteByEvent(appId: string, eventId: string): Promise<void>;
}
