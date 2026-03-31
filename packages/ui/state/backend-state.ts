import { create } from "zustand";

import type { IProfile } from "../types";
import type { IAIState } from "./backend-state/ai-state";
import type { IApiKeyState } from "./backend-state/api-key-state";
import type { IApiState } from "./backend-state/api-state";
import type {
	AppCommentItem,
	AppCommentsResponse,
	IAppState,
	IPurchaseResponse,
	UpsertAppCommentRequest,
	UpsertAppCommentResponse,
} from "./backend-state/app-state";
import type { IBitState } from "./backend-state/bit-state";
import type { IBoardState } from "./backend-state/board-state";
import type { IDatabaseState } from "./backend-state/db-state";
import {
	EmptyAIState,
	EmptyApiKeyState,
	EmptyApiState,
	EmptyAppState,
	EmptyBitState,
	EmptyBoardState,
	EmptyDatabaseState,
	EmptyEventState,
	EmptyHelperState,
	EmptyRoleState,
	EmptyRouteState,
	EmptyStorageState,
	EmptyTeamState,
	EmptyTemplateState,
	EmptyUserState,
} from "./backend-state/empty-states";
import { EmptyUsageState } from "./backend-state/empty-states";
import type { IEventState } from "./backend-state/event-state";
import type { IHelperState } from "./backend-state/helper-state";
import type { IPageState } from "./backend-state/page-state";
import type { IRegistryState } from "./backend-state/registry-state";
import type { IRoleState } from "./backend-state/role-state";
import type { IAppRouteState } from "./backend-state/route-state";
import type { ISalesState } from "./backend-state/sales-state";
import type {
	IEventRegistration,
	ISinkState,
} from "./backend-state/sink-state";
import type { IStorageState } from "./backend-state/storage-state";
import type { ITeamState } from "./backend-state/team-state";
import type { ITemplateState } from "./backend-state/template-state";
import type { IAnalyticsState } from "./backend-state/analytics-state";
import type { IUsageState } from "./backend-state/usage-state";
import type { IUserState } from "./backend-state/user-state";
import type { IWidgetState } from "./backend-state/widget-state";

export * from "./backend-state/api-key-state";
export * from "./backend-state/api-key-state";
export * from "./backend-state/api-state";
export * from "./backend-state/empty-states/index";
export * from "./backend-state/registry-state";
export * from "./backend-state/idb-route-state";
export * from "./backend-state/sales-state";
export type {
	IAIState,
	IApiKeyState,
	IApiState,
	AppCommentItem,
	AppCommentsResponse,
	IAppState,
	IPurchaseResponse,
	UpsertAppCommentRequest,
	UpsertAppCommentResponse,
	IAppRouteState,
	IBitState,
	IBoardState,
	IEventState,
	IHelperState,
	IPageState,
	IRegistryState,
	IRoleState,
	ISinkState,
	IEventRegistration,
	IStorageState,
	ITeamState,
	ITemplateState,
	IUserState,
	IWidgetState,
	IUsageState,
	IAnalyticsState,
};

export type { SinkType } from "./backend-state/sink-state";

export type {
	IPage,
	IWidgetRef,
	PageContent,
	PageLayoutType,
	PageMeta,
	PageListItem,
	CanvasSettings,
	WidgetInstance,
} from "./backend-state/page-state";

export type { IRouteMapping } from "./backend-state/route-state";

export type {
	CustomizationOption,
	CustomizationType,
	IWidget,
	ValidationRule,
	Version,
	VersionType,
} from "./backend-state/widget-state";

export type {
	IBackendRole,
	IInvite,
	IInviteLink,
	IJoinRequest,
	IMember,
	IStorageItemActionResult,
	INotification,
	INotificationsOverview,
	INotificationEvent,
	NotificationType,
	IRuntimeVariable,
	IOAuthRequirement,
	IPrerunBoardResponse,
	IPrerunEventResponse,
} from "./backend-state/types";
export * from "./backend-state/db-state";
export type {
	IUserWidgetInfo,
	IUserTemplateInfo,
} from "./backend-state/user-state";

export interface ICapabilities {
	needsSignIn: boolean;
	canHostLlamaCPP: boolean;
	canHostEmbeddings: boolean;
	canExecuteLocally: boolean;
}

export interface IBackendState {
	appState: IAppState;
	apiState: IApiState;
	apiKeyState: IApiKeyState;
	bitState: IBitState;
	boardState: IBoardState;
	userState: IUserState;
	teamState: ITeamState;
	roleState: IRoleState;
	storageState: IStorageState;
	templateState: ITemplateState;
	helperState: IHelperState;
	eventState: IEventState;
	aiState: IAIState;
	dbState: IDatabaseState;
	widgetState: IWidgetState;
	pageState: IPageState;
	routeState: IAppRouteState;
	registryState: IRegistryState;
	/** Sink state for managing active event sinks (desktop only) */
	sinkState?: ISinkState;
	/** Sales state for managing app sales and discounts (online apps only) */
	salesState?: ISalesState;
	/** Usage tracking state for LLM, embedding, and execution usage history */
	usageState?: IUsageState;
	/** Analytics state for creator dashboard metrics and feedback */
	analyticsState?: IAnalyticsState;

	/** Optional runtime profile (desktop/mobile providers populate this). */
	profile?: IProfile;

	capabilities(): ICapabilities;
	isOffline(appId: string): Promise<boolean>;
}

interface BackendStoreState {
	backend: IBackendState | null;
	setBackend: (backend: IBackendState) => void;
}

export const useBackendStore = create<BackendStoreState>((set) => ({
	backend: null,
	setBackend: (backend: IBackendState) => set({ backend }),
}));

const serverBackend: IBackendState = {
	appState: new EmptyAppState(),
	apiState: new EmptyApiState(),
	apiKeyState: new EmptyApiKeyState(),
	bitState: new EmptyBitState(),
	boardState: new EmptyBoardState(),
	userState: new EmptyUserState(),
	teamState: new EmptyTeamState(),
	roleState: new EmptyRoleState(),
	storageState: new EmptyStorageState(),
	templateState: new EmptyTemplateState(),
	helperState: new EmptyHelperState(),
	eventState: new EmptyEventState(),
	aiState: new EmptyAIState(),
	dbState: new EmptyDatabaseState(),
	widgetState: new Proxy(
		{},
		{
			get: () => {
				throw new Error("WidgetState is not available during prerender");
			},
		},
	) as IWidgetState,
	pageState: new Proxy(
		{},
		{
			get: () => {
				throw new Error("PageState is not available during prerender");
			},
		},
	) as IPageState,
	routeState: new EmptyRouteState(),
	registryState: new Proxy(
		{},
		{
			get: () => {
				throw new Error("RegistryState is not available during prerender");
			},
		},
	) as IRegistryState,
	usageState: new EmptyUsageState(),
	capabilities: () => ({
		needsSignIn: false,
		canHostLlamaCPP: false,
		canHostEmbeddings: false,
		canExecuteLocally: false,
	}),
	isOffline: async () => true,
};

export function useBackend(): IBackendState {
	const backend = useBackendStore((state) => state.backend);
	if (!backend) {
		return serverBackend;
	}
	return backend;
}
