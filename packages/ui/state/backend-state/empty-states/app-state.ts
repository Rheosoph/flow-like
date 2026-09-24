import type {
	ExploreQuery,
	ExploreSearchQuery,
	ExploreSearchResponse,
	IApp,
	IAppCategory,
	IAppState,
	IAppVisibility,
	IBoard,
	IGroup,
	IMetadata,
	ResolvedExplore,
} from "@flow-like/flow-like-ui";
import type { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import type {
	IBeginOfflineForkBody,
	IBeginOfflineForkResponse,
	IForkPolicy,
	IForkPreviewResponse,
	IForkPreviewTarget,
	IForkSettings,
	IOnlineForkBody,
	IOnlineForkResponse,
} from "@flow-like/flow-like-ui/lib/schema/app/fork";
import type {
	AppCommentsResponse,
	IMediaItem,
	IPurchaseResponse,
	UpsertAppCommentRequest,
	UpsertAppCommentResponse,
} from "../app-state";

export class EmptyAppState implements IAppState {
	createApp(
		metadata: IMetadata,
		bits: string[],
		online: boolean,
		template?: IBoard,
	): Promise<IApp> {
		throw new Error("Method not implemented.");
	}
	deleteApp(appId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	leaveApp(appId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	searchApps(
		id?: string,
		query?: string,
		language?: string,
		category?: IAppCategory,
		author?: string,
		sort?: IAppSearchSort,
		tag?: string,
		offset?: number,
		limit?: number,
	): Promise<[IApp, IMetadata | undefined][]> {
		throw new Error("Method not implemented.");
	}
	getExplore(query: ExploreQuery): Promise<ResolvedExplore> {
		throw new Error("Method not implemented.");
	}
	searchExplore(query: ExploreSearchQuery): Promise<ExploreSearchResponse> {
		throw new Error("Method not implemented.");
	}
	getStoreGroups(offset?: number, limit?: number): Promise<IGroup[]> {
		throw new Error("Method not implemented.");
	}
	getStoreGroup(groupId: string): Promise<IGroup> {
		throw new Error("Method not implemented.");
	}
	getMyGroups(): Promise<IGroup[]> {
		throw new Error("Method not implemented.");
	}
	getApps(): Promise<[IApp, IMetadata | undefined][]> {
		throw new Error("Method not implemented.");
	}
	getApp(appId: string): Promise<IApp> {
		throw new Error("Method not implemented.");
	}
	getAppAuthoritative(appId: string): Promise<IApp> {
		throw new Error("Method not implemented.");
	}
	updateApp(app: IApp): Promise<void> {
		throw new Error("Method not implemented.");
	}
	updateAppAuthoritative(app: IApp): Promise<void> {
		throw new Error("Method not implemented.");
	}
	readAppBuild(appId: string, buildId: string): Promise<unknown | null> {
		throw new Error("Method not implemented.");
	}
	writeAppBuild(
		appId: string,
		buildId: string,
		record: unknown,
		expectedRevision: number | null,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getAppMeta(appId: string, language?: string): Promise<IMetadata> {
		throw new Error("Method not implemented.");
	}
	pushAppMeta(
		appId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	pushAppMedia(
		appId: string,
		item: IMediaItem,
		file: File,
		language?: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	changeAppVisibility(
		appId: string,
		visibility: IAppVisibility,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getAppStylesheet(appId: string): Promise<string | undefined> {
		throw new Error("Method not implemented.");
	}
	setAppStylesheet(appId: string, css: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	changeAppAllowForking(appId: string, allow: boolean): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getForkSettings(appId: string): Promise<IForkSettings> {
		throw new Error("Method not implemented.");
	}
	changeAppForkPolicy(appId: string, policy: IForkPolicy): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getForkPreview(
		appId: string,
		target: IForkPreviewTarget,
	): Promise<IForkPreviewResponse> {
		throw new Error("Method not implemented.");
	}
	beginOfflineFork(
		appId: string,
		body: IBeginOfflineForkBody,
	): Promise<IBeginOfflineForkResponse> {
		throw new Error("Method not implemented.");
	}
	onlineFork(
		appId: string,
		body: IOnlineForkBody,
	): Promise<IOnlineForkResponse> {
		throw new Error("Method not implemented.");
	}

	requestJoinApp(appId: string, comment?: string): Promise<void> {
		throw new Error("Method not implemented.");
	}

	purchaseApp(appId: string): Promise<IPurchaseResponse> {
		throw new Error("Method not implemented.");
	}
	getAppComments(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<AppCommentsResponse> {
		throw new Error("Method not implemented.");
	}
	upsertAppComment(
		appId: string,
		body: UpsertAppCommentRequest,
	): Promise<UpsertAppCommentResponse> {
		throw new Error("Method not implemented.");
	}
	deleteAppComment(appId: string, commentId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
