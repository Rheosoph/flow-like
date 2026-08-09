import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import {
	EmptyAIState,
	EmptyApiKeyState,
	EmptyApiState,
	EmptyAppState,
	EmptyBitState,
	EmptyBoardState,
	EmptyDatabaseState,
	EmptyEventState,
	EmptyGraphState,
	EmptyHelperState,
	EmptyQueryState,
	EmptyRoleState,
	EmptyRouteState,
	EmptyStorageState,
	EmptyTeamState,
	EmptyTemplateState,
	EmptyUserState,
} from "@flow-like/flow-like-ui/state/backend-state/empty-states";

/**
 * Barrel-free stub backend for the showcase islands.
 *
 * IMPORTANT: import only leaf paths here, never "@flow-like/flow-like-ui".
 * The package barrel (index.ts) has a circular dependency that Next tolerates
 * but Vite's module-eval order trips at runtime ("Cannot access
 * 'RolePermissions' before initialization"), which kills island hydration.
 */
function unavailableState<T>(name: string): T {
	return new Proxy(
		{},
		{
			get: () => {
				throw new Error(`${name} is not available in the showcase backend`);
			},
		},
	) as T;
}

export class EmptyBackend implements IBackendState {
	aiState = new EmptyAIState();
	apiState = new EmptyApiState();
	apiKeyState = new EmptyApiKeyState();
	appState = new EmptyAppState();
	bitState = new EmptyBitState();
	boardState = new EmptyBoardState();
	eventState = new EmptyEventState();
	helperState = new EmptyHelperState();
	roleState = new EmptyRoleState();
	storageState = new EmptyStorageState();
	teamState = new EmptyTeamState();
	templateState = new EmptyTemplateState();
	userState = new EmptyUserState();
	dbState = new EmptyDatabaseState();
	queryState = new EmptyQueryState();
	graphState = new EmptyGraphState();
	widgetState: IBackendState["widgetState"] = unavailableState("WidgetState");
	pageState: IBackendState["pageState"] = unavailableState("PageState");
	routeState = new EmptyRouteState();
	registryState: IBackendState["registryState"] =
		unavailableState("RegistryState");

	capabilities(): ReturnType<IBackendState["capabilities"]> {
		return {
			needsSignIn: false,
			canHostLlamaCPP: false,
			canHostMLX: false,
			canHostEmbeddings: false,
			canExecuteLocally: false,
		};
	}

	async isOffline(_appId: string): Promise<boolean> {
		return false;
	}
}

/**
 * Single stub backend shared by every backend-consuming showcase island.
 * `useBackendStore` is a global zustand store, so all islands on a page read
 * the same backend. Per-surface overrides (runs/logs, model bits, …) are
 * layered on here as those variants land.
 */
export class ShowcaseBackend extends EmptyBackend {}

export const showcaseBackend = new ShowcaseBackend();
