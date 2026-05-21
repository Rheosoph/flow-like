import { runIDBCleanup } from "@flow-like/flow-like-ui/lib/idb-cleanup";
import { notificationsDB } from "./notifications-db";

const NOTIFICATION_MAX_AGE_DAYS = 14;
const DAY_MS = 24 * 60 * 60 * 1000;

async function pruneReadNotifications(): Promise<void> {
	const cutoff = new Date(
		Date.now() - NOTIFICATION_MAX_AGE_DAYS * DAY_MS,
	).toISOString();
	const old = await notificationsDB.notifications
		.filter((n) => n.read && n.createdAt < cutoff)
		.primaryKeys();
	if (old.length > 0) await notificationsDB.notifications.bulkDelete(old);
}

let cleanupScheduled = false;

/**
 * Run IDB cleanup once per app session, deferred to avoid blocking startup.
 */
export function scheduleIDBCleanup(): void {
	if (cleanupScheduled) return;
	cleanupScheduled = true;

	// Defer cleanup to avoid competing with startup I/O
	setTimeout(async () => {
		try {
			await Promise.allSettled([runIDBCleanup(), pruneReadNotifications()]);
		} catch (e) {
			console.warn("[IDB Cleanup] error:", e);
		}
	}, 5_000);
}
