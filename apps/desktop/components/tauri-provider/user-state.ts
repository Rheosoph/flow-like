import { invoke } from "@tauri-apps/api/core";
import type {
	IProfile,
	IProfileApp,
	ISettingsProfile,
	IUserState,
} from "@tm9657/flow-like-ui";
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
import { fetcher } from "../../lib/api";
import {
	type ILocalNotification,
	deleteLocalNotification,
	getLocalNotificationCounts,
	getLocalNotifications,
	markAllLocalNotificationsRead,
	markLocalNotificationRead,
} from "../../lib/notifications-db";
import type { TauriBackend } from "../tauri-provider";

function localToINotification(local: ILocalNotification): INotification {
	return {
		id: local.id,
		user_id: local.userId,
		app_id: local.appId,
		title: local.title,
		description: local.description,
		icon: local.icon,
		link: local.link,
		notification_type: local.notificationType,
		read: local.read,
		source_run_id: local.sourceRunId,
		source_node_id: local.sourceNodeId,
		created_at: local.createdAt,
		read_at: local.readAt,
	};
}

export class UserState implements IUserState {
	constructor(private readonly backend: TauriBackend) {}

	private getUserId(): string {
		// Use a constant for offline/unauthenticated users
		return this.backend.auth?.user?.profile?.sub ?? "offline-user";
	}

	async lookupUser(userId: string): Promise<IUserLookup> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<IUserLookup>(
			this.backend.profile,
			`user/lookup/${userId}`,
			{
				method: "GET",
			},
			this.backend.auth,
		);

		return result;
	}
	async searchUsers(query: string): Promise<IUserLookup[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<IUserLookup[]>(
			this.backend.profile,
			`user/search/${query}`,
			{
				method: "GET",
			},
			this.backend.auth,
		);

		return result;
	}
	async getNotifications(): Promise<INotificationsOverview> {
		const userId = this.getUserId();

		// Get local notifications first (works offline)
		let localCounts = { total: 0, unread: 0 };
		try {
			localCounts = await getLocalNotificationCounts(userId);
			const offlineCounts = await getLocalNotificationCounts("offline-user");
			localCounts.total = offlineCounts.total;
			localCounts.unread = offlineCounts.unread;
		} catch (e) {
			console.error(
				"[UserState.getNotifications] Error getting local counts:",
				e,
			);
		}

		// Try to get remote notifications if online
		if (this.backend.profile && this.backend.auth) {
			try {
				const remoteResult = await fetcher<INotificationsOverview>(
					this.backend.profile,
					`user/notifications`,
					{ method: "GET" },
					this.backend.auth,
				);

				return {
					invites_count: remoteResult.invites_count,
					notifications_count:
						(remoteResult.notifications_count ?? 0) + localCounts.total,
					unread_count: (remoteResult.unread_count ?? 0) + localCounts.unread,
				};
			} catch {
				// Fall back to local only on API error
			}
		}

		// Offline or API error: return local counts only
		return {
			invites_count: 0,
			notifications_count: localCounts.total,
			unread_count: localCounts.unread,
		};
	}

	async listNotifications(
		unreadOnly = false,
		offset = 0,
		limit = 20,
	): Promise<INotification[]> {
		const userId = this.getUserId();

		// Get local notifications first (works offline)
		let localNotifications: INotification[] = [];
		try {
			// Fetch more than needed for proper pagination when merged
			const local = await getLocalNotifications(
				userId,
				limit + offset,
				0,
				unreadOnly,
			);
			localNotifications = local.map(localToINotification);
		} catch (e) {
			console.error(
				"[UserState.listNotifications] Error getting local notifications:",
				e,
			);
		}

		// Try to get remote notifications if online
		let remoteResult: INotification[] = [];
		if (this.backend.profile && this.backend.auth) {
			try {
				const params = new URLSearchParams({
					limit: (limit + offset).toString(), // Fetch more for proper merge
					offset: "0",
					unread_only: unreadOnly.toString(),
				});

				remoteResult = await fetcher<INotification[]>(
					this.backend.profile,
					`user/notifications/list?${params}`,
					{ method: "GET" },
					this.backend.auth,
				);
			} catch {
				// Fall back to local only on API error
			}
		}

		// Merge and sort by createdAt descending
		const merged = [...remoteResult, ...localNotifications].sort(
			(a, b) =>
				new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
		);

		// Apply pagination to merged result
		return merged.slice(offset, offset + limit);
	}

	async markNotificationRead(notificationId: string): Promise<void> {
		// Try local first
		try {
			await markLocalNotificationRead(notificationId);
			return;
		} catch {
			// Not a local notification, try remote
		}

		// Only attempt remote if authenticated
		if (!this.backend.profile || !this.backend.auth) {
			return; // Silently succeed for offline mode
		}

		await fetcher(
			this.backend.profile,
			`user/notifications/${notificationId}`,
			{
				method: "POST",
			},
			this.backend.auth,
		);
	}

	async deleteNotification(notificationId: string): Promise<void> {
		// Try local first
		try {
			await deleteLocalNotification(notificationId);
			return;
		} catch {
			// Not a local notification, try remote
		}

		// Only attempt remote if authenticated
		if (!this.backend.profile || !this.backend.auth) {
			return; // Silently succeed for offline mode
		}

		await fetcher(
			this.backend.profile,
			`user/notifications/${notificationId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async markAllNotificationsRead(): Promise<number> {
		let remoteResult = 0;

		// Try remote if authenticated
		if (this.backend.profile && this.backend.auth) {
			try {
				remoteResult = await fetcher<number>(
					this.backend.profile,
					`user/notifications/read-all`,
					{
						method: "POST",
					},
					this.backend.auth,
				);
			} catch {
				// Ignore remote errors for offline support
			}
		}

		// Also mark all local notifications as read
		const userId = this.getUserId();
		let localCount = 0;
		try {
			localCount = await markAllLocalNotificationsRead(userId);
		} catch {
			// Ignore local errors
		}

		return remoteResult + localCount;
	}

	async getProfile(): Promise<IProfile> {
		const profile: ISettingsProfile = await invoke("get_current_profile");
		if (profile.hub_profile === undefined) {
			throw new Error("Profile not found");
		}
		return profile.hub_profile;
	}
	async getProfiles(): Promise<IProfile[]> {
		const profiles: ISettingsProfile[] = await invoke("get_profiles");
		return profiles
			.map((p) => p.hub_profile)
			.filter((p): p is IProfile => p !== undefined);
	}
	async getSettingsProfile(): Promise<ISettingsProfile> {
		const profile: ISettingsProfile = await invoke("get_current_profile");
		return profile;
	}
	async getAllSettingsProfiles(): Promise<ISettingsProfile[]> {
		const profiles: ISettingsProfile[] = await invoke("get_profiles");
		return profiles;
	}

	async updateUser(data: IUserUpdate, avatar?: File): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		if (avatar) {
			data.avatar_extension = avatar.name.split(".").pop() || "";
		}

		const response = await fetcher<{ signed_url?: string }>(
			this.backend.profile,
			`user/info`,
			{
				method: "PUT",
				body: JSON.stringify(data),
			},
			this.backend.auth,
		);

		if (response.signed_url && avatar) {
			await fetch(response.signed_url, {
				method: "PUT",
				body: avatar,
				headers: {
					"Content-Type": avatar.type,
				},
			});
		}
	}

	async getInfo(): Promise<IUserInfo> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<IUserInfo>(
			this.backend.profile,
			`user/info`,
			{
				method: "GET",
			},
			this.backend.auth,
		);

		return result;
	}

	async updateProfileApp(
		profile: ISettingsProfile,
		app: IProfileApp,
		operation: "Upsert" | "Remove",
	): Promise<void> {
		await invoke("profile_update_app", {
			profile,
			app,
			operation,
		});
	}

	async createPAT(
		name: string,
		validUntil?: Date,
		permissions?: number,
	): Promise<{ pat: string; permission: number }> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const unix = validUntil
			? Math.floor(validUntil.getTime() / 1000)
			: undefined;

		const result = await fetcher<{
			pat: string;
			permission: number;
		}>(
			this.backend.profile,
			`user/pat`,
			{
				method: "PUT",
				body: JSON.stringify({ name, valid_until: unix, permissions }),
			},
			this.backend.auth,
		);

		return result;
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
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<
			{
				id: string;
				name: string;
				created_at: string;
				valid_until: string | null;
				permission: number;
			}[]
		>(
			this.backend.profile,
			`user/pat`,
			{
				method: "GET",
			},
			this.backend.auth,
		);

		return result;
	}

	async deletePAT(id: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`user/pat/${id}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);

		return;
	}

	async getPricing(): Promise<IPricingResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<IPricingResponse>(
			this.backend.profile,
			"user/pricing",
			{ method: "GET" },
			this.backend.auth,
		);

		return result;
	}

	async createSubscription(
		request: ISubscribeRequest,
	): Promise<ISubscribeResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<ISubscribeResponse>(
			this.backend.profile,
			"user/subscribe",
			{
				method: "POST",
				body: JSON.stringify(request),
			},
			this.backend.auth,
		);

		return result;
	}

	async getBillingSession(): Promise<IBillingSession> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<IBillingSession>(
			this.backend.profile,
			"user/billing",
			{ method: "GET" },
			this.backend.auth,
		);

		return result;
	}

	async getUserWidgets(language?: string): Promise<IUserWidgetInfo[]> {
		const mergedWidgets = new Map<string, IUserWidgetInfo>();

		// First, get all local apps and their widgets
		try {
			const localApps = await invoke<[{ id: string }, any][]>("get_apps");
			for (const [app] of localApps) {
				try {
					const widgets = await invoke<{ id: string }[]>("get_widgets", {
						appId: app.id,
					});
					for (const widget of widgets) {
						try {
							const metadata = await invoke<any | null>("get_widget_meta", {
								appId: app.id,
								widgetId: widget.id,
								language,
							});
							const key = `${app.id}:${widget.id}`;
							mergedWidgets.set(key, {
								appId: app.id,
								widgetId: widget.id,
								metadata: {
									name: metadata?.name ?? widget.id,
									description: metadata?.description ?? "",
									thumbnail: metadata?.thumbnail ?? null,
									tags: metadata?.tags ?? [],
									icon: metadata?.icon ?? null,
									preview_media: metadata?.preview_media ?? [],
								},
							});
						} catch {
							// Widget meta not available, use defaults
							const key = `${app.id}:${widget.id}`;
							mergedWidgets.set(key, {
								appId: app.id,
								widgetId: widget.id,
								metadata: {
									name: widget.id,
									description: "",
									thumbnail: null,
									tags: [],
									icon: null,
									preview_media: [],
								},
							});
						}
					}
				} catch {
					// Failed to get widgets for this app, continue
				}
			}
		} catch (error) {
			console.warn("Failed to get local widgets:", error);
		}

		// If logged in, merge with remote widgets (remote takes precedence for metadata)
		if (this.backend.profile && this.backend.auth) {
			try {
				const queryParams = language
					? `?language=${encodeURIComponent(language)}`
					: "";
				const remoteWidgets = await fetcher<[string, string, any][]>(
					this.backend.profile,
					`user/widgets${queryParams}`,
					{ method: "GET" },
					this.backend.auth,
				);

				for (const [appId, widgetId, metadata] of remoteWidgets) {
					const key = `${appId}:${widgetId}`;
					mergedWidgets.set(key, {
						appId,
						widgetId,
						metadata: {
							name: metadata?.name ?? widgetId,
							description: metadata?.description ?? "",
							thumbnail: metadata?.thumbnail,
							tags: metadata?.tags ?? [],
							icon: metadata?.icon,
							preview_media: metadata?.preview_media ?? [],
						},
					});
				}
			} catch (error) {
				console.warn("Failed to get remote widgets:", error);
			}
		}

		return Array.from(mergedWidgets.values());
	}

	async getUserTemplates(language?: string): Promise<IUserTemplateInfo[]> {
		if (!this.backend.profile || !this.backend.auth) {
			return [];
		}

		const queryParams = language
			? `?language=${encodeURIComponent(language)}`
			: "";
		const result = await fetcher<[string, string, any][]>(
			this.backend.profile,
			`user/templates${queryParams}`,
			{ method: "GET" },
			this.backend.auth,
		);

		return result.map(([appId, templateId, metadata]) => ({
			appId,
			templateId,
			metadata: {
				name: metadata?.name ?? templateId,
				description: metadata?.description ?? "",
				thumbnail: metadata?.thumbnail,
				tags: metadata?.tags ?? [],
				icon: metadata?.icon,
				preview_media: metadata?.preview_media ?? [],
			},
		}));
	}
}
