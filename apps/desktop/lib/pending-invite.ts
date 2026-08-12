const PENDING_INVITE_KEY = "flow-like-pending-invite";
const MAX_AGE_MS = 24 * 60 * 60 * 1000;

export interface PendingInvite {
	appId: string;
	token: string;
	ts: number;
}

/**
 * Desktop sign-in leaves the app (system browser) and onboarding may navigate
 * freely afterwards, so an invite must survive outside the /join route. This
 * store is written the moment /join opens and consumed once the join succeeds
 * or terminally fails.
 */
export function setPendingInvite(appId: string, token: string): void {
	try {
		localStorage.setItem(
			PENDING_INVITE_KEY,
			JSON.stringify({ appId, token, ts: Date.now() } satisfies PendingInvite),
		);
	} catch {}
}

export function getPendingInvite(): PendingInvite | null {
	try {
		const raw = localStorage.getItem(PENDING_INVITE_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as Partial<PendingInvite>;
		if (
			typeof parsed?.appId !== "string" ||
			typeof parsed?.token !== "string" ||
			typeof parsed?.ts !== "number" ||
			Date.now() - parsed.ts > MAX_AGE_MS
		) {
			clearPendingInvite();
			return null;
		}
		return parsed as PendingInvite;
	} catch {
		return null;
	}
}

export function clearPendingInvite(): void {
	try {
		localStorage.removeItem(PENDING_INVITE_KEY);
	} catch {}
}
