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
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
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

const PROJECT_INSTALL_RETRY_MS = 5 * 60_000;

/** Native installs of one package share its staging paths, so they run one after another. */
const packageInstallQueues = new Map<string, Promise<unknown>>();
/** One install through a project's licence per `{appId}:{packageId}@{version}`, however many widgets ask. */
const projectInstalls = new Map<string, Promise<void>>();
/** When an install through a project last failed, per `{appId}:{packageId}@{version}`. */
const failedProjectInstalls = new Map<string, number>();

/** Runs `install` once every earlier install of the package settled; `waited` says whether one was pending. */
function queuePackageInstall<T>(
	packageId: string,
	install: (waited: boolean) => Promise<T>,
): Promise<T> {
	const previous = packageInstallQueues.get(packageId);
	const queued = (previous ?? Promise.resolve())
		.catch(() => undefined)
		.then(() => install(previous !== undefined));
	packageInstallQueues.set(packageId, queued);
	const release = () => {
		if (packageInstallQueues.get(packageId) === queued) {
			packageInstallQueues.delete(packageId);
		}
	};
	queued.then(release, release);
	return queued;
}

/** Rust: `Widget bundle {hash} of package '{pkg}' is not installed`. */
function isBundleNotInstalledError(error: unknown): boolean {
	return getErrorMessage(error, "").includes("is not installed");
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

	/** With `access` the caller decides the result, so a missing token or failed request throws instead of listing nothing. */
	async getOwnedPackages(filters?: SearchFilters): Promise<SearchResults> {
		const access = filters?.access;
		const hasProfile = Boolean(this.backend.profile && this.backend.auth);
		if (access && !(hasProfile && this.currentToken)) {
			throw new Error(`Sign in to list your packages (access=${access})`);
		}
		if (!hasProfile) {
			return { packages: [], totalCount: 0, offset: 0, limit: 20 };
		}
		try {
			return await this.fetchSearch({ ...filters, ownedOnly: true });
		} catch (error) {
			if (access) throw error;
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
		const ownedOnly = filters?.ownedOnly || filters?.access !== undefined;
		if (ownedOnly) params.set("owned_only", "true");
		if (!ownedOnly) params.set("include_own", "true");
		if (filters?.access) params.set("access", filters.access);
		if (filters?.ids) params.set("ids", filters.ids.join(","));
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

	/**
	 * The local install. With `appId` of an online project it is the version the
	 * project pins: a missing or other local version is replaced by the pinned
	 * one, installed through the project, and null stands for a failed install.
	 */
	async getPackage(
		packageId: string,
		appId?: string,
	): Promise<InstalledPackage | null> {
		const local = await this.getInstalledPackage(packageId);
		if (!appId || !(await this.canInstallThroughProject(appId))) return local;
		const pinned = await this.projectPin(appId, packageId);
		if (!pinned || local?.version === pinned) return local;
		try {
			await this.installThroughProject(packageId, appId, pinned);
		} catch (error) {
			console.warn(
				`[Registry] Could not install ${packageId}@${pinned} through project ${appId}:`,
				error,
			);
			return null;
		}
		const installed = await this.getInstalledPackage(packageId);
		return installed?.version === pinned ? installed : null;
	}

	private async getInstalledPackage(
		packageId: string,
	): Promise<InstalledPackage | null> {
		await this.ensureInit();
		return invoke("registry_get_package", { packageId });
	}

	private async canInstallThroughProject(appId: string): Promise<boolean> {
		if (!this.backend.profile || !this.backend.auth) return false;
		return !(await this.backend.isOffline(appId).catch(() => true));
	}

	/** `appState.listPackages` syncs an online project's local pins with the hub first. */
	private async projectPin(
		appId: string,
		packageId: string,
	): Promise<string | undefined> {
		try {
			const pin = (await this.backend.appState.listPackages?.(appId))?.[
				packageId
			];
			return typeof pin === "string" && pin ? pin : undefined;
		} catch (error) {
			console.warn(
				`[Registry] Could not read the package pins of project ${appId}:`,
				error,
			);
			return undefined;
		}
	}

	/**
	 * An install of the package that was pending and left `version` installed
	 * makes this one unnecessary. A failed install is not retried for
	 * `PROJECT_INSTALL_RETRY_MS`.
	 */
	private installThroughProject(
		packageId: string,
		appId: string,
		version: string,
	): Promise<void> {
		const key = `${appId}:${packageId}@${version}`;
		const pending = projectInstalls.get(key);
		if (pending) return pending;
		const failedAt = failedProjectInstalls.get(key);
		if (
			failedAt !== undefined &&
			Date.now() - failedAt < PROJECT_INSTALL_RETRY_MS
		) {
			return Promise.reject(
				new Error(
					`Installing ${packageId}@${version} through project ${appId} failed less than ${PROJECT_INSTALL_RETRY_MS / 60_000} minutes ago`,
				),
			);
		}
		const install = queuePackageInstall(packageId, async (waited) => {
			if (
				waited &&
				(await this.getInstalledPackage(packageId))?.version === version
			) {
				return;
			}
			await this.invokeInstall(packageId, version, appId);
		})
			.then(
				() => {
					failedProjectInstalls.delete(key);
				},
				(error: unknown) => {
					failedProjectInstalls.set(key, Date.now());
					throw error;
				},
			)
			.finally(() => {
				projectInstalls.delete(key);
			});
		projectInstalls.set(key, install);
		return install;
	}

	async installPackage(
		packageId: string,
		version?: string,
		_token?: string | null,
		appId?: string,
	): Promise<CachedPackage> {
		return queuePackageInstall(packageId, () =>
			this.invokeInstall(packageId, version, appId),
		);
	}

	private async invokeInstall(
		packageId: string,
		version?: string,
		appId?: string,
	): Promise<CachedPackage> {
		await this.ensureInit();
		return invoke("registry_install_package", {
			packageId,
			version,
			token: this.currentToken,
			appId,
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
			descriptor = await this.describeInstalledWidget(request, bundleHash);
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

	/**
	 * A widget of the version the online project pins, whose bundle this device
	 * lacks, is described once more after installing that version through the
	 * project. Any other version keeps the original error: installing it would
	 * replace the pinned one device-wide, and Reload in the builder moves a stale
	 * placement to the pin.
	 */
	private async describeInstalledWidget(
		request: WidgetPolicyRequest,
		bundleHash: string,
	): Promise<unknown> {
		const describe = () =>
			invoke<unknown>(
				"registry_describe_widget_policy",
				widgetPolicyArgs(request, bundleHash),
			);
		try {
			return await describe();
		} catch (error) {
			const appId = request.appId;
			if (
				!appId ||
				!isBundleNotInstalledError(error) ||
				!(await this.canInstallThroughProject(appId)) ||
				(await this.projectPin(appId, request.packageId)) !==
					request.packageVersion
			) {
				throw error;
			}
			try {
				await this.installThroughProject(
					request.packageId,
					appId,
					request.packageVersion,
				);
			} catch (installError) {
				console.warn(
					`[Registry] Could not install ${request.packageId}@${request.packageVersion} through project ${appId}:`,
					installError,
				);
				throw error;
			}
			return describe();
		}
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
