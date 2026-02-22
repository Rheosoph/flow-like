import type {
	AccessRequest,
	CachedPackage,
	InstalledPackage,
	PackageUpdate,
	RequestAccessParams,
	RequestAccessResponse,
	SearchFilters,
	SearchResults,
	WasmPurchaseParams,
	WasmPurchaseResponse,
} from "../../lib/schema/wasm";

export interface IRegistryState {
	init(registryUrl?: string): Promise<void>;
	searchPackages(filters?: SearchFilters): Promise<SearchResults>;
	getPackage(packageId: string): Promise<InstalledPackage | null>;
	installPackage(packageId: string, version?: string): Promise<CachedPackage>;
	uninstallPackage(packageId: string): Promise<void>;
	getInstalledPackages(): Promise<InstalledPackage[]>;
	isPackageInstalled(packageId: string): Promise<boolean>;
	getInstalledVersion(packageId: string): Promise<string | null>;
	updatePackage(packageId: string, version?: string): Promise<CachedPackage>;
	checkForUpdates(): Promise<PackageUpdate[]>;
	purchasePackage(packageId: string, params?: WasmPurchaseParams): Promise<WasmPurchaseResponse>;
	requestAccess(packageId: string, params?: RequestAccessParams): Promise<RequestAccessResponse>;
	listAccessRequests(packageId: string): Promise<AccessRequest[]>;
	acceptAccessRequest(packageId: string, requestId: string): Promise<void>;
	rejectAccessRequest(packageId: string, requestId: string): Promise<void>;
}
