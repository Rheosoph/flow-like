import type {
	IApp,
	IAppCategory,
	IAppVisibility,
	IBoard,
	IMetadata,
} from "../../lib";
import type { IAppSearchSort } from "../../lib/schema/app/app-search-query";
import type {
	IBeginOfflineForkBody,
	IBeginOfflineForkResponse,
	IForkPolicy,
	IForkPreviewResponse,
	IForkPreviewTarget,
	IForkSettings,
	IOnlineForkBody,
	IOnlineForkResponse,
} from "../../lib/schema/app/fork";
import type { IGroup } from "./types";

export type IMediaItem = "icon" | "thumbnail" | "preview";

export interface IPurchaseResponse {
	checkoutUrl: string | null;
	alreadyMember: boolean;
	appId: string;
}

export interface AppCommentItem {
	id: string;
	text: string;
	rating: number;
	userId: string;
	userName?: string;
	userAvatar?: string;
	createdAt: string;
	updatedAt: string;
}

export interface AppCommentsResponse {
	comments: AppCommentItem[];
	total: number;
	offset: number;
	limit: number;
}

export interface UpsertAppCommentRequest {
	text: string;
	rating: number;
}

export interface UpsertAppCommentResponse {
	commentId: string;
}

export interface IAppState {
	createApp(
		metadata: IMetadata,
		bits: string[],
		online: boolean,
		template?: IBoard,
	): Promise<IApp>;
	deleteApp(appId: string): Promise<void>;
	/**
	 * Give up membership of an app someone else owns. The app itself survives —
	 * only this user's access to it ends — and the local copy goes with it, so
	 * nothing is left behind syncing against a project the hub will now refuse.
	 *
	 * An owner cannot leave: the hub refuses to remove a membership whose role
	 * carries the `Owner` bit. Check `roleState.getOwnRole` before offering it.
	 */
	leaveApp(appId: string): Promise<void>;
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
	): Promise<[IApp, IMetadata | undefined][]>;
	getStoreGroups(offset?: number, limit?: number): Promise<IGroup[]>;
	getStoreGroup(groupId: string): Promise<IGroup>;
	/** Suites across all apps the caller is a member of (for the library). */
	getMyGroups(): Promise<IGroup[]>;
	getApps(): Promise<[IApp, IMetadata | undefined][]>;
	getApp(appId: string): Promise<IApp>;
	updateApp(app: IApp): Promise<void>;
	getAppMeta(appId: string, language?: string): Promise<IMetadata>;
	pushAppMeta(
		appId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void>;
	pushAppMedia(
		appId: string,
		item: IMediaItem,
		file: File,
		language?: string,
	): Promise<void>;
	changeAppVisibility(appId: string, visibility: IAppVisibility): Promise<void>;
	/**
	 * Toggle the project-level Fork-an-app opt-in. Owner-only on the backend
	 * (PATCH /apps/{app_id}/settings/forking).
	 */
	changeAppAllowForking(appId: string, allow: boolean): Promise<void>;
	/**
	 * Read the opt-in plus the owner-defined policy. Owner-only on the backend
	 * (GET /apps/{app_id}/settings/forking).
	 */
	getForkSettings(appId: string): Promise<IForkSettings>;
	/**
	 * Replace the owner-defined policy describing what a fork of this app
	 * copies. Owner-only on the backend
	 * (PATCH /apps/{app_id}/settings/forking).
	 */
	changeAppForkPolicy(appId: string, policy: IForkPolicy): Promise<void>;
	/**
	 * Pre-fork dry run — returns size totals, remote-token requirements, and
	 * the permission verdict. Safe to call as a probe before committing to a
	 * full fork. (GET /apps/{app_id}/fork/preview)
	 */
	getForkPreview(
		appId: string,
		target: IForkPreviewTarget,
	): Promise<IForkPreviewResponse>;
	/**
	 * Materialize an offline-bundle fork on the server and return scoped
	 * read credentials + the bundle prefix so the desktop client can
	 * pull the bundle. (POST /apps/{app_id}/fork/offline/begin)
	 */
	beginOfflineFork(
		appId: string,
		body: IBeginOfflineForkBody,
	): Promise<IBeginOfflineForkResponse>;
	/**
	 * Create an online → online fork on the calling user's account.
	 * (POST /apps/{app_id}/fork)
	 */
	onlineFork(
		appId: string,
		body: IOnlineForkBody,
	): Promise<IOnlineForkResponse>;
	requestJoinApp(appId: string, comment?: string): Promise<void>;
	purchaseApp(appId: string): Promise<IPurchaseResponse>;
	getAppComments(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<AppCommentsResponse>;
	upsertAppComment(
		appId: string,
		body: UpsertAppCommentRequest,
	): Promise<UpsertAppCommentResponse>;
	deleteAppComment(appId: string, commentId: string): Promise<void>;
	/**
	 * Record what an app's visibility already is, for backends that keep a
	 * local visibility cache. Purely local bookkeeping — unlike
	 * {@link changeAppVisibility} it never asks the server to change anything.
	 *
	 * Callers that make an app appear on this device without opening it (fork,
	 * acquire) must call this. A device that has not learned an app's
	 * visibility falls back to guessing, and the desktop's guess is
	 * "offline" — which routes every board and data read at the local store
	 * and away from the hub the app actually lives on.
	 */
	recordLocalAppVisibility?(
		appId: string,
		visibility: IAppVisibility,
	): Promise<void>;
	listPackages?(appId: string): Promise<Record<string, string>>;
	addPackage?(appId: string, packageId: string, version: string): Promise<void>;
	removePackage?(appId: string, packageId: string): Promise<void>;
}
