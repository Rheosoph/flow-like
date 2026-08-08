import type {
	IMetadata,
	IProfile,
	IProfileApp,
	IProfileShortcut,
	ISettingsProfile,
	IUserState,
} from "@flow-like/flow-like-ui";
import type {
	INotification,
	INotificationsOverview,
	IUserLookup,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import {
	type IBillingSession,
	type IPushTargetStatus,
	type IPricingResponse,
	type IRegisterPushTargetRequest,
	type IRegisterPushTargetResponse,
	type ISubscribeRequest,
	type ISubscribeResponse,
	type IUserInfo,
	type IUserTemplateInfo,
	type IUserUpdate,
	type IUserWidgetInfo,
	isLocalUserSub,
	userLookupFromClaims,
} from "@flow-like/flow-like-ui/state/backend-state/user-state";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import { ApiResponseError } from "../../lib/api-error";
import { appsDB, type IShortcut } from "../../lib/apps-db";
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

function sortNotificationsByCreatedAtDesc(
	notifications: INotification[],
): INotification[] {
	return notifications.sort(
		(a, b) =>
			new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
	);
}

// The hub serializes the sea-orm model directly: JSON key `type` with values
// "Workflow"/"System". The UI contract is `notification_type` with "WORKFLOW"/
// "SYSTEM", so map it here at the boundary.
function normalizeRemoteNotification(raw: INotification): INotification {
	const rawType =
		(raw as { notification_type?: string }).notification_type ??
		(raw as { type?: string }).type;
	const notification_type =
		typeof rawType === "string" && rawType.toUpperCase() === "WORKFLOW"
			? "WORKFLOW"
			: "SYSTEM";
	return { ...raw, notification_type };
}

// A local workflow notification and its hub-persisted copy share a source run +
// node but never an id, so key duplicate detection on the pair.
function notificationRunKey(notification: INotification): string | null {
	return notification.source_run_id && notification.source_node_id
		? `${notification.source_run_id}::${notification.source_node_id}`
		: null;
}

const REMOTE_NOTIFICATION_PAGE_SIZE = 100;

function normalizeProfileShortcut(
	shortcut: IProfileShortcut,
	profileId: string,
): IShortcut {
	return {
		...shortcut,
		profileId,
		appId: shortcut.appId ?? undefined,
		icon: shortcut.icon ?? undefined,
	};
}

export class UserState implements IUserState {
	constructor(private readonly backend: TauriBackend) {}

	private hasRemoteAccessToken(): boolean {
		return Boolean(
			this.backend.profile &&
				this.backend.auth?.isAuthenticated &&
				this.backend.auth.user?.access_token,
		);
	}

	private getUserId(): string {
		return this.backend.auth?.user?.profile?.sub ?? "offline-user";
	}

	private getRelevantLocalUserIds(): string[] {
		const userId = this.getUserId();
		return userId === "offline-user" ? [userId] : [userId, "offline-user"];
	}

	private async getMergedLocalNotificationCounts(): Promise<{
		total: number;
		unread: number;
	}> {
		const counts = await Promise.all(
			this.getRelevantLocalUserIds().map((userId) =>
				getLocalNotificationCounts(userId),
			),
		);

		return counts.reduce(
			(acc, count) => ({
				total: acc.total + count.total,
				unread: acc.unread + count.unread,
			}),
			{ total: 0, unread: 0 },
		);
	}

	private async getMergedLocalNotifications(
		limit: number,
		unreadOnly: boolean,
	): Promise<INotification[]> {
		const localNotifications = await Promise.all(
			this.getRelevantLocalUserIds().map((userId) =>
				getLocalNotifications(userId, limit, 0, unreadOnly),
			),
		);

		const merged = new Map<string, INotification>();
		for (const notification of localNotifications
			.flat()
			.map(localToINotification)) {
			merged.set(notification.id, notification);
		}

		return sortNotificationsByCreatedAtDesc([...merged.values()]).slice(
			0,
			limit,
		);
	}

	private async markAllRelevantLocalNotificationsRead(): Promise<number> {
		const counts = await Promise.all(
			this.getRelevantLocalUserIds().map((userId) =>
				markAllLocalNotificationsRead(userId),
			),
		);

		return counts.reduce((sum, count) => sum + count, 0);
	}

	/**
	 * Local executions report the "local" sub, which no account matches —
	 * resolve it to the signed-in user, falling back to cached auth claims
	 * when the hub is unreachable.
	 */
	private async lookupCurrentUser(): Promise<IUserLookup> {
		const claims = this.backend.auth?.user?.profile;
		const sub = claims?.sub;

		if (sub && !isLocalUserSub(sub) && this.hasRemoteAccessToken()) {
			try {
				return await this.lookupUser(sub);
			} catch (error) {
				console.warn(
					"[UserState.lookupUser] falling back to auth claims for the local sub:",
					error,
				);
			}
		}

		return userLookupFromClaims(claims);
	}

	async lookupUser(userId: string): Promise<IUserLookup> {
		if (isLocalUserSub(userId)) {
			return await this.lookupCurrentUser();
		}

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
		const trimmed = query.trim();
		if (!trimmed || !this.backend.profile || !this.backend.auth) {
			return [];
		}

		try {
			const result = await fetcher<IUserLookup[]>(
				this.backend.profile,
				`user/search/${encodeURIComponent(trimmed)}`,
				{
					method: "GET",
				},
				this.backend.auth,
			);

			return result ?? [];
		} catch (error) {
			console.warn("[UserState.searchUsers] search failed:", error);
			return [];
		}
	}
	async getNotifications(): Promise<INotificationsOverview> {
		// Get local notifications first (works offline)
		let localCounts = { total: 0, unread: 0 };
		try {
			localCounts = await this.getMergedLocalNotificationCounts();
		} catch (e) {
			console.error(
				"[UserState.getNotifications] Error getting local counts:",
				e,
			);
		}

		// Try to get remote notifications if online
		if (
			this.backend.profile &&
			this.backend.auth &&
			this.hasRemoteAccessToken()
		) {
			try {
				const remoteResult = await fetcher<INotificationsOverview>(
					this.backend.profile,
					"user/notifications",
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

	private async fetchRemoteNotifications(
		unreadOnly: boolean,
		count: number,
	): Promise<INotification[]> {
		const collected: INotification[] = [];
		let pageOffset = 0;

		// The server clamps `limit` to 100; page through it so the merged list
		// can extend past the first 100 items instead of dead-ending there.
		while (collected.length < count) {
			const params = new URLSearchParams({
				limit: Math.min(
					REMOTE_NOTIFICATION_PAGE_SIZE,
					count - collected.length,
				).toString(),
				offset: pageOffset.toString(),
				unread_only: unreadOnly.toString(),
			});

			const batch = await fetcher<INotification[]>(
				// biome-ignore lint/style/noNonNullAssertion: callers guard presence
				this.backend.profile!,
				`user/notifications/list?${params}`,
				{ method: "GET" },
				// biome-ignore lint/style/noNonNullAssertion: callers guard presence
				this.backend.auth!,
			);

			if (!batch.length) break;
			collected.push(...batch);
			if (batch.length < REMOTE_NOTIFICATION_PAGE_SIZE) break;
			pageOffset += REMOTE_NOTIFICATION_PAGE_SIZE;
		}

		return collected.map(normalizeRemoteNotification);
	}

	async listNotifications(
		unreadOnly = false,
		offset = 0,
		limit = 20,
	): Promise<INotification[]> {
		// Get local notifications first (works offline)
		let localNotifications: INotification[] = [];
		try {
			localNotifications = await this.getMergedLocalNotifications(
				limit + offset,
				unreadOnly,
			);
		} catch (e) {
			console.error(
				"[UserState.listNotifications] Error getting local notifications:",
				e,
			);
		}

		// Try to get remote notifications if online
		let remoteResult: INotification[] = [];
		if (
			this.backend.profile &&
			this.backend.auth &&
			this.hasRemoteAccessToken()
		) {
			try {
				remoteResult = await this.fetchRemoteNotifications(
					unreadOnly,
					limit + offset,
				);
			} catch (error) {
				// Offline / network failures fall back to local history silently. A
				// genuine server error with nothing local to show is surfaced so the
				// page renders an error state instead of a misleading "you're caught
				// up".
				if (
					error instanceof ApiResponseError &&
					localNotifications.length === 0
				) {
					throw error;
				}
				console.warn(
					"[UserState.listNotifications] remote fetch failed:",
					error,
				);
			}
		}

		// The hub owns persistence for signed-in runs, so a workflow notification
		// can exist both remotely and as a local shadow (different ids, same source
		// run + node). Prefer the remote copy and drop the local duplicate.
		const remoteRunKeys = new Set<string>();
		for (const notification of remoteResult) {
			const key = notificationRunKey(notification);
			if (key) remoteRunKeys.add(key);
		}

		const byId = new Map<string, INotification>();
		for (const notification of remoteResult) {
			byId.set(notification.id, notification);
		}
		for (const notification of localNotifications) {
			const key = notificationRunKey(notification);
			if (key && remoteRunKeys.has(key)) continue;
			if (!byId.has(notification.id)) byId.set(notification.id, notification);
		}

		const merged = sortNotificationsByCreatedAtDesc([...byId.values()]);

		// Apply pagination to merged result
		return merged.slice(offset, offset + limit);
	}

	async markNotificationRead(notificationId: string): Promise<void> {
		// Try local first
		try {
			if (await markLocalNotificationRead(notificationId)) {
				return;
			}
		} catch {
			// Not a local notification, try remote
		}

		// Only attempt remote if authenticated
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			return; // Silently succeed for offline mode
		}

		await fetcher(
			this.backend.profile,
			`user/notifications/${notificationId}/read`,
			{
				method: "POST",
			},
			this.backend.auth,
		);
	}

	async deleteNotification(notificationId: string): Promise<void> {
		// Try local first
		try {
			if (await deleteLocalNotification(notificationId)) {
				return;
			}
		} catch {
			// Not a local notification, try remote
		}

		// Only attempt remote if authenticated
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
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
		if (
			this.backend.profile &&
			this.backend.auth &&
			this.hasRemoteAccessToken()
		) {
			try {
				remoteResult = await fetcher<number>(
					this.backend.profile,
					"user/notifications/read-all",
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
		let localCount = 0;
		try {
			localCount = await this.markAllRelevantLocalNotificationsRead();
		} catch {
			// Ignore local errors
		}

		return remoteResult + localCount;
	}

	async registerPushTarget(
		request: IRegisterPushTargetRequest,
	): Promise<IRegisterPushTargetResponse> {
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			throw new Error("Profile or auth context not available");
		}

		return fetcher<IRegisterPushTargetResponse>(
			this.backend.profile,
			"user/push-targets/register",
			{
				method: "POST",
				body: JSON.stringify(request),
			},
			this.backend.auth,
		);
	}

	async getPushTargetStatus(deviceId: string): Promise<IPushTargetStatus> {
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			throw new Error("Profile or auth context not available");
		}

		return fetcher<IPushTargetStatus>(
			this.backend.profile,
			`user/push-targets/${encodeURIComponent(deviceId)}`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async setPushTargetEnabled(
		deviceId: string,
		enabled: boolean,
	): Promise<IPushTargetStatus> {
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			throw new Error("Profile or auth context not available");
		}

		return fetcher<IPushTargetStatus>(
			this.backend.profile,
			`user/push-targets/${encodeURIComponent(deviceId)}`,
			{
				method: "PATCH",
				body: JSON.stringify({ push_enabled: enabled }),
			},
			this.backend.auth,
		);
	}

	async getProfile(): Promise<IProfile> {
		const profile: ISettingsProfile = await invoke("get_current_profile");
		if (profile.hub_profile === undefined) {
			throw new Error("Profile not found");
		}
		return profile.hub_profile;
	}
	async getProfiles(): Promise<IProfile[]> {
		const profiles =
			await invoke<Record<string, ISettingsProfile>>("get_profiles");
		return Object.values(profiles)
			.map((p) => p.hub_profile)
			.filter((p): p is IProfile => p !== undefined);
	}
	async getSettingsProfile(): Promise<ISettingsProfile> {
		const profile: ISettingsProfile = await invoke("get_current_profile");
		return profile;
	}
	async getAllSettingsProfiles(): Promise<ISettingsProfile[]> {
		const profiles =
			await invoke<Record<string, ISettingsProfile>>("get_profiles");
		return Object.values(profiles);
	}

	async updateUser(data: IUserUpdate, avatar?: File): Promise<void> {
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			throw new Error("Profile or auth context not available");
		}

		if (avatar) {
			data.avatar_extension = avatar.name.split(".").pop() || "";
		}

		const response = await fetcher<{ signed_url?: string }>(
			this.backend.profile,
			"user/info",
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
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<IUserInfo>(
			this.backend.profile,
			"user/info",
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

	async updateProfileShortcuts(
		profile: ISettingsProfile,
		shortcuts: IProfileShortcut[],
	): Promise<void> {
		const profileId = profile.hub_profile.id;
		if (!profileId) {
			throw new Error("Profile ID is required");
		}

		const normalizedShortcuts = shortcuts.map((shortcut) =>
			normalizeProfileShortcut(shortcut, profileId),
		);
		const nextShortcutIds = new Set(
			normalizedShortcuts.map((shortcut) => shortcut.id),
		);
		const localShortcuts = await appsDB.shortcuts
			.where("profileId")
			.equals(profileId)
			.toArray();

		await appsDB.transaction("rw", appsDB.shortcuts, async () => {
			for (const shortcut of normalizedShortcuts) {
				await appsDB.shortcuts.put(shortcut);
			}
			for (const localShortcut of localShortcuts) {
				if (!nextShortcutIds.has(localShortcut.id)) {
					await appsDB.shortcuts.delete(localShortcut.id);
				}
			}
		});

		profile.hub_profile.shortcuts = normalizedShortcuts;
		await invoke("profile_update_shortcuts", {
			profileId,
			shortcuts: normalizedShortcuts,
		});

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			return;
		}

		await fetcher(
			this.backend.profile,
			`profile/${profileId}`,
			{
				method: "POST",
				body: JSON.stringify({ shortcuts: normalizedShortcuts }),
			},
			this.backend.auth,
		);
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
			"user/pat",
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
			"user/pat",
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
			const localApps =
				await invoke<[{ id: string }, IMetadata | undefined][]>("get_apps");
			for (const [app] of localApps) {
				try {
					const widgets = await invoke<{ id: string }[]>("get_widgets", {
						appId: app.id,
					});
					for (const widget of widgets) {
						try {
							const metadata = await invoke<IMetadata | null>(
								"get_widget_meta",
								{
									appId: app.id,
									widgetId: widget.id,
									language,
								},
							);
							const widgetName =
								typeof metadata?.name === "string" ? metadata.name.trim() : "";
							if (!widgetName || widgetName === widget.id) continue;

							const key = `${app.id}:${widget.id}`;
							mergedWidgets.set(key, {
								appId: app.id,
								widgetId: widget.id,
								metadata: {
									name: widgetName,
									description: metadata?.description ?? "",
									thumbnail: metadata?.thumbnail ?? null,
									tags: metadata?.tags ?? [],
									icon: metadata?.icon ?? null,
									preview_media: metadata?.preview_media ?? [],
								},
							});
						} catch {
							// Widgets without metadata names are intentionally omitted.
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
				const remoteWidgets = await fetcher<[string, string, IMetadata][]>(
					this.backend.profile,
					`user/widgets${queryParams}`,
					{ method: "GET" },
					this.backend.auth,
				);

				for (const [appId, widgetId, metadata] of remoteWidgets) {
					const widgetName =
						typeof metadata?.name === "string" ? metadata.name.trim() : "";
					if (!widgetName || widgetName === widgetId) continue;

					const key = `${appId}:${widgetId}`;
					mergedWidgets.set(key, {
						appId,
						widgetId,
						metadata: {
							name: widgetName,
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
		const result = await fetcher<[string, string, IMetadata][]>(
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
