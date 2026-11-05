const PENDING_INVITE_KEY = "flow-like-pending-invite";
const MAX_AGE_MS = 24 * 60 * 60 * 1000;

export type PendingInvite =
	| { kind: "app"; appId: string; token: string; ts: number }
	| { kind: "room"; token: string; ts: number };

/**
 * Desktop sign-in leaves the app (system browser) and onboarding may navigate
 * freely afterwards, so an invite must survive outside the /join route. This
 * store is written the moment /join opens and consumed once the join succeeds
 * or terminally fails.
 */
function store(invite: PendingInvite): void {
	try {
		localStorage.setItem(PENDING_INVITE_KEY, JSON.stringify(invite));
	} catch {}
}

export function setPendingInvite(appId: string, token: string): void {
	store({ kind: "app", appId, token, ts: Date.now() });
}

export function setPendingRoomInvite(token: string): void {
	store({ kind: "room", token, ts: Date.now() });
}

/** A value stored before `kind` existed is an app invite. */
export function parsePendingInvite(
	raw: string,
	now = Date.now(),
): PendingInvite | null {
	const parsed = JSON.parse(raw) as Record<string, unknown> | null;
	if (typeof parsed?.token !== "string" || typeof parsed?.ts !== "number")
		return null;
	if (now - parsed.ts > MAX_AGE_MS) return null;
	if (parsed.kind === "room")
		return { kind: "room", token: parsed.token, ts: parsed.ts };
	if (typeof parsed.appId !== "string") return null;
	return {
		kind: "app",
		appId: parsed.appId,
		token: parsed.token,
		ts: parsed.ts,
	};
}

export function pendingInviteUrl(invite: PendingInvite): string {
	return invite.kind === "room"
		? `/join?room=${encodeURIComponent(invite.token)}`
		: `/join?appId=${encodeURIComponent(invite.appId)}&token=${encodeURIComponent(invite.token)}`;
}

export function getPendingInvite(): PendingInvite | null {
	try {
		const raw = localStorage.getItem(PENDING_INVITE_KEY);
		if (!raw) return null;
		const invite = parsePendingInvite(raw);
		if (!invite) clearPendingInvite();
		return invite;
	} catch {
		return null;
	}
}

export function clearPendingInvite(): void {
	try {
		localStorage.removeItem(PENDING_INVITE_KEY);
	} catch {}
}
