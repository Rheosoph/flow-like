import type {
	IApp,
	IAppCategory,
	IAppVisibility,
	IBoard,
	IMetadata,
} from "../../lib";
import type { IAppSearchSort } from "../../lib/schema/app/app-search-query";

export type IMediaItem = "icon" | "thumbnail" | "preview";

export interface IPurchaseResponse {
	checkoutUrl: string | null;
	alreadyMember: boolean;
	appId: string;
}

export interface IAppState {
	createApp(
		metadata: IMetadata,
		bits: string[],
		online: boolean,
		template?: IBoard,
	): Promise<IApp>;
	deleteApp(appId: string): Promise<void>;
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
	requestJoinApp(appId: string, comment?: string): Promise<void>;
	purchaseApp(appId: string): Promise<IPurchaseResponse>;
	listPackages?(appId: string): Promise<Record<string, string>>;
	addPackage?(appId: string, packageId: string, version: string): Promise<void>;
	removePackage?(appId: string, packageId: string): Promise<void>;
}
