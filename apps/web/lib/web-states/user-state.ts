import type {
	IMetadata,
	IProfile,
	IProfileApp,
	IProfileShortcut,
	ISettings,
	IUserState,
} from "@flow-like/flow-like-ui";
import { IAppVisibility, isAzureBlobStorageUrl } from "@flow-like/flow-like-ui";
import type {
	INotification,
	INotificationsOverview,
	IUserLookup,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import type {
	IBillingSession,
	IPricingResponse,
	IPushTargetStatus,
	IRegisterPushTargetRequest,
	IRegisterPushTargetResponse,
	ISubscribeRequest,
	ISubscribeResponse,
	IUserInfo,
	IUserTemplateInfo,
	IUserUpdate,
	IUserWidgetInfo,
} from "@flow-like/flow-like-ui/state/backend-state/user-state";
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import { createId } from "@paralleldrive/cuid2";
import { appsDB, type IShortcut } from "../apps-db";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
} from "./api-utils";

// API returns snake_case fields, frontend expects camelCase
interface ApiProfile {
	id: string;
	name: string;
	thumbnail?: string | null;
	icon?: string | null;
	description?: string | null;
	interests?: string[] | null;
	tags?: string[] | null;
	theme?: unknown;
	settings?: unknown;
	apps?: unknown;
	shortcuts?: unknown;
	bit_ids?: string[] | null;
	hub: string;
	hubs?: string[] | null;
	user_id: string;
	created_at: string;
	updated_at: string;
	deleted_at?: string | null;
}

type UpsertProfileResponse = {
	profile: ApiProfile;
};

function transformApiProfile(apiProfile: ApiProfile): IProfile {
	return {
		id: apiProfile.id,
		name: apiProfile.name,
		thumbnail: apiProfile.thumbnail,
		icon: apiProfile.icon,
		description: apiProfile.description,
		interests: apiProfile.interests ?? undefined,
		tags: apiProfile.tags ?? undefined,
		theme: apiProfile.theme,
		settings: apiProfile.settings as ISettings | undefined,
		apps: apiProfile.apps as IProfileApp[] | undefined,
		shortcuts: apiProfile.shortcuts as IProfileShortcut[] | undefined,
		bits: apiProfile.bit_ids ?? [],
		hub: apiProfile.hub,
		hubs: apiProfile.hubs ?? undefined,
		created: apiProfile.created_at,
		updated: apiProfile.updated_at,
	};
}

function mergeProfileAppEntry(
	existing: IProfileApp | undefined,
	update: IProfileApp,
): IProfileApp {
	return {
		...existing,
		...update,
		favorite_order: update.favorite
			? (update.favorite_order ?? existing?.favorite_order ?? null)
			: (update.favorite_order ?? null),
		pinned_order: update.pinned
			? (update.pinned_order ?? existing?.pinned_order ?? null)
			: (update.pinned_order ?? null),
	};
}

function normalizeProfileShortcut(shortcut: IProfileShortcut): IShortcut {
	return {
		...shortcut,
		appId: shortcut.appId ?? undefined,
		icon: shortcut.icon ?? undefined,
	};
}

export class WebUserState implements IUserState {
	constructor(private readonly backend: WebBackendRef) {}

	private hasRemoteAccessToken(): boolean {
		return Boolean(
			this.backend.auth?.isAuthenticated &&
				this.backend.auth.user?.access_token,
		);
	}

	async lookupUser(userId: string): Promise<IUserLookup> {
		return apiGet<IUserLookup>(`user/lookup/${userId}`, this.backend.auth);
	}

	async searchUsers(query: string): Promise<IUserLookup[]> {
		try {
			return await apiGet<IUserLookup[]>(
				`user/search/${encodeURIComponent(query)}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getNotifications(): Promise<INotificationsOverview> {
		if (!this.hasRemoteAccessToken()) {
			return { unread_count: 0, invites_count: 0, notifications_count: 0 };
		}

		try {
			return await apiGet<INotificationsOverview>(
				"user/notifications",
				this.backend.auth,
			);
		} catch {
			return { unread_count: 0, invites_count: 0, notifications_count: 0 };
		}
	}

	async listNotifications(
		unreadOnly?: boolean,
		offset?: number,
		limit?: number,
	): Promise<INotification[]> {
		const params = new URLSearchParams();
		if (unreadOnly !== undefined) params.set("unread_only", String(unreadOnly));
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		if (!this.hasRemoteAccessToken()) {
			return [];
		}

		try {
			return await apiGet<INotification[]>(
				`user/notifications/list?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async markNotificationRead(notificationId: string): Promise<void> {
		await apiPost(
			`user/notifications/${notificationId}/read`,
			undefined,
			this.backend.auth,
		);
	}

	async deleteNotification(notificationId: string): Promise<void> {
		await apiDelete(`user/notifications/${notificationId}`, this.backend.auth);
	}

	async markAllNotificationsRead(): Promise<number> {
		const result = await apiPost<number | { count: number }>(
			"user/notifications/read-all",
			undefined,
			this.backend.auth,
		);
		if (typeof result === "number") {
			return result;
		}

		return result?.count ?? 0;
	}

	async registerPushTarget(
		request: IRegisterPushTargetRequest,
	): Promise<IRegisterPushTargetResponse> {
		return apiPost<IRegisterPushTargetResponse>(
			"user/push-targets/register",
			request,
			this.backend.auth,
		);
	}

	async getPushTargetStatus(deviceId: string): Promise<IPushTargetStatus> {
		return apiGet<IPushTargetStatus>(
			`user/push-targets/${encodeURIComponent(deviceId)}`,
			this.backend.auth,
		);
	}

	async setPushTargetEnabled(
		deviceId: string,
		enabled: boolean,
	): Promise<IPushTargetStatus> {
		return apiPatch<IPushTargetStatus>(
			`user/push-targets/${encodeURIComponent(deviceId)}`,
			{ push_enabled: enabled },
			this.backend.auth,
		);
	}

	private async mergeOfflineApps(profile: IProfile): Promise<IProfile> {
		if (typeof window === "undefined") return profile;

		try {
			const visibilityRecords = await appsDB.visibility.toArray();
			const offlineAppIds = new Set(
				visibilityRecords
					.filter((v) => v.visibility === IAppVisibility.Offline)
					.map((v) => v.appId),
			);

			if (offlineAppIds.size === 0) return profile;

			const localAppsKey = `flow-like-offline-apps-${profile.id}`;
			const localAppsJson = localStorage.getItem(localAppsKey);
			const localApps: IProfileApp[] = localAppsJson
				? JSON.parse(localAppsJson)
				: [];

			const localOfflineApps = localApps.filter((app) =>
				offlineAppIds.has(app.app_id),
			);

			if (localOfflineApps.length === 0) return profile;

			const appsById = new Map(
				(profile.apps ?? []).map((app) => [app.app_id, app]),
			);
			for (const localApp of localOfflineApps) {
				appsById.set(
					localApp.app_id,
					mergeProfileAppEntry(appsById.get(localApp.app_id), localApp),
				);
			}

			return {
				...profile,
				apps: Array.from(appsById.values()),
			};
		} catch {
			return profile;
		}
	}

	private async syncRemoteShortcuts(profile: IProfile): Promise<IProfile> {
		if (typeof window === "undefined" || !profile.id) return profile;
		if (!Array.isArray(profile.shortcuts)) return profile;

		try {
			const localShortcuts = await appsDB.shortcuts
				.where("profileId")
				.equals(profile.id)
				.toArray();
			const remoteShortcutIds = new Set(
				profile.shortcuts.map((shortcut) => shortcut.id),
			);

			await appsDB.transaction("rw", appsDB.shortcuts, async () => {
				for (const shortcut of profile.shortcuts ?? []) {
					await appsDB.shortcuts.put(normalizeProfileShortcut(shortcut));
				}
				for (const localShortcut of localShortcuts) {
					if (!remoteShortcutIds.has(localShortcut.id)) {
						await appsDB.shortcuts.delete(localShortcut.id);
					}
				}
			});
		} catch {
			return profile;
		}

		return profile;
	}

	private async prepareProfile(apiProfile: ApiProfile): Promise<IProfile> {
		const profile = await this.mergeOfflineApps(transformApiProfile(apiProfile));
		return this.syncRemoteShortcuts(profile);
	}

	async getProfile(): Promise<IProfile> {
		const allApiProfiles = await apiGet<ApiProfile[]>(
			"profile",
			this.backend.auth,
		);
		const apiProfiles = allApiProfiles?.filter((p) => !p.deleted_at) ?? [];

		if ((allApiProfiles?.length ?? 0) > 0 && apiProfiles.length === 0) {
			if (typeof window !== "undefined") {
				localStorage.removeItem("flow-like-profile-id");
			}
			throw new Error("No active profiles available");
		}

		if (apiProfiles.length > 0) {
			// Check localStorage for a preferred profile ID
			const savedProfileId =
				typeof window !== "undefined"
					? localStorage.getItem("flow-like-profile-id")
					: null;

			if (savedProfileId) {
				const savedApiProfile = apiProfiles.find(
					(p) => p.id === savedProfileId,
				);
				if (savedApiProfile) return this.prepareProfile(savedApiProfile);
			}

			// Fall back to first profile and save it
			const firstApiProfile = apiProfiles[0];
			if (typeof window !== "undefined" && firstApiProfile.id) {
				localStorage.setItem("flow-like-profile-id", firstApiProfile.id);
			}
			return this.prepareProfile(firstApiProfile);
		}

		// No profiles exist - create a default one using upsert endpoint
		const hubUrl =
			process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";
		const newProfileId = createId();

		const newProfileResponse = await apiPost<UpsertProfileResponse>(
			`profile/${newProfileId}`,
			{
				name: "Default Profile",
				description: "Your default profile",
				hub: hubUrl,
				hubs: [hubUrl],
			},
			this.backend.auth,
		);
		const newApiProfile = newProfileResponse.profile;

		if (typeof window !== "undefined" && newApiProfile.id) {
			localStorage.setItem("flow-like-profile-id", newApiProfile.id);
		}

		return this.prepareProfile(newApiProfile);
	}

	async getProfiles(): Promise<IProfile[]> {
		const allApiProfiles = await apiGet<ApiProfile[]>(
			"profile",
			this.backend.auth,
		);
		const apiProfiles = allApiProfiles?.filter((p) => !p.deleted_at) ?? [];
		if (apiProfiles.length === 0) return [];
		return apiProfiles.map(transformApiProfile);
	}

	async getAllSettingsProfiles(): Promise<ISettingsProfile[]> {
		const profiles = await this.getProfiles();
		console.log("getAllSettingsProfiles - profiles count:", profiles.length);
		return profiles.map((profile) => ({
			hub_profile: profile,
			execution_settings: {
				gpu_mode: false,
				max_context_size: 8192,
			},
			updated: profile.updated ?? new Date().toISOString(),
			created: profile.created ?? new Date().toISOString(),
		}));
	}

	async getSettingsProfile(): Promise<ISettingsProfile> {
		const profile = await this.getProfile();
		return {
			hub_profile: profile,
			execution_settings: {
				gpu_mode: false,
				max_context_size: 8192,
			},
			updated: profile.updated ?? new Date().toISOString(),
			created: profile.created ?? new Date().toISOString(),
		};
	}

	async updateUser(data: IUserUpdate, avatar?: File): Promise<void> {
		if (avatar) {
			data.avatar_extension = avatar.name.split(".").pop() || "";
		}

		const response = await apiPut<{ signed_url?: string }>(
			"user/info",
			data,
			this.backend.auth,
		);

		if (response?.signed_url && avatar) {
			const headers: HeadersInit = {
				"Content-Type": avatar.type,
			};

			// Azure Blob Storage requires x-ms-blob-type header
			if (isAzureBlobStorageUrl(response.signed_url)) {
				headers["x-ms-blob-type"] = "BlockBlob";
			}

			await fetch(response.signed_url, {
				method: "PUT",
				body: avatar,
				headers,
			});
		}
	}

	async updateProfileApp(
		profile: ISettingsProfile,
		app: IProfileApp,
		operation: "Upsert" | "Remove",
	): Promise<void> {
		const profileId = profile.hub_profile.id;
		if (!profileId) {
			throw new Error("Profile ID is required");
		}

		const visibilityRecords = await appsDB.visibility.toArray();
		const offlineAppIds = new Set(
			visibilityRecords
				.filter((v) => v.visibility === IAppVisibility.Offline)
				.map((v) => v.appId),
		);
		const isOffline = offlineAppIds.has(app.app_id);

		let currentApps = [...(profile.hub_profile.apps || [])];

		if (operation === "Remove") {
			currentApps = currentApps.filter((a) => a.app_id !== app.app_id);
			await this.removeFromOfflineStorage(profileId, app.app_id);
		} else {
			const existingIndex = currentApps.findIndex(
				(a) => a.app_id === app.app_id,
			);
			const mergedApp = mergeProfileAppEntry(
				existingIndex >= 0 ? currentApps[existingIndex] : undefined,
				app,
			);
			if (existingIndex >= 0) {
				currentApps[existingIndex] = mergedApp;
			} else {
				currentApps.push(mergedApp);
			}

			if (isOffline) {
				await this.saveToOfflineStorage(profileId, mergedApp);
			}
		}

		const appsToSync = currentApps.filter((a) => !offlineAppIds.has(a.app_id));
		profile.hub_profile.apps = currentApps;

		await apiPost(
			`profile/${profileId}`,
			{ apps: appsToSync },
			this.backend.auth,
		);
	}

	async updateProfileShortcuts(
		profile: ISettingsProfile,
		shortcuts: IProfileShortcut[],
	): Promise<void> {
		const profileId = profile.hub_profile.id;
		if (!profileId) {
			throw new Error("Profile ID is required");
		}

		await apiPost(
			`profile/${profileId}`,
			{ shortcuts },
			this.backend.auth,
		);
	}

	private async saveToOfflineStorage(
		profileId: string,
		app: IProfileApp,
	): Promise<void> {
		if (typeof window === "undefined") return;

		const localAppsKey = `flow-like-offline-apps-${profileId}`;
		const localAppsJson = localStorage.getItem(localAppsKey);
		const localApps: IProfileApp[] = localAppsJson
			? JSON.parse(localAppsJson)
			: [];

		const existingIndex = localApps.findIndex((a) => a.app_id === app.app_id);
		if (existingIndex >= 0) {
			localApps[existingIndex] = app;
		} else {
			localApps.push(app);
		}

		localStorage.setItem(localAppsKey, JSON.stringify(localApps));
	}

	private async removeFromOfflineStorage(
		profileId: string,
		appId: string,
	): Promise<void> {
		if (typeof window === "undefined") return;

		const localAppsKey = `flow-like-offline-apps-${profileId}`;
		const localAppsJson = localStorage.getItem(localAppsKey);
		if (!localAppsJson) return;

		const localApps: IProfileApp[] = JSON.parse(localAppsJson);
		const filtered = localApps.filter((a) => a.app_id !== appId);
		localStorage.setItem(localAppsKey, JSON.stringify(filtered));
	}

	async getInfo(): Promise<IUserInfo> {
		return apiGet<IUserInfo>("user/info", this.backend.auth);
	}

	async createPAT(
		name: string,
		validUntil?: Date,
		permissions?: number,
	): Promise<{ pat: string; permission: number }> {
		return apiPut<{ pat: string; permission: number }>(
			"user/pat",
			{
				name,
				valid_until: validUntil
					? Math.floor(validUntil.getTime() / 1000)
					: undefined,
				permissions,
			},
			this.backend.auth,
		);
	}

	async getPATs(): Promise<
		{
			id: string;
			name: string;
			created_at: string;
			valid_until: string | null;
			permission: number;
		}[]
	> {
		try {
			return await apiGet("user/pat", this.backend.auth);
		} catch {
			return [];
		}
	}

	async deletePAT(id: string): Promise<void> {
		await apiDelete(`user/pat/${id}`, this.backend.auth);
	}

	async getPricing(): Promise<IPricingResponse> {
		return apiGet<IPricingResponse>("user/pricing", this.backend.auth);
	}

	async createSubscription(
		request: ISubscribeRequest,
	): Promise<ISubscribeResponse> {
		return apiPost<ISubscribeResponse>(
			"user/subscribe",
			request,
			this.backend.auth,
		);
	}

	async getBillingSession(): Promise<IBillingSession> {
		return apiGet<IBillingSession>("user/billing", this.backend.auth);
	}

	async getUserWidgets(language?: string): Promise<IUserWidgetInfo[]> {
		const params = language ? `?language=${language}` : "";
		try {
			const response = await apiGet<[string, string, IMetadata][]>(
				`user/widgets${params}`,
				this.backend.auth,
			);
			if (!Array.isArray(response)) return [];
			return response
				.filter(
					(entry): entry is [string, string, IMetadata] =>
						Array.isArray(entry) &&
						entry.length >= 3 &&
						typeof entry[0] === "string" &&
						entry[0] !== "" &&
						typeof entry[1] === "string" &&
						entry[1] !== "" &&
						entry[2] !== null &&
						typeof entry[2] === "object" &&
						typeof entry[2].name === "string" &&
						entry[2].name.trim() !== "",
				)
				.map(([appId, widgetId, meta]) => ({
					appId,
					widgetId,
					metadata: {
						name: meta.name.trim(),
						description: meta.description ?? "",
						thumbnail: meta.thumbnail,
						tags: meta.tags ?? [],
						icon: meta.icon,
						preview_media: meta.preview_media,
					},
				}));
		} catch {
			return [];
		}
	}

	async getUserTemplates(language?: string): Promise<IUserTemplateInfo[]> {
		const params = language ? `?language=${language}` : "";
		try {
			return await apiGet<IUserTemplateInfo[]>(
				`user/templates${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}
}
