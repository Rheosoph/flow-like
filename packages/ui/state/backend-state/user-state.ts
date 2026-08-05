import type { IProfile, IProfileApp, IProfileShortcut } from "../../lib";
import type { ISettingsProfile } from "../../types";
import type {
	INotification,
	INotificationsOverview,
	IUserLookup,
} from "./types";

export interface IUserUpdate {
	name?: string;
	description?: string;
	avatar_extension?: string;
	accepted_terms_version?: string;
	tutorial_completed?: boolean;
}

/**
 * Sub reported by executions without an authenticated caller (local/offline
 * runs, anonymous invocations). It never identifies a stored account, so the
 * frontend resolves it to whoever is currently signed in.
 */
export const LOCAL_USER_SUB = "local";

/** Subset of the OIDC id-token claims used to describe the signed-in user. */
export interface IUserClaims {
	sub?: string;
	name?: string;
	preferred_username?: string;
	nickname?: string;
	email?: string;
	picture?: string;
}

export function isLocalUserSub(userId?: string | null): boolean {
	return userId?.trim().toLowerCase() === LOCAL_USER_SUB;
}

/**
 * First candidate that identifies a stored account. The local placeholder never
 * does, so it is dropped before an id is rendered or linked to a profile page.
 */
export function resolveAccountId(
	...candidates: (string | null | undefined)[]
): string | undefined {
	const match = candidates.find(
		(candidate) => candidate && !isLocalUserSub(candidate),
	);
	return match ?? undefined;
}

/**
 * Build a lookup record for the signed-in user from cached auth claims. Used
 * when the local sub cannot be resolved against the hub (offline, no account).
 */
export function userLookupFromClaims(claims?: IUserClaims | null): IUserLookup {
	return {
		id: claims?.sub ?? LOCAL_USER_SUB,
		name: claims?.name ?? (claims?.sub ? undefined : "You"),
		preferred_username: claims?.preferred_username,
		username: claims?.nickname,
		email: claims?.email,
		avatar_url: claims?.picture,
		created_at: "",
	};
}

export interface IUserInfo {
	id: string;
	stripeId?: string;
	email?: string;
	username?: string;
	preferred_username?: string;
	name?: string;
	description?: string;
	avatar?: string;

	permission?: number;
	accepted_terms_version?: string;
	tutorial_completed?: boolean;

	status?: string;
	tier?: string;

	total_size?: number;

	created_at?: string;
	updated_at?: string;
}

export interface IPriceInfo {
	amount: number;
	currency: string;
	interval?: string;
}

export interface ITierInfo {
	name: string;
	display_name?: string;
	tagline?: string;
	features?: string[];
	highlight?: boolean;
	badge?: string;
	product_id?: string;
	max_non_visible_projects: number;
	max_remote_executions: number;
	execution_tier: string;
	max_total_size: number;
	max_llm_cost: number;
	max_llm_calls?: number;
	llm_tiers: string[];
	price?: IPriceInfo;
	contact_url?: string;
}

export interface IConversionInfo {
	enabled: boolean;
	mode: "consumer" | "enterprise" | string;
	headline?: string;
	subheadline?: string;
	contact_name: string;
	contact_email: string;
	contact_url: string;
	contact_message?: string;
}

export interface IPricingResponse {
	current_tier: string;
	tiers: Record<string, ITierInfo>;
	conversion?: IConversionInfo;
}

export interface ISubscribeRequest {
	tier: string;
	success_url: string;
	cancel_url: string;
}

export interface ISubscribeResponse {
	checkout_url: string;
	session_id: string;
}

export interface IBillingSession {
	session_id: string;
	url: string;
}

export type PushTargetPlatform = "IOS" | "ANDROID" | "DESKTOP";

export interface IRegisterPushTargetRequest {
	device_id: string;
	platform: PushTargetPlatform;
	token: string;
	device_name?: string;
	channel_id?: string | null;
	metadata?: Record<string, unknown>;
}

export interface IRegisterPushTargetResponse {
	id: string;
	provider: string;
	success: boolean;
	push_enabled: boolean;
}

export interface IPushTargetStatus {
	device_id: string;
	provider?: string | null;
	registered: boolean;
	push_enabled: boolean;
	platform?: PushTargetPlatform | null;
	device_name?: string | null;
	channel_id?: string | null;
	failure_count?: number | null;
	last_registered_at?: string | null;
	last_seen_at?: string | null;
	invalidated_at?: string | null;
	invalidation_reason?: string | null;
	updated_at?: string | null;
}

/** Widget info returned from the user widgets endpoint */
export interface IUserWidgetInfo {
	/** The app ID where the widget is defined */
	appId: string;
	/** The widget ID */
	widgetId: string;
	/** Widget metadata */
	metadata: {
		name: string;
		description: string;
		thumbnail?: string | null;
		tags: string[];
		icon?: string | null;
		preview_media?: string[];
	};
}

/** Template info returned from the user templates endpoint */
export interface IUserTemplateInfo {
	/** The app ID where the template is defined */
	appId: string;
	/** The template ID */
	templateId: string;
	/** Template metadata */
	metadata: {
		name: string;
		description: string;
		thumbnail?: string | null;
		tags: string[];
		icon?: string | null;
		preview_media?: string[];
	};
}

export interface IUserState {
	lookupUser(userId: string): Promise<IUserLookup>;
	searchUsers(query: string): Promise<IUserLookup[]>;
	getNotifications(): Promise<INotificationsOverview>;
	listNotifications(
		unreadOnly?: boolean,
		offset?: number,
		limit?: number,
	): Promise<INotification[]>;
	markNotificationRead(notificationId: string): Promise<void>;
	deleteNotification(notificationId: string): Promise<void>;
	markAllNotificationsRead(): Promise<number>;
	registerPushTarget(
		request: IRegisterPushTargetRequest,
	): Promise<IRegisterPushTargetResponse>;
	getPushTargetStatus(deviceId: string): Promise<IPushTargetStatus>;
	setPushTargetEnabled(
		deviceId: string,
		enabled: boolean,
	): Promise<IPushTargetStatus>;
	getProfile(): Promise<IProfile>;
	getProfiles(): Promise<IProfile[]>;
	getSettingsProfile(): Promise<ISettingsProfile>;
	getAllSettingsProfiles(): Promise<ISettingsProfile[]>;
	updateUser(data: IUserUpdate, avatar?: File): Promise<void>;
	updateProfileApp(
		profile: ISettingsProfile,
		app: IProfileApp,
		operation: "Upsert" | "Remove",
	): Promise<void>;
	updateProfileShortcuts(
		profile: ISettingsProfile,
		shortcuts: IProfileShortcut[],
	): Promise<void>;
	getInfo(): Promise<IUserInfo>;
	createPAT(
		name: string,
		validUntil?: Date,
		permissions?: number,
	): Promise<{ pat: string; permission: number }>;
	getPATs(): Promise<
		{
			id: string;
			name: string;
			created_at: string;
			valid_until: string | null;
			permission: number;
		}[]
	>;
	deletePAT(id: string): Promise<void>;
	getPricing(): Promise<IPricingResponse>;
	createSubscription(request: ISubscribeRequest): Promise<ISubscribeResponse>;
	getBillingSession(): Promise<IBillingSession>;
	/** Get all widgets accessible to the user across all apps with ReadWidgets permission */
	getUserWidgets(language?: string): Promise<IUserWidgetInfo[]>;
	/** Get all templates accessible to the user across all apps with ReadTemplates permission */
	getUserTemplates(language?: string): Promise<IUserTemplateInfo[]>;
}
