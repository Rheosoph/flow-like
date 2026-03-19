import type { IAppRouteState, IRouteMapping } from "../route-state";

export class EmptyRouteState implements IAppRouteState {
	getRoutes(_appId: string, _force?: boolean): Promise<IRouteMapping[]> {
		throw new Error("Method not implemented.");
	}
	getRouteByPath(_appId: string, _path: string): Promise<IRouteMapping | null> {
		throw new Error("Method not implemented.");
	}
	getDefaultRoute(_appId: string): Promise<IRouteMapping | null> {
		throw new Error("Method not implemented.");
	}
	setRoute(
		_appId: string,
		_path: string,
		_eventId: string,
	): Promise<IRouteMapping> {
		throw new Error("Method not implemented.");
	}
	setRoutes(
		_appId: string,
		_routes: Record<string, string>,
	): Promise<IRouteMapping[]> {
		throw new Error("Method not implemented.");
	}
	deleteRouteByPath(_appId: string, _path: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	deleteRouteByEvent(_appId: string, _eventId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
