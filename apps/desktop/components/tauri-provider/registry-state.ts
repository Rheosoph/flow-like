import type { IProfile } from "@flow-like/flow-like-ui";
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
	PackageInvitation,
	PackageSource,
	PackageUpdate,
	PackageUser,
	RegistryEntry,
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
import { isHubUnavailable } from "../../lib/api-error";
import { HUB_REFRESH_TIMEOUT_MS } from "../../lib/request-deadline";
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
const HUB_PIN_FRESH_MS = 30_000;
const HUB_PIN_RETRY_MS = 5_000;
/** The native client's registry for a profile without a hub. */
const OFFICIAL_REGISTRY_URL = "https://api.flow-like.com/api/v1/registry";

/** Native installs of one package share its staging paths, so they run one after another. */
const packageInstallQueues = new Map<string, Promise<unknown>>();
/** One install through a project's licence per `{appId}:{packageId}@{version}`, however many widgets ask. */
const projectInstalls = new Map<string, Promise<void>>();
/**
 * The last failed install through a project per `{appId}:{packageId}@{version}`,
 * with the session token it failed with and how long it is not tried again.
 */
const failedProjectInstalls = new Map<
	string,
	{ at: number; token: string; retryMs: number }
>();

interface HubPin {
	readonly token: string;
	readonly pin: Promise<string | undefined>;
	until: number;
}

/**
 * What the hub said a project pins per `{appId}:{packageId}`, asked once per
 * session token until `until`: `HUB_PIN_FRESH_MS` after the hub ruled, a
 * refusal included, `HUB_PIN_RETRY_MS` after it could not be reached.
 */
const hubPins = new Map<string, HubPin>();

/**
 * The registry the native client installs from and records on its copies: the
 * one of the profile's hub (`hub_registry_url` in `src-tauri`). Not
 * `getApiUrl`, whose `NEXT_PUBLIC_API_URL` override moves this app's requests
 * but not the native client.
 */
function profileRegistryUrl(profile: IProfile): string {
	const hub = (profile.hub ?? "").trim().replace(/\/+$/, "");
	if (!hub) return OFFICIAL_REGISTRY_URL;
	const origin = hub.includes("://")
		? hub
		: `${profile.secure === false ? "http" : "https"}://${hub}`;
	try {
		const url = new URL(origin);
		const base = url.pathname.replace(/\/+$/, "");
		url.pathname = `${base}${base.endsWith("/api/v1") ? "" : "/api/v1"}/registry`;
		return url.href;
	} catch {
		return OFFICIAL_REGISTRY_URL;
	}
}

function comparableUrl(url: string): string {
	try {
		return new URL(url).href.replace(/\/+$/, "");
	} catch {
		return url.trim().replace(/\/+$/, "");
	}
}

/**
 * Whether a registry install came from another registry than `registryUrl`,
 * which may publish other bytes under the same id and version. Developer
 * packages record no registry.
 */
function isFromAnotherRegistry(
	source: PackageSource | undefined,
	registryUrl: string,
): boolean {
	if (source?.type !== "remote") return false;
	const recorded = source.registry_url ?? source.registryUrl;
	return (
		typeof recorded === "string" &&
		recorded !== "" &&
		comparableUrl(recorded) !== comparableUrl(registryUrl)
	);
}

/** `version` as this device holds it: the active copy as it is, another installed one by its own record. */
function installedVersion(
	local: InstalledPackage | null,
	version: string,
): InstalledPackage | null {
	if (!local) return null;
	if (local.version === version) return local;
	const versions = local.versions;
	if (!versions || !Object.hasOwn(versions, version)) return null;
	const installed = versions[version];
	return {
		...local,
		...installed,
		version,
		metadata: installed.metadata ?? local.metadata,
	};
}

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

/**
 * Rust: `Failed to download package: 403 Forbidden: …`, the registry's 4xx
 * answer to an install, which the same session gets again. A transport error,
 * a timeout, a 5xx, a failing CDN or a registry that is throttling (429) or
 * timed out (408) is no ruling on the package, as for `isHubUnavailable`.
 */
function isRegistryRefusal(error: unknown): boolean {
	return /^(Failed to download package: )?4(?!08\b|29\b)\d\d\b/.test(
		getErrorMessage(error, ""),
	);
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

	/** The access token of a live session. `backend.auth` itself is also set while signed out, restoring or expired. */
	private get sessionToken(): string | undefined {
		const auth = this.backend.auth;
		if (!auth?.isAuthenticated || auth.user?.expired) return undefined;
		return auth.user?.access_token || undefined;
	}

	/**
	 * The local install. With `appId` of an online project it is the version the
	 * project pins. One this device holds answers from where it is, without
	 * becoming the active version. A missing one, or a copy another registry
	 * installed, is installed through the project, and null stands for a failed
	 * install.
	 */
	async getPackage(
		packageId: string,
		appId?: string,
	): Promise<InstalledPackage | null> {
		const local = await this.getInstalledPackage(packageId);
		if (!appId || !(await this.canInstallThroughProject(appId))) return local;
		const pinned = await this.projectPin(appId, packageId);
		if (!pinned) return local;
		const held = this.heldVersion(local, pinned);
		if (held) return held;
		try {
			await this.installThroughProject(packageId, appId, pinned);
		} catch (error) {
			console.warn(
				`[Registry] Could not install ${packageId}@${pinned} through project ${appId}:`,
				error,
			);
			return null;
		}
		// The native client decides which copy an install replaces, so what it left is the answer.
		return installedVersion(await this.getInstalledPackage(packageId), pinned);
	}

	private async getInstalledPackage(
		packageId: string,
	): Promise<InstalledPackage | null> {
		await this.ensureInit();
		return invoke("registry_get_package", { packageId });
	}

	/** `version` as this device holds it for the profile's hub: a copy from another registry does not count. */
	private heldVersion(
		local: InstalledPackage | null,
		version: string,
	): InstalledPackage | null {
		const profile = this.backend.profile;
		if (
			profile &&
			isFromAnotherRegistry(local?.source, profileRegistryUrl(profile))
		) {
			return null;
		}
		return installedVersion(local, version);
	}

	private async canInstallThroughProject(appId: string): Promise<boolean> {
		if (!this.backend.profile || !this.sessionToken) return false;
		return !(await this.backend.isOffline(appId).catch(() => true));
	}

	/**
	 * The version an online project pins: from its local pins, which
	 * `appState.listPackages` first syncs with the hub and answers to members
	 * who may list them, otherwise from the hub's read of the package through
	 * the project, which every member may make.
	 */
	private async projectPin(
		appId: string,
		packageId: string,
	): Promise<string | undefined> {
		return (
			(await this.listedPin(appId, packageId)) ?? this.hubPin(appId, packageId)
		);
	}

	/**
	 * Whether the online project pins `version`. The listed pins trail the hub
	 * by up to one sync, so the hub is asked about a version they do not name.
	 */
	private async isProjectPin(
		appId: string,
		packageId: string,
		version: string,
	): Promise<boolean> {
		if ((await this.listedPin(appId, packageId)) === version) return true;
		return (await this.hubPin(appId, packageId)) === version;
	}

	private async listedPin(
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

	/** One hub read per project, package and session token, however many widgets ask. */
	private hubPin(
		appId: string,
		packageId: string,
	): Promise<string | undefined> {
		const token = this.sessionToken;
		if (!token) return Promise.resolve(undefined);
		const key = `${appId}:${packageId}`;
		const known = hubPins.get(key);
		if (known && known.token === token && Date.now() < known.until) {
			return known.pin;
		}
		const asked: HubPin = {
			token,
			until: Number.POSITIVE_INFINITY,
			pin: this.readHubPin(appId, packageId).then(
				(pin) => {
					asked.until = Date.now() + HUB_PIN_FRESH_MS;
					return pin;
				},
				(error: unknown) => {
					asked.until =
						Date.now() +
						(isHubUnavailable(error) ? HUB_PIN_RETRY_MS : HUB_PIN_FRESH_MS);
					console.warn(
						`[Registry] Could not ask the hub which version of ${packageId} project ${appId} pins:`,
						error,
					);
					return undefined;
				},
			),
		};
		hubPins.set(key, asked);
		return asked.pin;
	}

	/** Only `pinnedVersion` is the pin: `manifest.version` of a hub that does not send it is the latest version. */
	private async readHubPin(
		appId: string,
		packageId: string,
	): Promise<string | undefined> {
		const { profile, auth } = this.backend;
		if (!profile) return undefined;
		const entry = await fetcher<RegistryEntry>(
			profile,
			`registry/package/${encodeURIComponent(packageId)}?app_id=${encodeURIComponent(appId)}`,
			{ method: "GET", timeoutMs: HUB_REFRESH_TIMEOUT_MS },
			auth,
		);
		const pin = isRecord(entry) ? entry.pinnedVersion : undefined;
		return typeof pin === "string" && pin ? pin : undefined;
	}

	/**
	 * An install of the package that was pending and left `version` on this
	 * device makes this one unnecessary. An install the registry refused is not
	 * retried for `PROJECT_INSTALL_RETRY_MS` with the session token it failed
	 * with, one that failed otherwise for `HUB_PIN_RETRY_MS`: a sign-in or a
	 * renewed token tries at once. Without a session nothing is tried and
	 * nothing is remembered.
	 */
	private installThroughProject(
		packageId: string,
		appId: string,
		version: string,
	): Promise<void> {
		const key = `${appId}:${packageId}@${version}`;
		const pending = projectInstalls.get(key);
		if (pending) return pending;
		const failed = failedProjectInstalls.get(key);
		if (
			failed &&
			failed.token === this.sessionToken &&
			Date.now() - failed.at < failed.retryMs
		) {
			return Promise.reject(
				new Error(
					`Installing ${packageId}@${version} through project ${appId} failed less than ${failed.retryMs / 1000} seconds ago`,
				),
			);
		}
		let token: string | undefined;
		const install = queuePackageInstall(packageId, async (waited) => {
			if (
				waited &&
				this.heldVersion(await this.getInstalledPackage(packageId), version)
			) {
				return;
			}
			token = this.sessionToken;
			if (!token) {
				throw new Error(
					`Installing ${packageId}@${version} through project ${appId} needs a signed-in session`,
				);
			}
			await this.invokeInstall(packageId, version, appId, token);
		})
			.then(
				() => {
					failedProjectInstalls.delete(key);
				},
				(error: unknown) => {
					if (token) {
						failedProjectInstalls.set(key, {
							at: Date.now(),
							token,
							retryMs: isRegistryRefusal(error)
								? PROJECT_INSTALL_RETRY_MS
								: HUB_PIN_RETRY_MS,
						});
					}
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
		token = this.currentToken,
	): Promise<CachedPackage> {
		await this.ensureInit();
		return invoke("registry_install_package", {
			packageId,
			version,
			token,
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

	async listMyInvitations(): Promise<PackageInvitation[]> {
		if (!this.backend.profile || !this.backend.auth?.user?.access_token) {
			throw new Error("Sign in to view package invitations.");
		}
		return fetcher<PackageInvitation[]>(
			this.backend.profile,
			"registry/invitations/me",
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async acceptInvitation(invitationId: string): Promise<PackageUser> {
		if (!this.backend.profile || !this.backend.auth?.user?.access_token) {
			throw new Error("Sign in to accept a package invitation.");
		}
		return fetcher<PackageUser>(
			this.backend.profile,
			`registry/invitation/${encodeURIComponent(invitationId)}/accept`,
			{ method: "POST" },
			this.backend.auth,
		);
	}

	async rejectInvitation(invitationId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth?.user?.access_token) {
			throw new Error("Sign in to decline a package invitation.");
		}
		await fetcher<void>(
			this.backend.profile,
			`registry/invitation/${encodeURIComponent(invitationId)}/reject`,
			{ method: "POST" },
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
	 * placement to the pin. So does a placement that names no version: nothing
	 * says which one it needs.
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
			const version = request.packageVersion;
			if (
				!appId ||
				!version ||
				!isBundleNotInstalledError(error) ||
				!(await this.canInstallThroughProject(appId)) ||
				!(await this.isProjectPin(appId, request.packageId, version))
			) {
				throw error;
			}
			try {
				await this.installThroughProject(request.packageId, appId, version);
			} catch (installError) {
				console.warn(
					`[Registry] Could not install ${request.packageId}@${version} through project ${appId}:`,
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
