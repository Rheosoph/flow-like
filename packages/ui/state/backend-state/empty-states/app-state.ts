import type {
	IApp,
	IAppCategory,
	IAppState,
	IAppVisibility,
	IBoard,
	IGroup,
	IMetadata,
} from "@flow-like/flow-like-ui";
import type { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import type {
	IBeginOfflineForkBody,
	IBeginOfflineForkResponse,
	IForkPreviewResponse,
	IForkPreviewTarget,
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
	updateApp(app: IApp): Promise<void> {
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
	changeAppAllowForking(appId: string, allow: boolean): Promise<void> {
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
