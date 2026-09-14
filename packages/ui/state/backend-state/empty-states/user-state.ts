import type {
	IProfile,
	IProfileApp,
	IProfileShortcut,
	ISettingsProfile,
	IUserState,
} from "@flow-like/flow-like-ui";
import type {
	INotification,
	INotificationsOverview,
	IProjectContactsPage,
	IUserLookup,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import type {
	IBillingSession,
	IPricingResponse,
	QuotaOverview,
	QuotaOperationsPage,
	QuotaOperationDetail,
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
import type {
	IHomeDefault,
	IHomeDefaults,
	IHomeLayout,
} from "../../../components/home/types";

export class EmptyUserState implements IUserState {
	getHomeDefaults(defaultId?: string): Promise<IHomeDefaults> {
		return Promise.resolve({ main: null, profile: null });
	}
	saveHomeLayout(
		layout: IHomeLayout | null,
		profileId?: string,
	): Promise<IProfile> {
		throw new Error("Home layouts are unavailable.");
	}
	saveHomeDefault(
		id: string,
		layout: IHomeLayout | null,
		expectedRevision?: string | null,
	): Promise<IHomeDefault | null> {
		throw new Error("Home defaults are unavailable.");
	}
	lookupUser(userId: string): Promise<IUserLookup> {
		throw new Error("Method not implemented.");
	}
	lookupUsers(userIds: string[]): Promise<IUserLookup[]> {
		throw new Error("Method not implemented.");
	}
	searchUsers(query: string, appId?: string): Promise<IUserLookup[]> {
		throw new Error("Method not implemented.");
	}
	getProjectContacts(
		appId: string,
		after?: string,
	): Promise<IProjectContactsPage> {
		return Promise.resolve({ users: [], next_cursor: null });
	}
	getNotifications(): Promise<INotificationsOverview> {
		throw new Error("Method not implemented.");
	}
	getProfile(): Promise<IProfile> {
		throw new Error("Method not implemented.");
	}
	getProfiles(): Promise<IProfile[]> {
		throw new Error("Method not implemented.");
	}
	getSettingsProfile(): Promise<ISettingsProfile> {
		throw new Error("Method not implemented.");
	}
	getAllSettingsProfiles(): Promise<ISettingsProfile[]> {
		throw new Error("Method not implemented.");
	}
	updateUser(data: IUserUpdate, avatar?: File): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getInfo(): Promise<IUserInfo> {
		throw new Error("Method not implemented.");
	}
	updateProfileApp(
		profile: ISettingsProfile,
		app: IProfileApp,
		operation: "Upsert" | "Remove",
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	updateProfileShortcuts(
		profile: ISettingsProfile,
		shortcuts: IProfileShortcut[],
	): Promise<void> {
		throw new Error("Method not implemented.");
	}

	createPAT(
		name: string,
		validUntil?: Date,
		permissions?: number,
	): Promise<{ pat: string; permission: number }> {
		throw new Error("Method not implemented.");
	}

	getPATs(): Promise<
		{
			id: string;
			name: string;
			created_at: string;
			valid_until: string | null;
			permission: number;
		}[]
	> {
		throw new Error("Method not implemented.");
	}

	deletePAT(id: string): Promise<void> {
		throw new Error("Method not implemented.");
	}

	getQuotaOperations(cursor?: string): Promise<QuotaOperationsPage> {
		throw new Error("Usage is unavailable without an account.");
	}

	getQuotaOperationDetail(id: string): Promise<QuotaOperationDetail> {
		throw new Error("Usage is unavailable without an account.");
	}

	getQuotaUsage(): Promise<QuotaOverview> {
		throw new Error("Usage is unavailable without an account.");
	}

	getPricing(): Promise<IPricingResponse> {
		throw new Error("Method not implemented.");
	}

	createSubscription(request: ISubscribeRequest): Promise<ISubscribeResponse> {
		throw new Error("Method not implemented.");
	}

	getBillingSession(): Promise<IBillingSession> {
		throw new Error("Method not implemented.");
	}

	listNotifications(): Promise<INotification[]> {
		throw new Error("Method not implemented.");
	}

	markNotificationRead(notificationId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}

	deleteNotification(notificationId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}

	markAllNotificationsRead(): Promise<number> {
		throw new Error("Method not implemented.");
	}

	registerPushTarget(
		request: IRegisterPushTargetRequest,
	): Promise<IRegisterPushTargetResponse> {
		throw new Error("Method not implemented.");
	}

	getPushTargetStatus(deviceId: string): Promise<IPushTargetStatus> {
		throw new Error("Method not implemented.");
	}

	setPushTargetEnabled(
		deviceId: string,
		enabled: boolean,
	): Promise<IPushTargetStatus> {
		throw new Error("Method not implemented.");
	}

	getUserWidgets(language?: string): Promise<IUserWidgetInfo[]> {
		return Promise.resolve([]);
	}

	getUserTemplates(language?: string): Promise<IUserTemplateInfo[]> {
		return Promise.resolve([]);
	}
}
