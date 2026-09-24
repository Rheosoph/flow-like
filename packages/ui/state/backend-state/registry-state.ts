import type {
	WidgetGrantRequest,
	WidgetGrantResponse,
	WidgetPolicyDescriptor,
	WidgetPolicyRequest,
} from "../../components/a2ui/micro-widget-policy";
import type {
	AccessRequest,
	CachedPackage,
	InstalledPackage,
	PackageCommentsResponse,
	PackageUpdate,
	RequestAccessParams,
	RequestAccessResponse,
	SearchFilters,
	SearchResults,
	UpsertPackageCommentRequest,
	UpsertPackageCommentResponse,
	WasmPurchaseParams,
	WasmPurchaseResponse,
} from "../../lib/schema/wasm";

export interface IRegistryState {
	init(registryUrl?: string): Promise<void>;
	searchPackages(filters?: SearchFilters): Promise<SearchResults>;
	getOwnedPackages(filters?: SearchFilters): Promise<SearchResults>;
	getPackage(packageId: string): Promise<InstalledPackage | null>;
	/**
	 * `appId` downloads through that project's licence, so members can install
	 * a paid package an admin or the owner holds for the project.
	 */
	installPackage(
		packageId: string,
		version?: string,
		token?: string | null,
		appId?: string,
	): Promise<CachedPackage>;
	uninstallPackage(packageId: string): Promise<void>;
	getInstalledPackages(): Promise<InstalledPackage[]>;
	isPackageInstalled(packageId: string): Promise<boolean>;
	getInstalledVersion(packageId: string): Promise<string | null>;
	updatePackage(
		packageId: string,
		version?: string,
		token?: string | null,
	): Promise<CachedPackage>;
	checkForUpdates(token?: string | null): Promise<PackageUpdate[]>;
	purchasePackage(
		packageId: string,
		params?: WasmPurchaseParams,
	): Promise<WasmPurchaseResponse>;
	requestAccess(
		packageId: string,
		params?: RequestAccessParams,
	): Promise<RequestAccessResponse>;
	listAccessRequests(packageId: string): Promise<AccessRequest[]>;
	acceptAccessRequest(packageId: string, requestId: string): Promise<void>;
	rejectAccessRequest(packageId: string, requestId: string): Promise<void>;
	setAuthToken?(token: string | null): Promise<void>;
	getPackageComments(
		packageId: string,
		offset?: number,
		limit?: number,
	): Promise<PackageCommentsResponse>;
	upsertPackageComment(
		packageId: string,
		body: UpsertPackageCommentRequest,
	): Promise<UpsertPackageCommentResponse>;
	deletePackageComment(packageId: string, commentId: string): Promise<void>;
	/**
	 * Authoritative policy of a package widget. Absent on backends that predate
	 * widget grants. With `runtimeSources` the descriptor also covers addresses
	 * the host extracted from the widget's props; a refused request shape
	 * rejects with a `WidgetRuntimeSourcesError`.
	 */
	describeWidgetPolicy?(
		request: WidgetPolicyRequest,
	): Promise<WidgetPolicyDescriptor>;
	/**
	 * Mint a grant for the approved `policyDigest`, with the same
	 * `runtimeSources` the approved descriptor was described with. Rejects with
	 * an error that `isPolicyChangedError` recognizes when the backend now
	 * derives another policy. Web grants with runtime sources carry `runtime`.
	 */
	mintWidgetGrant?(request: WidgetGrantRequest): Promise<WidgetGrantResponse>;
	/** Drop issued grants so remounts run at baseline. Desktop only; web tokens expire. */
	revokeWidgetGrants?(packageId: string, widgetId?: string): Promise<void>;
}
