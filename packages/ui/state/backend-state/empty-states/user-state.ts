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

export class EmptyUserState implements IUserState {
	lookupUser(userId: string): Promise<IUserLookup> {
		throw new Error("Method not implemented.");
	}
	searchUsers(query: string): Promise<IUserLookup[]> {
		throw new Error("Method not implemented.");
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

	getPricing(): Promise<IPricingResponse> {
		throw new Error("Method not implemented.");
	}

	createSubscription(request: ISubscribeRequest): Promise<ISubscribeResponse> {
		throw new Error("Method not implemented.");
	}

	getBillingSession(): Promise<IBillingSession> {
		throw new Error("Method not implemented.");
	}

	listNotifications(
		unreadOnly?: boolean,
		offset?: number,
		limit?: number,
	): Promise<INotification[]> {
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
