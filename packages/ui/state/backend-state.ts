import { create } from "zustand";

import type { IProfile } from "../types";
import type { IAIState } from "./backend-state/ai-state";
import type { IAnalyticsState } from "./backend-state/analytics-state";
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
import type {
	IApplyFlowIrCommitResponse,
	IApplyFlowScriptResponse,
	IBoardMutationOptions,
	IBoardServerResetResult,
	IBoardState,
	IBoardSyncQueueEntry,
	IBoardSyncStatus,
	ICheckFlowScriptReconcileResponse,
	IFlowScriptDiagnostic,
	IScopedFlowScriptResponse,
} from "./backend-state/board-state";
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
	EmptyGraphState,
	EmptyHelperState,
	EmptyQueryState,
	EmptyRoleState,
	EmptyRouteState,
	EmptyStorageState,
	EmptyTeamState,
	EmptyTemplateState,
	EmptyUserState,
} from "./backend-state/empty-states";
import { EmptyUsageState } from "./backend-state/empty-states";
import type { IEventState } from "./backend-state/event-state";
import type { IGraphState } from "./backend-state/graph-state";
import type {
	IHelperState,
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
	ITemporaryUploadedFile,
} from "./backend-state/helper-state";
import type { IPageState } from "./backend-state/page-state";
import type { IQueryState } from "./backend-state/query-state";
import type { IRegistryState } from "./backend-state/registry-state";
import type { IRoleState } from "./backend-state/role-state";
import type { IAppRouteState } from "./backend-state/route-state";
import type { ISalesState } from "./backend-state/sales-state";
import type {
	IEventRegistration,
	ISinkState,
} from "./backend-state/sink-state";
import type { IStorageState } from "./backend-state/storage-state";

export type { IStorageUploadOptions } from "./backend-state/storage-state";
import type { ITeamState } from "./backend-state/team-state";
import type { ITemplateState } from "./backend-state/template-state";
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
	IBoardMutationOptions,
	IBoardServerResetResult,
	IBoardSyncQueueEntry,
	IBoardSyncStatus,
	IApplyFlowIrCommitResponse,
	IApplyFlowScriptResponse,
	ICheckFlowScriptReconcileResponse,
	IFlowScriptDiagnostic,
	IScopedFlowScriptResponse,
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
	IGraphState,
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
	ITemporaryUploadedFile,
};

export type { SinkType } from "./backend-state/sink-state";

export type {
	IEventRunsResult,
	IEventTimeline,
	IEventTimelineEntry,
	IEventTimelineRun,
} from "./backend-state/event-state";

export type {
	IGetPageOptions,
	IPageBootstrap,
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

export { applyWidgetRename } from "./backend-state/widget-state";

export type { IMediaItem } from "./backend-state/app-state";

export type {
	IAccessibleApp,
	IAppConnection,
	IAppConnectionStatus,
	IAppConnectionsResponse,
	IAppContentStats,
	IChangeGroupVisibilityResult,
	ICreateGroupPayload,
	IGroup,
	IGroupMember,
	IGroupMembershipRequest,
	IGroupPublicationLog,
	IGroupPublicationRequest,
	IGroupPublicationStatus,
	IMemberReadiness,
	IOwnRole,
	IUpdateGroupPayload,
	IBackendRole,
	IInvite,
	IInviteLink,
	IJoinRequest,
	IMember,
	IProcessCase,
	IProcessCaseDetailResponse,
	IProcessCaseRun,
	IProcessCasesResponse,
	IProcessFlow,
	IProcessGraphEdge,
	IProcessGraphNode,
	IProcessGraphResponse,
	IProcessNote,
	IRemoteEvent,
	IRemoteEventDetail,
	IRemoteMcpResource,
	IRemoteMcpTool,
	IRemoteRestFile,
	IRemoteRestRoute,
	IStorageItemActionResult,
	INotification,
	INotificationsOverview,
	INotificationEvent,
	NotificationType,
	IRuntimeVariable,
	IOAuthRequirement,
	IPrerunBoardResponse,
	IPrerunEventResponse,
	IUserLookup,
} from "./backend-state/types";
export * from "./backend-state/db-state";
export * from "./backend-state/graph-state";
export * from "./backend-state/query-state";
export type {
	IPushTargetStatus,
	IRegisterPushTargetRequest,
	IRegisterPushTargetResponse,
	IUserWidgetInfo,
	IUserTemplateInfo,
} from "./backend-state/user-state";

export interface ICapabilities {
	needsSignIn: boolean;
	canHostLlamaCPP: boolean;
	canHostMLX: boolean;
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
	graphState: IGraphState;
	queryState: IQueryState;
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
	/**
	 * True only when this device positively knows the app is local-only.
	 * `isOffline` also answers true for an app whose visibility has never been
	 * cached, so it cannot be used to rule out the server.
	 */
	isLocalOnly?(appId: string): Promise<boolean>;
}

interface BackendStoreState {
	backend: IBackendState | null;
	setBackend: (backend: IBackendState) => void;
}

export const useBackendStore = create<BackendStoreState>((set) => ({
	backend: null,
	setBackend: (backend: IBackendState) => set({ backend }),
}));

interface AuthStatusState {
	/** `undefined` until a host provider pushes its OIDC state. */
	signedIn?: boolean;
	setSignedIn: (signedIn: boolean) => void;
}

/**
 * Sign-in signal for components in this package, pushed by the host provider
 * (`pushAuthContext`) on every OIDC change. `packages/ui` has no auth context
 * of its own, so without it queries that only a signed-in session can serve
 * fire on every mount while signed out and fail (with retries).
 */
export const useAuthStatusStore = create<AuthStatusState>((set) => ({
	signedIn: undefined,
	setSignedIn: (signedIn: boolean) =>
		set((state) => (state.signedIn === signedIn ? state : { signedIn })),
}));

/** False only while a host positively reports a signed-out session. */
export function useSignedIn(): boolean {
	return useAuthStatusStore((state) => state.signedIn !== false);
}

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
	graphState: new EmptyGraphState(),
	queryState: new EmptyQueryState(),
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
		canHostMLX: false,
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

/**
 * False while `useBackend()` still hands out the prerender placeholder, whose
 * states throw on every call. Queries that mount before the host provider has
 * published its backend gate on this.
 */
export function useBackendReady(): boolean {
	return useBackendStore((state) => state.backend !== null);
}
