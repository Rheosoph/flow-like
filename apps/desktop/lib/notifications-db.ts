import Dexie, { type EntityTable } from "dexie";

export interface ILocalNotification {
	id: string;
	userId: string;
	appId?: string;
	title: string;
	description?: string;
	icon?: string;
	link?: string;
	notificationType: "WORKFLOW" | "SYSTEM";
	read: boolean;
	sourceRunId?: string;
	sourceNodeId?: string;
	createdAt: string;
	readAt?: string;
}

const notificationsDB = new Dexie("Notifications") as Dexie & {
	notifications: EntityTable<ILocalNotification, "id">;
};

notificationsDB.version(1).stores({
	notifications: "id, userId, appId, read, createdAt",
});

export { notificationsDB };

export async function addLocalNotification(
	notification: Omit<ILocalNotification, "id" | "createdAt" | "read">,
): Promise<ILocalNotification> {
	const id = crypto.randomUUID();
	const createdAt = new Date().toISOString();
	const fullNotification: ILocalNotification = {
		...notification,
		id,
		createdAt,
		read: false,
	};
	await notificationsDB.notifications.add(fullNotification);
	return fullNotification;
}

export async function getLocalNotifications(
	userId: string,
	limit = 20,
	offset = 0,
	unreadOnly = false,
): Promise<ILocalNotification[]> {
	// Get all notifications for user and sort by createdAt descending
	let all = await notificationsDB.notifications
		.where("userId")
		.equals(userId)
		.toArray();

	// Sort by createdAt descending (newest first)
	all.sort(
		(a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime(),
	);

	if (unreadOnly) {
		all = all.filter((n) => !n.read);
	}

	return all.slice(offset, offset + limit);
}

export async function markLocalNotificationRead(id: string): Promise<boolean> {
	const updated = await notificationsDB.notifications.update(id, {
		read: true,
		readAt: new Date().toISOString(),
	});

	return updated > 0;
}


export async function deleteLocalNotification(id: string): Promise<boolean> {
	const existing = await notificationsDB.notifications.get(id);
	if (!existing) {
		return false;
	}

	await notificationsDB.notifications.delete(id);
	return true;
}

export async function markAllLocalNotificationsRead(
	userId: string,
): Promise<number> {
	const unread = await notificationsDB.notifications
		.where("userId")
		.equals(userId)
		.filter((n) => !n.read)
		.toArray();

	const readAt = new Date().toISOString();
	await notificationsDB.notifications.bulkUpdate(
		unread.map((n) => ({ key: n.id, changes: { read: true, readAt } })),
	);

	return unread.length;
}

export async function getLocalNotificationCounts(
	userId: string,
): Promise<{ total: number; unread: number }> {
	const all = await notificationsDB.notifications
		.where("userId")
		.equals(userId)
		.toArray();

	return {
		total: all.length,
		unread: all.filter((n) => !n.read).length,
	};
}
