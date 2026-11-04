import type {
	BotPermission,
	IBotRestrictions,
} from "../../state/backend-state/bot-state";
import type { IBotRoomLimits } from "../../state/backend-state/inbox-state";
import { EMPTY_SCOPES } from "./bot-restrictions-model";

/** The role bit of each bot permission, equal to the server's `RolePermissions`. */
export const PERMISSION_BITS: Readonly<Record<BotPermission, number>> = {
	read_team: 1 << 2,
	read_files: 1 << 4,
	write_files: 1 << 5,
	invoke_api: 1 << 6,
	read_boards: 1 << 8,
	execute_boards: 1 << 9,
	write_boards: 1 << 10,
	list_events: 1 << 11,
	read_events: 1 << 12,
	execute_events: 1 << 13,
	write_events: 1 << 14,
	read_logs: 1 << 15,
	read_config: 1 << 17,
	read_templates: 1 << 19,
	write_templates: 1 << 20,
	read_courses: 1 << 21,
	write_courses: 1 << 22,
	read_widgets: 1 << 23,
	write_widgets: 1 << 24,
	read_database: 1 << 26,
	write_database: 1 << 27,
};

const ENTRIES = Object.entries(PERMISSION_BITS) as [BotPermission, number][];

export function maskFromPermissions(permissions: readonly BotPermission[]) {
	return permissions.reduce((mask, id) => mask | PERMISSION_BITS[id], 0);
}

export function permissionsFromMask(mask: number): BotPermission[] {
	return ENTRIES.filter(([, bit]) => (mask & bit) !== 0).map(([id]) => id);
}

/** What the dialog edits: the permission part as restrictions, and which attached apps stay usable. */
export interface RoomLimitsDraft {
	restrictions: IBotRestrictions;
	/** `null` keeps every app the bot has. */
	apps: string[] | null;
	mentionedOnly: boolean;
}

export const NO_ROOM_LIMITS: RoomLimitsDraft = {
	restrictions: { permissions: null, scopes: EMPTY_SCOPES },
	apps: null,
	mentionedOnly: false,
};

export function draftFromLimits(
	limits: IBotRoomLimits | null | undefined,
): RoomLimitsDraft {
	return {
		restrictions: {
			permissions:
				limits?.mask === undefined || limits.mask === null
					? null
					: permissionsFromMask(limits.mask),
			scopes: EMPTY_SCOPES,
		},
		apps: limits?.apps ?? null,
		mentionedOnly: limits?.mentioned_only === true,
	};
}

/** `null` when the draft adds no limit; scopes of the earlier limits are kept. */
export function limitsFromDraft(
	draft: RoomLimitsDraft,
	previous?: IBotRoomLimits | null,
): IBotRoomLimits | null {
	const limits: IBotRoomLimits = {};
	if (draft.restrictions.permissions !== null) {
		limits.mask = maskFromPermissions(draft.restrictions.permissions);
	}
	if (draft.apps !== null) limits.apps = draft.apps;
	if (draft.mentionedOnly) limits.mentioned_only = true;
	if (previous?.scopes && Object.keys(previous.scopes).length > 0) {
		limits.scopes = previous.scopes;
	}
	return Object.keys(limits).length > 0 ? limits : null;
}

export function toggleApp(
	draft: RoomLimitsDraft,
	appId: string,
	allApps: readonly string[],
	on: boolean,
): RoomLimitsDraft {
	const current = draft.apps ?? [...allApps];
	const next = on
		? [...new Set([...current, appId])]
		: current.filter((id) => id !== appId);
	return {
		...draft,
		apps: next.length === allApps.length ? null : next,
	};
}
