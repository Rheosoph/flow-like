import type {
	BotPermission,
	BotRestrictionPreset,
	IBotRestrictions,
	IBotScopes,
} from "../../state/backend-state/bot-state";

export const EMPTY_SCOPES: IBotScopes = {
	files: { prefixes: [], read_only: false },
};

const READ_ONLY: readonly BotPermission[] = [
	"read_files",
	"read_database",
	"read_boards",
	"list_events",
	"read_events",
];

export function presetRestrictions(
	preset: BotRestrictionPreset,
): IBotRestrictions {
	if (preset === "full") return { permissions: null, scopes: EMPTY_SCOPES };
	return {
		permissions:
			preset === "read_only"
				? [...READ_ONLY]
				: [...READ_ONLY, "execute_events"],
		scopes: EMPTY_SCOPES,
	};
}

export const NO_RESTRICTIONS: IBotRestrictions = presetRestrictions("full");

export const RESTRICTION_PRESETS: readonly BotRestrictionPreset[] = [
	"read_only",
	"read_and_call_events",
	"full",
];

function sameSet(a: readonly string[], b: readonly string[]) {
	return a.length === b.length && a.every((entry) => b.includes(entry));
}

function sameList(a?: string[] | null, b?: string[] | null) {
	if (!a || !b) return !a && !b;
	return sameSet(a, b);
}

export function sameScopes(a: IBotScopes, b: IBotScopes) {
	return (
		a.files.read_only === b.files.read_only &&
		sameSet(a.files.prefixes, b.files.prefixes) &&
		sameList(a.tables, b.tables) &&
		sameList(a.events, b.events) &&
		sameList(a.boards, b.boards)
	);
}

/** The preset these restrictions equal; null once they are customised. */
export function detectPreset(
	restrictions: IBotRestrictions,
): BotRestrictionPreset | null {
	return (
		RESTRICTION_PRESETS.find((preset) => {
			const candidate = presetRestrictions(preset);
			const samePermissions =
				candidate.permissions === null || restrictions.permissions === null
					? candidate.permissions === restrictions.permissions
					: sameSet(candidate.permissions, restrictions.permissions);
			return (
				samePermissions && sameScopes(candidate.scopes, restrictions.scopes)
			);
		}) ?? null
	);
}

export type PermissionGroupId =
	| "files"
	| "tables"
	| "boards"
	| "events"
	| "other";

export interface PermissionGroup {
	id: PermissionGroupId;
	permissions: BotPermission[];
}

export const PERMISSION_GROUPS: readonly PermissionGroup[] = [
	{
		id: "files",
		permissions: ["read_files", "write_files"],
	},
	{
		id: "tables",
		permissions: ["read_database", "write_database"],
	},
	{
		id: "boards",
		permissions: ["read_boards", "execute_boards", "write_boards"],
	},
	{
		id: "events",
		permissions: [
			"list_events",
			"read_events",
			"execute_events",
			"write_events",
		],
	},
	{
		id: "other",
		permissions: [
			"invoke_api",
			"read_logs",
			"read_team",
			"read_config",
			"read_templates",
			"write_templates",
			"read_courses",
			"write_courses",
			"read_widgets",
			"write_widgets",
		],
	},
];

const ALL_PERMISSIONS: readonly BotPermission[] = PERMISSION_GROUPS.flatMap(
	(group) => group.permissions,
);

export function holds(restrictions: IBotRestrictions, id: BotPermission) {
	return (
		restrictions.permissions === null || restrictions.permissions.includes(id)
	);
}

/** Toggling from the owner's whole role first spells that role out, so one switch narrows it. */
export function togglePermission(
	restrictions: IBotRestrictions,
	id: BotPermission,
	on: boolean,
): IBotRestrictions {
	const current = restrictions.permissions ?? [...ALL_PERMISSIONS];
	const next = on
		? [...new Set([...current, id])]
		: current.filter((entry) => entry !== id);
	return { ...restrictions, permissions: next };
}

export type ScopeList = "tables" | "events" | "boards";

export function setScopeList(
	restrictions: IBotRestrictions,
	list: ScopeList,
	value: string[] | null,
): IBotRestrictions {
	return { ...restrictions, scopes: { ...restrictions.scopes, [list]: value } };
}

export function setFolders(
	restrictions: IBotRestrictions,
	prefixes: string[],
): IBotRestrictions {
	return {
		...restrictions,
		scopes: {
			...restrictions.scopes,
			files: { ...restrictions.scopes.files, prefixes },
		},
	};
}

export function setFilesReadOnly(
	restrictions: IBotRestrictions,
	readOnly: boolean,
): IBotRestrictions {
	return {
		...restrictions,
		scopes: {
			...restrictions.scopes,
			files: { ...restrictions.scopes.files, read_only: readOnly },
		},
	};
}

/** A folder as the server stores it: no leading or trailing slash. */
export function normalizeFolder(input: string) {
	return input.trim().replace(/^\/+|\/+$/g, "");
}

export function normalizeRestrictions(
	value: Partial<IBotRestrictions> | undefined | null,
): IBotRestrictions {
	return {
		permissions: value?.permissions ?? null,
		scopes: {
			...value?.scopes,
			files: {
				prefixes: value?.scopes?.files?.prefixes ?? [],
				read_only: value?.scopes?.files?.read_only ?? false,
			},
		},
	};
}
