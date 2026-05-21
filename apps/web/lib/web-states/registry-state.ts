import type { IRegistryState } from "@flow-like/flow-like-ui";
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
		await apiDelete(`registry/packages/${packageId}`, this.backend.auth);
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
}
