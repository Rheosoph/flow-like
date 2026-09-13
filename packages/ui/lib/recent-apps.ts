export interface RecentAppUse {
	appId: string;
	lastOpenedAt: string;
	openCount: number;
}
const PREFIX = "flow-like:recent-apps:v1:";
export const RECENT_APPS_CHANGED = "flow-like:recent-apps-changed";

export function recentAppsKey(scope: readonly string[]): string {
	return PREFIX + JSON.stringify(scope);
}

export function readRecentApps(scope: readonly string[]): RecentAppUse[] {
	try {
		const value: unknown = JSON.parse(
			localStorage.getItem(recentAppsKey(scope)) ?? "[]",
		);
		if (!Array.isArray(value)) return [];
		return value
			.filter(
				(row): row is RecentAppUse =>
					row &&
					typeof row.appId === "string" &&
					typeof row.lastOpenedAt === "string" &&
					Number.isFinite(Date.parse(row.lastOpenedAt)) &&
					Number.isInteger(row.openCount) &&
					row.openCount > 0,
			)
			.slice(0, 100);
	} catch {
		return [];
	}
}

export function recordRecentApp(
	scope: readonly string[],
	appId: string,
	now = new Date(),
): void {
	if (!appId) return;
	const rows = readRecentApps(scope);
	const previous = rows.find((row) => row.appId === appId);
	const next = [
		{
			appId,
			lastOpenedAt: now.toISOString(),
			openCount: (previous?.openCount ?? 0) + 1,
		},
		...rows.filter((row) => row.appId !== appId),
	].slice(0, 100);
	try {
		localStorage.setItem(recentAppsKey(scope), JSON.stringify(next));
		window.dispatchEvent(
			new CustomEvent(RECENT_APPS_CHANGED, { detail: recentAppsKey(scope) }),
		);
	} catch {
		/* App opening still works when local storage is unavailable. */
	}
}
