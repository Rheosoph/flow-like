import type { IRegistryState } from "@flow-like/flow-like-ui";
import {
	type WidgetGrantRequest,
	type WidgetGrantResponse,
	WidgetPolicyChangedError,
	type WidgetPolicyDescriptor,
	type WidgetPolicyRequest,
	WidgetRuntimeSourcesError,
	isPolicyChangedError,
	isWebWidgetGrant,
	parseWidgetGrantResponse,
	parseWidgetPolicyDescriptor,
	widgetRuntimeSourcesErrorCode,
} from "@flow-like/flow-like-ui/components/a2ui/micro-widget-policy";
import { forgetMicroWidgetGrants } from "@flow-like/flow-like-ui/components/a2ui/use-micro-widget-grant";
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
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

function widgetPolicyPath(request: WidgetPolicyRequest): string {
	return `registry/package/${encodeURIComponent(
		request.packageId,
	)}/widget-policy/${encodeURIComponent(
		request.packageVersion,
	)}/${encodeURIComponent(request.widgetId)}`;
}

/** 400 `INVALID_RUNTIME_SOURCES` / `RUNTIME_SOURCES_IN_PREVIEW` are host bugs, never user decisions. */
function runtimeSourcesError(
	error: unknown,
	request: WidgetPolicyRequest,
): WidgetRuntimeSourcesError | null {
	const code = widgetRuntimeSourcesErrorCode(error);
	return code
		? new WidgetRuntimeSourcesError(
				code,
				`The runtime sources sent for widget ${request.packageId}@${request.packageVersion}/${request.widgetId} were refused (${code})`,
			)
		: null;
}

export class WebRegistryState implements IRegistryState {
	constructor(private readonly backend: WebBackendRef) {}

	async init(registryUrl?: string): Promise<void> {
		// In web mode, registry is managed server-side
	}

	async searchPackages(filters?: SearchFilters): Promise<SearchResults> {
		try {
			return await this.fetchSearch(filters);
		} catch {
			return { packages: [], totalCount: 0, offset: 0, limit: 20 };
		}
	}

	async getOwnedPackages(filters?: SearchFilters): Promise<SearchResults> {
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
		return apiGet<SearchResults>(
			`registry/search${qs ? `?${qs}` : ""}`,
			this.backend.auth,
		);
	}

	async getPackage(packageId: string): Promise<InstalledPackage | null> {
		try {
			return await apiGet<InstalledPackage>(
				`registry/packages/${packageId}`,
				this.backend.auth,
			);
		} catch {
			return null;
		}
	}

	async installPackage(
		packageId: string,
		version?: string,
	): Promise<CachedPackage> {
		const params = version ? `?version=${version}` : "";
		return apiPost<CachedPackage>(
			`registry/packages/${packageId}/install${params}`,
			undefined,
			this.backend.auth,
		);
	}

	async uninstallPackage(packageId: string): Promise<void> {
		try {
			await apiDelete(`registry/packages/${packageId}`, this.backend.auth);
		} finally {
			forgetMicroWidgetGrants(packageId);
		}
	}

	async getInstalledPackages(): Promise<InstalledPackage[]> {
		try {
			return await apiGet<InstalledPackage[]>(
				"registry/installed",
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async isPackageInstalled(packageId: string): Promise<boolean> {
		try {
			const result = await apiGet<{ installed: boolean }>(
				`registry/packages/${packageId}/installed`,
				this.backend.auth,
			);
			return result?.installed ?? false;
		} catch {
			return false;
		}
	}

	async getInstalledVersion(packageId: string): Promise<string | null> {
		try {
			const result = await apiGet<{ version: string | null }>(
				`registry/packages/${packageId}/version`,
				this.backend.auth,
			);
			return result?.version ?? null;
		} catch {
			return null;
		}
	}

	async updatePackage(
		packageId: string,
		version?: string,
	): Promise<CachedPackage> {
		const params = version ? `?version=${version}` : "";
		return apiPost<CachedPackage>(
			`registry/packages/${packageId}/update${params}`,
			undefined,
			this.backend.auth,
		);
	}

	async checkForUpdates(): Promise<PackageUpdate[]> {
		try {
			return await apiGet<PackageUpdate[]>(
				"registry/updates",
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async purchasePackage(
		packageId: string,
		params?: WasmPurchaseParams,
	): Promise<WasmPurchaseResponse> {
		return apiPost<WasmPurchaseResponse>(
			`registry/package/${packageId}/purchase`,
			params ?? {},
			this.backend.auth,
		);
	}

	async requestAccess(
		packageId: string,
		params?: RequestAccessParams,
	): Promise<RequestAccessResponse> {
		return apiPut<RequestAccessResponse>(
			`registry/package/${packageId}/access`,
			params ?? {},
			this.backend.auth,
		);
	}

	async listAccessRequests(packageId: string): Promise<AccessRequest[]> {
		try {
			return await apiGet<AccessRequest[]>(
				`registry/package/${packageId}/access`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async acceptAccessRequest(
		packageId: string,
		requestId: string,
	): Promise<void> {
		await apiPost(
			`registry/package/${packageId}/access/${requestId}`,
			undefined,
			this.backend.auth,
		);
	}

	async rejectAccessRequest(
		packageId: string,
		requestId: string,
	): Promise<void> {
		await apiDelete(
			`registry/package/${packageId}/access/${requestId}`,
			this.backend.auth,
		);
	}

	async getPackageComments(
		packageId: string,
		offset?: number,
		limit?: number,
	): Promise<PackageCommentsResponse> {
		const params = new URLSearchParams();
		if (offset != null) params.set("offset", String(offset));
		if (limit != null) params.set("limit", String(limit));
		const qs = params.toString();
		try {
			return await apiGet<PackageCommentsResponse>(
				`registry/package/${packageId}/comments${qs ? `?${qs}` : ""}`,
				this.backend.auth,
			);
		} catch {
			return { comments: [], total: 0, offset: 0, limit: 20 };
		}
	}

	async upsertPackageComment(
		packageId: string,
		body: UpsertPackageCommentRequest,
	): Promise<UpsertPackageCommentResponse> {
		return apiPut<UpsertPackageCommentResponse>(
			`registry/package/${packageId}/comments`,
			body,
			this.backend.auth,
		);
	}

	async deletePackageComment(
		packageId: string,
		commentId: string,
	): Promise<void> {
		await apiDelete(
			`registry/package/${packageId}/comments/${commentId}`,
			this.backend.auth,
		);
	}

	/**
	 * Declared-only descriptors come from the memoized `GET`; a request with
	 * runtime sources is described through `POST` on the same path. An API
	 * that predates runtime sources answers that `POST` with 404 or 405.
	 */
	async describeWidgetPolicy(
		request: WidgetPolicyRequest,
	): Promise<WidgetPolicyDescriptor> {
		const runtimeSources = request.runtimeSources ?? [];
		let descriptor: unknown;
		if (runtimeSources.length === 0) {
			descriptor = await apiGet<unknown>(
				`${widgetPolicyPath(request)}?preview=${request.preview}`,
				this.backend.auth,
			);
		} else {
			try {
				descriptor = await apiPost<unknown>(
					widgetPolicyPath(request),
					{
						preview: request.preview,
						...(request.appId ? { appId: request.appId } : {}),
						runtimeSources,
					},
					this.backend.auth,
				);
			} catch (error) {
				throw runtimeSourcesError(error, request) ?? error;
			}
		}
		return parseWidgetPolicyDescriptor(descriptor, {
			packageId: request.packageId,
			packageVersion: request.packageVersion,
			widgetId: request.widgetId,
			preview: request.preview,
		});
	}

	async mintWidgetGrant(
		request: WidgetGrantRequest,
	): Promise<WidgetGrantResponse> {
		let response: unknown;
		try {
			response = await apiPost<unknown>(
				`registry/package/${encodeURIComponent(request.packageId)}/widget-grant`,
				{
					version: request.packageVersion,
					widgetId: request.widgetId,
					preview: request.preview,
					policyDigest: request.policyDigest,
					...(request.appId ? { appId: request.appId } : {}),
					...(request.runtimeSources && request.runtimeSources.length > 0
						? { runtimeSources: request.runtimeSources }
						: {}),
				},
				this.backend.auth,
			);
		} catch (error) {
			if (isPolicyChangedError(error)) {
				throw new WidgetPolicyChangedError(
					`The permissions of widget ${request.packageId}@${request.packageVersion}/${request.widgetId} changed since they were approved`,
				);
			}
			throw runtimeSourcesError(error, request) ?? error;
		}
		return parseWidgetGrantResponse(response, isWebWidgetGrant);
	}
}
