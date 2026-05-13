import { createId } from "@paralleldrive/cuid2";
import type {
	IMetadata,
	IProfile,
	IProfileApp,
	ISettings,
	IUserState,
} from "@tm9657/flow-like-ui";
import { IAppVisibility } from "@tm9657/flow-like-ui";
import type {
	INotification,
	INotificationsOverview,
	IUserLookup,
} from "@tm9657/flow-like-ui/state/backend-state/types";
import type {
	IBillingSession,
	IPricingResponse,
	ISubscribeRequest,
	ISubscribeResponse,
	IUserInfo,
	IUserTemplateInfo,
	IUserUpdate,
	IUserWidgetInfo,
} from "@tm9657/flow-like-ui/state/backend-state/user-state";
import type { ISettingsProfile } from "@tm9657/flow-like-ui/types";
import { appsDB } from "../apps-db";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
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
		bits: apiProfile.bit_ids ?? [],
		hub: apiProfile.hub,
		hubs: apiProfile.hubs ?? undefined,
		created: apiProfile.created_at,
		updated: apiProfile.updated_at,
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

	private async mergeOfflineApps(profile: IProfile): Promise<IProfile> {
		if (typeof window === "undefined") return profile;

		try {
			// Get offline apps from local IndexedDB
			const visibilityRecords = await appsDB.visibility.toArray();
			const offlineAppIds = visibilityRecords
				.filter((v) => v.visibility === IAppVisibility.Offline)
				.map((v) => v.appId);

			if (offlineAppIds.length === 0) return profile;

			// Find apps that are marked as offline locally but not in the server profile
			const serverAppIds = new Set(profile.apps?.map((a) => a.app_id) || []);

			// Get local storage data for offline apps that need to be merged
			const localAppsKey = `flow-like-offline-apps-${profile.id}`;
			const localAppsJson = localStorage.getItem(localAppsKey);
			const localApps: IProfileApp[] = localAppsJson
				? JSON.parse(localAppsJson)
				: [];

			// Filter to only offline apps that aren't already in the server profile
			const missingOfflineApps = localApps.filter(
				(app) =>
					offlineAppIds.includes(app.app_id) && !serverAppIds.has(app.app_id),
			);

			if (missingOfflineApps.length === 0) return profile;

			// Merge offline apps back into the profile
			return {
				...profile,
				apps: [...(profile.apps || []), ...missingOfflineApps],
			};
		} catch {
			return profile;
		}
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
				if (savedApiProfile)
					return this.mergeOfflineApps(transformApiProfile(savedApiProfile));
			}

			// Fall back to first profile and save it
			const firstApiProfile = apiProfiles[0];
			if (typeof window !== "undefined" && firstApiProfile.id) {
				localStorage.setItem("flow-like-profile-id", firstApiProfile.id);
			}
			return this.mergeOfflineApps(transformApiProfile(firstApiProfile));
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

		return transformApiProfile(newApiProfile);
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
			if (response.signed_url.includes(".blob.core.windows.net")) {
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

		// Check if this app is offline
		const visibility = await appsDB.visibility.get(app.app_id);
		const isOffline = visibility?.visibility === IAppVisibility.Offline;

		// Get current apps from the profile
		let currentApps = profile.hub_profile.apps || [];

		if (operation === "Remove") {
			// Remove the app from the array
			currentApps = currentApps.filter((a) => a.app_id !== app.app_id);
			// Also remove from offline storage
			await this.removeFromOfflineStorage(profileId, app.app_id);
		} else {
			// Upsert: find existing app or add new one
			const existingIndex = currentApps.findIndex(
				(a) => a.app_id === app.app_id,
			);
			if (existingIndex >= 0) {
				currentApps[existingIndex] = app;
			} else {
				currentApps.push(app);
			}

			// Save to offline storage if app is offline
			if (isOffline) {
				await this.saveToOfflineStorage(profileId, app);
			}
		}

		// Only sync non-offline apps to server
		const appsToSync = currentApps.filter((a) => {
			const vis = a.app_id === app.app_id ? visibility : undefined;
			return vis?.visibility !== IAppVisibility.Offline;
		});

		// Update the profile with apps (excluding offline ones)
		await apiPost(
			`profile/${profileId}`,
			{ apps: appsToSync },
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
						typeof entry[2] === "object",
				)
				.map(([appId, widgetId, meta]) => ({
					appId,
					widgetId,
					metadata: {
						name: meta.name ?? "Unnamed Widget",
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
