import {
	type WidgetGrantRequest,
	type WidgetGrantResponse,
	WidgetPolicyChangedError,
	type WidgetPolicyDescriptor,
	type WidgetPolicyRequest,
	WidgetRuntimeSourcesError,
	isDesktopWidgetGrant,
	isPolicyChangedError,
	parseWidgetGrantResponse,
	parseWidgetPolicyDescriptor,
	widgetRuntimeSourcesErrorCode,
} from "@flow-like/flow-like-ui/components/a2ui/micro-widget-policy";
import { forgetMicroWidgetGrants } from "@flow-like/flow-like-ui/components/a2ui/use-micro-widget-grant";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
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
} from "@flow-like/flow-like-ui/lib/schema/wasm";
import type { IRegistryState } from "@flow-like/flow-like-ui/state/backend-state/registry-state";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function requireBundleHash(request: WidgetPolicyRequest): string {
	if (!request.bundleHash) {
		throw new Error(
			`Widget ${request.packageId}/${request.widgetId} has no bundle hash, so its installed bundle cannot be resolved`,
		);
	}
	return request.bundleHash;
}

function widgetPolicyArgs(request: WidgetPolicyRequest, bundleHash: string) {
	const runtimeSources = request.runtimeSources ?? [];
	return {
		packageId: request.packageId,
		bundleHash,
		widgetId: request.widgetId,
		preview: request.preview,
		appId: request.appId ?? null,
		runtimeSources: runtimeSources.length > 0 ? runtimeSources : null,
	};
}

/** `invalid_runtime_sources: …` and `runtime_sources_in_preview: …` are host bugs, never user decisions. */
function runtimeSourcesError(
	error: unknown,
	request: WidgetPolicyRequest,
): WidgetRuntimeSourcesError | null {
	const code = widgetRuntimeSourcesErrorCode(error);
	return code
		? new WidgetRuntimeSourcesError(
				code,
				`The runtime sources sent for widget ${request.packageId}/${request.widgetId} were refused (${code})`,
			)
		: null;
}

export class RegistryState implements IRegistryState {
	private initPromise: Promise<void> | null = null;
	private initialized = false;

	constructor(private readonly backend: TauriBackend) {}

	async init(registryUrl?: string): Promise<void> {
		if (this.initialized) return;
		if (this.initPromise) return this.initPromise;
		this.initPromise = (async () => {
			try {
				const config = registryUrl ? { registry_url: registryUrl } : null;
				await invoke("registry_init", { config });
				this.initialized = true;
			} finally {
				this.initPromise = null;
			}
		})();
		return this.initPromise;
	}

	private async ensureInit(): Promise<void> {
		return this.init();
	}

	async searchPackages(filters?: SearchFilters): Promise<SearchResults> {
		if (!this.backend.profile || !this.backend.auth) {
			await this.ensureInit();
			return invoke("registry_search_packages", {
				filters: filters ?? {},
				token: this.currentToken,
			});
		}
		try {
			return await this.fetchSearch(filters);
		} catch {
			await this.ensureInit();
			return invoke("registry_search_packages", {
				filters: filters ?? {},
				token: this.currentToken,
			});
		}
	}

	async getOwnedPackages(filters?: SearchFilters): Promise<SearchResults> {
		if (!this.backend.profile || !this.backend.auth) {
			return { packages: [], totalCount: 0, offset: 0, limit: 20 };
		}
		try {
			return await this.fetchSearch({ ...filters, ownedOnly: true });
		} catch {
			return { packages: [], totalCount: 0, offset: 0, limit: 20 };
		}
	}

	private async fetchSearch(filters?: SearchFilters): Promise<SearchResults> {
		const params = new URLSearchParams();
		if (filters?.query) params.set("query", filters.query);
		if (filters?.category) params.set("category", filters.category);
		if (filters?.keywords?.length)
			params.set("keywords", filters.keywords.join(","));
		if (filters?.author) params.set("author", filters.author);
		if (filters?.verifiedOnly) params.set("verified_only", "true");
		if (filters?.includeDeprecated) params.set("include_deprecated", "true");
		if (filters?.includeDisabled) params.set("include_disabled", "true");
		if (filters?.sortBy) params.set("sort_by", filters.sortBy);
		if (filters?.sortDesc !== undefined)
			params.set("sort_desc", String(filters.sortDesc));
		if (filters?.offset) params.set("offset", String(filters.offset));
		if (filters?.limit) params.set("limit", String(filters.limit));
		if (filters?.language) params.set("language", filters.language);
		if (filters?.ownedOnly) params.set("owned_only", "true");
		if (!filters?.ownedOnly) params.set("include_own", "true");
		const qs = params.toString();
		const profile = this.backend.profile;
		if (!profile)
			throw new Error("Profile not set. Cannot search the registry.");
		const results = await fetcher<SearchResults>(
			profile,
			`registry/search${qs ? `?${qs}` : ""}`,
			{ method: "GET" },
			this.backend.auth,
		);
		if (!isRecord(results) || !Array.isArray(results.packages)) {
			throw new Error("registry/search returned no package list");
		}
		return results;
	}

	private get currentToken(): string | undefined {
		return this.backend.auth?.user?.access_token ?? undefined;
	}

	async getPackage(packageId: string): Promise<InstalledPackage | null> {
		await this.ensureInit();
		return invoke("registry_get_package", { packageId });
	}

	async installPackage(
		packageId: string,
		version?: string,
	): Promise<CachedPackage> {
		await this.ensureInit();
		return invoke("registry_install_package", {
			packageId,
			version,
			token: this.currentToken,
		});
	}

	async uninstallPackage(packageId: string): Promise<void> {
		try {
			await this.ensureInit();
			await invoke("registry_uninstall_package", { packageId });
		} finally {
			forgetMicroWidgetGrants(packageId);
		}
	}

	async getInstalledPackages(): Promise<InstalledPackage[]> {
		await this.ensureInit();
		return invoke("registry_get_installed_packages");
	}

	async isPackageInstalled(packageId: string): Promise<boolean> {
		await this.ensureInit();
		return invoke("registry_is_package_installed", { packageId });
	}

	async getInstalledVersion(packageId: string): Promise<string | null> {
		await this.ensureInit();
		return invoke("registry_get_installed_version", { packageId });
	}

	async updatePackage(
		packageId: string,
		version?: string,
	): Promise<CachedPackage> {
		await this.ensureInit();
		return invoke("registry_update_package", {
			packageId,
			version,
			token: this.currentToken,
		});
	}

	async checkForUpdates(): Promise<PackageUpdate[]> {
		await this.ensureInit();
		return invoke("registry_check_for_updates", { token: this.currentToken });
	}

	async purchasePackage(
		packageId: string,
		params?: WasmPurchaseParams,
	): Promise<WasmPurchaseResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("You must be logged in to purchase a package.");
		}
		return fetcher<WasmPurchaseResponse>(
			this.backend.profile,
			`registry/package/${packageId}/purchase`,
			{ method: "POST", body: JSON.stringify(params ?? {}) },
			this.backend.auth,
		);
	}

	async requestAccess(
		packageId: string,
		params?: RequestAccessParams,
	): Promise<RequestAccessResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("You must be logged in to request access.");
		}
		return fetcher<RequestAccessResponse>(
			this.backend.profile,
			`registry/package/${packageId}/access`,
			{ method: "PUT", body: JSON.stringify(params ?? {}) },
			this.backend.auth,
		);
	}

	async listAccessRequests(packageId: string): Promise<AccessRequest[]> {
		if (!this.backend.profile || !this.backend.auth) return [];
		return fetcher<AccessRequest[]>(
			this.backend.profile,
			`registry/package/${packageId}/access`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async acceptAccessRequest(
		packageId: string,
		requestId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("You must be logged in.");
		}
		await fetcher<void>(
			this.backend.profile,
			`registry/package/${packageId}/access/${requestId}`,
			{ method: "POST" },
			this.backend.auth,
		);
	}

	async rejectAccessRequest(
		packageId: string,
		requestId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("You must be logged in.");
		}
		await fetcher<void>(
			this.backend.profile,
			`registry/package/${packageId}/access/${requestId}`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}

	async setAuthToken(token: string | null): Promise<void> {
		return invoke("registry_set_auth_token", { token });
	}

	async describeWidgetPolicy(
		request: WidgetPolicyRequest,
	): Promise<WidgetPolicyDescriptor> {
		const bundleHash = requireBundleHash(request);
		await this.ensureInit();
		let descriptor: unknown;
		try {
			descriptor = await invoke<unknown>(
				"registry_describe_widget_policy",
				widgetPolicyArgs(request, bundleHash),
			);
		} catch (error) {
			throw runtimeSourcesError(error, request) ?? error;
		}
		return parseWidgetPolicyDescriptor(descriptor, {
			packageId: request.packageId,
			bundleHash,
			widgetId: request.widgetId,
			preview: request.preview,
		});
	}

	async mintWidgetGrant(
		request: WidgetGrantRequest,
	): Promise<WidgetGrantResponse> {
		const bundleHash = requireBundleHash(request);
		await this.ensureInit();
		let response: unknown;
		try {
			response = await invoke<unknown>("registry_mint_widget_grant", {
				...widgetPolicyArgs(request, bundleHash),
				policyDigest: request.policyDigest,
			});
		} catch (error) {
			if (isPolicyChangedError(error)) {
				throw new WidgetPolicyChangedError(
					`The permissions of widget ${request.packageId}/${request.widgetId} changed since they were approved`,
				);
			}
			throw runtimeSourcesError(error, request) ?? error;
		}
		return {
			...parseWidgetGrantResponse(response, isDesktopWidgetGrant),
			runtime: null,
		};
	}

	async revokeWidgetGrants(
		packageId: string,
		widgetId?: string,
	): Promise<void> {
		try {
			await this.ensureInit();
			await invoke("registry_revoke_widget_grants", {
				packageId,
				widgetId: widgetId ?? null,
			});
		} finally {
			forgetMicroWidgetGrants(packageId, widgetId);
		}
	}

	async getPackageComments(
		packageId: string,
		offset?: number,
		limit?: number,
	): Promise<PackageCommentsResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			return { comments: [], total: 0, offset: 0, limit: 20 };
		}
		const params = new URLSearchParams();
		if (offset != null) params.set("offset", String(offset));
		if (limit != null) params.set("limit", String(limit));
		const qs = params.toString();
		return fetcher<PackageCommentsResponse>(
			this.backend.profile,
			`registry/package/${packageId}/comments${qs ? `?${qs}` : ""}`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async upsertPackageComment(
		packageId: string,
		body: UpsertPackageCommentRequest,
	): Promise<UpsertPackageCommentResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("You must be logged in to leave a review.");
		}
		return fetcher<UpsertPackageCommentResponse>(
			this.backend.profile,
			`registry/package/${packageId}/comments`,
			{ method: "PUT", body: JSON.stringify(body) },
			this.backend.auth,
		);
	}

	async deletePackageComment(
		packageId: string,
		commentId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("You must be logged in.");
		}
		await fetcher<void>(
			this.backend.profile,
			`registry/package/${packageId}/comments/${commentId}`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}
}
