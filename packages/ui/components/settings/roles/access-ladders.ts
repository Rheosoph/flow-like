import {
	BarChart3,
	BookOpen,
	FileText,
	type LucideIcon,
	Settings,
	SquareKanban,
	Users,
	Zap,
} from "lucide-react";
import { RolePermissions } from "../../../lib/permission/role-permission";

/**
 * Most permission groups are ladders, not independent switches: a role that may
 * edit workflows must be able to view them. Modelling them as ordered levels
 * keeps impossible combinations off the screen while `levelOf` still reports
 * -1 for any hand-rolled set, so the raw switches stay reachable.
 */
export type LadderTone = "none" | "view" | "run" | "edit" | "manage";

export interface AccessLevel {
	name: string;
	tone: LadderTone;
	/** Exact permissions this level grants — anything else in the ladder is revoked. */
	permissions: RolePermissions[];
	/** Verb phrase completing "Members can …". Absent on the None level. */
	can?: string;
	/** Verb phrase completing "Cannot …". Only on the None level. */
	cannot?: string;
}

export interface AccessLadder {
	id: string;
	label: string;
	/** Column heading above the collapsed-row gauges. */
	short: string;
	icon: LucideIcon;
	levels: AccessLevel[];
}

export const ACCESS_LADDERS: AccessLadder[] = [
	{
		id: "team",
		label: "Team & Access",
		short: "Team",
		icon: Users,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "see who else is in the app",
			},
			{
				name: "View",
				tone: "view",
				permissions: [RolePermissions.ReadTeam, RolePermissions.ReadRoles],
				can: "see members and how roles are defined",
			},
			{
				name: "Use API",
				tone: "edit",
				permissions: [
					RolePermissions.ReadTeam,
					RolePermissions.ReadRoles,
					RolePermissions.InvokeApi,
				],
				can: "see members and call the app's APIs",
			},
		],
	},
	{
		id: "files",
		label: "Files & Data",
		short: "Data",
		icon: FileText,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "open any file or database",
			},
			{
				name: "Read",
				tone: "view",
				permissions: [RolePermissions.ReadFiles, RolePermissions.ReadDatabase],
				can: "read files and databases",
			},
			{
				name: "Write",
				tone: "edit",
				permissions: [
					RolePermissions.ReadFiles,
					RolePermissions.WriteFiles,
					RolePermissions.ReadDatabase,
					RolePermissions.WriteDatabase,
					RolePermissions.WriteMeta,
				],
				can: "upload, change and delete files and data",
			},
		],
	},
	{
		id: "boards",
		label: "Workflows",
		short: "Flows",
		icon: SquareKanban,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "open workflows",
			},
			{
				name: "View",
				tone: "view",
				permissions: [RolePermissions.ReadBoards],
				can: "open workflows",
			},
			{
				name: "Run",
				tone: "run",
				permissions: [
					RolePermissions.ReadBoards,
					RolePermissions.ExecuteBoards,
				],
				can: "open and run workflows",
			},
			{
				name: "Edit",
				tone: "edit",
				permissions: [
					RolePermissions.ReadBoards,
					RolePermissions.ExecuteBoards,
					RolePermissions.WriteBoards,
				],
				can: "build and change workflows",
			},
		],
	},
	{
		id: "events",
		label: "Events",
		short: "Events",
		icon: Zap,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "see events",
			},
			{
				name: "Browse",
				tone: "view",
				permissions: [RolePermissions.ListEvents, RolePermissions.ReadEvents],
				can: "browse events and their payloads",
			},
			{
				name: "Trigger",
				tone: "run",
				permissions: [
					RolePermissions.ListEvents,
					RolePermissions.ReadEvents,
					RolePermissions.ExecuteEvents,
				],
				can: "trigger events",
			},
			{
				name: "Manage",
				tone: "edit",
				permissions: [
					RolePermissions.ListEvents,
					RolePermissions.ReadEvents,
					RolePermissions.ExecuteEvents,
					RolePermissions.WriteEvents,
				],
				can: "create and change events",
			},
		],
	},
	{
		id: "observability",
		label: "Observability",
		short: "Logs",
		icon: BarChart3,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "see logs or analytics",
			},
			{
				name: "Logs",
				tone: "view",
				permissions: [RolePermissions.ReadLogs],
				can: "read execution logs",
			},
			{
				name: "Full",
				tone: "run",
				permissions: [RolePermissions.ReadLogs, RolePermissions.ReadAnalytics],
				can: "read logs and usage analytics",
			},
		],
	},
	{
		id: "config",
		label: "Configuration",
		short: "Config",
		icon: Settings,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "see app settings",
			},
			{
				name: "View",
				tone: "view",
				permissions: [RolePermissions.ReadConfig],
				can: "read app settings",
			},
			{
				name: "Edit",
				tone: "manage",
				permissions: [RolePermissions.ReadConfig, RolePermissions.WriteConfig],
				can: "change app settings",
			},
		],
	},
	{
		id: "content",
		label: "Content",
		short: "Content",
		icon: BookOpen,
		levels: [
			{
				name: "None",
				tone: "none",
				permissions: [],
				cannot: "open templates, courses or widgets",
			},
			{
				name: "View",
				tone: "view",
				permissions: [
					RolePermissions.ReadTemplates,
					RolePermissions.ReadCourses,
					RolePermissions.ReadWidgets,
				],
				can: "browse templates, courses and widgets",
			},
			{
				name: "Edit",
				tone: "edit",
				permissions: [
					RolePermissions.ReadTemplates,
					RolePermissions.WriteTemplates,
					RolePermissions.ReadCourses,
					RolePermissions.WriteCourses,
					RolePermissions.ReadWidgets,
					RolePermissions.WriteWidgets,
				],
				can: "write templates, courses and widgets",
			},
			{
				name: "Publish",
				tone: "manage",
				permissions: [
					RolePermissions.ReadTemplates,
					RolePermissions.WriteTemplates,
					RolePermissions.ReadCourses,
					RolePermissions.WriteCourses,
					RolePermissions.ReadWidgets,
					RolePermissions.WriteWidgets,
					RolePermissions.WriteRoutes,
				],
				can: "publish content and manage routes",
			},
		],
	},
];

/** Permissions that let a role change something others depend on. */
export const WRITE_PERMISSIONS: RolePermissions[] = [
	RolePermissions.InvokeApi,
	RolePermissions.WriteFiles,
	RolePermissions.WriteDatabase,
	RolePermissions.WriteMeta,
	RolePermissions.WriteBoards,
	RolePermissions.WriteEvents,
	RolePermissions.WriteConfig,
	RolePermissions.WriteTemplates,
	RolePermissions.WriteCourses,
	RolePermissions.WriteWidgets,
	RolePermissions.WriteRoutes,
];

const WRITE_MASK = WRITE_PERMISSIONS.reduce(
	(acc, perm) => acc | perm.toBigInt(),
	0n,
);

function maskOf(permissions: RolePermissions[]): bigint {
	return permissions.reduce((acc, perm) => acc | perm.toBigInt(), 0n);
}

function topLevel(ladder: AccessLadder): AccessLevel {
	return ladder.levels[ladder.levels.length - 1];
}

/** Every permission the ladders can reach — Owner and Admin are not among them. */
export function ladderMask(ladder: AccessLadder): bigint {
	return maskOf(topLevel(ladder).permissions);
}

export const TOTAL_PERMISSION_COUNT =
	ACCESS_LADDERS.reduce(
		(acc, ladder) => acc + topLevel(ladder).permissions.length,
		0,
	) + 2;

export type Elevation = "standard" | "admin" | "owner";

export function elevationOf(permissions: RolePermissions): Elevation {
	if (permissions.contains(RolePermissions.Owner)) return "owner";
	if (permissions.contains(RolePermissions.Admin)) return "admin";
	return "standard";
}

export function applyElevation(
	permissions: RolePermissions,
	elevation: Elevation,
): RolePermissions {
	const base = permissions
		.remove(RolePermissions.Owner)
		.remove(RolePermissions.Admin);
	if (elevation === "admin") return base.insert(RolePermissions.Admin);
	if (elevation === "owner") return base.insert(RolePermissions.Owner);
	return base;
}

/** Index of the matching level, or -1 when the set does not match any level. */
export function levelOf(
	permissions: RolePermissions,
	ladder: AccessLadder,
): number {
	const held = permissions.toBigInt() & ladderMask(ladder);
	return ladder.levels.findIndex((level) => maskOf(level.permissions) === held);
}

/** Level after elevation is applied — Owner and Admin sit at the top of every ladder. */
export function effectiveLevel(
	permissions: RolePermissions,
	ladder: AccessLadder,
): number {
	return elevationOf(permissions) === "standard"
		? levelOf(permissions, ladder)
		: ladder.levels.length - 1;
}

export function applyLevel(
	permissions: RolePermissions,
	ladder: AccessLadder,
	index: number,
): RolePermissions {
	const cleared = permissions.toBigInt() & ~ladderMask(ladder);
	return new RolePermissions(
		cleared | maskOf(ladder.levels[index].permissions),
	);
}

export function levelGrantsWrite(level: AccessLevel): boolean {
	return (maskOf(level.permissions) & WRITE_MASK) !== 0n;
}

export function isWritePermission(permission: RolePermissions): boolean {
	return (permission.toBigInt() & WRITE_MASK) !== 0n;
}

/** Permissions actually in force, counting what Owner and Admin imply. */
export function effectivePermissionCount(permissions: RolePermissions): number {
	const elevation = elevationOf(permissions);
	if (elevation === "owner") return TOTAL_PERMISSION_COUNT;
	if (elevation === "admin") return TOTAL_PERMISSION_COUNT - 1;
	return ACCESS_LADDERS.reduce(
		(acc, ladder) =>
			acc +
			topLevel(ladder).permissions.filter((perm) => permissions.contains(perm))
				.length,
		0,
	);
}

export function writePermissionCount(permissions: RolePermissions): number {
	if (elevationOf(permissions) !== "standard") return WRITE_PERMISSIONS.length;
	return WRITE_PERMISSIONS.filter((perm) => permissions.contains(perm)).length;
}

/**
 * Plain-language account of a standard role. Owner and Admin get their own
 * copy in the component — "can do everything" needs no enumeration.
 */
export function describeAccess(permissions: RolePermissions): {
	can: string[];
	cannot: string[];
} {
	const can: string[] = [];
	const cannot: string[] = [];
	for (const ladder of ACCESS_LADDERS) {
		const index = levelOf(permissions, ladder);
		if (index < 0) {
			can.push(`a custom mix in ${ladder.label.toLowerCase()}`);
			continue;
		}
		const level = ladder.levels[index];
		if (level.can) can.push(level.can);
		else if (level.cannot) cannot.push(level.cannot);
	}
	return { can, cannot };
}

export function joinClauses(clauses: string[]): string {
	if (clauses.length === 0) return "";
	if (clauses.length === 1) return clauses[0];
	return `${clauses.slice(0, -1).join(", ")} and ${clauses[clauses.length - 1]}`;
}

export interface RoleTemplate {
	name: string;
	description: string;
	/** Level index per ladder id. Omitted ladders stay at None. */
	levels?: Record<string, number>;
	elevation?: Elevation;
}

export const ROLE_TEMPLATES: RoleTemplate[] = [
	{
		name: "Viewer",
		description: "Reads the app but changes nothing anywhere.",
		levels: {
			team: 1,
			files: 1,
			boards: 1,
			events: 1,
			observability: 1,
			config: 1,
			content: 1,
		},
	},
	{
		name: "Operator",
		description: "Runs the work day to day without editing it.",
		levels: {
			team: 1,
			files: 1,
			boards: 2,
			events: 2,
			observability: 2,
			config: 1,
			content: 1,
		},
	},
	{
		name: "Editor",
		description: "Builds workflows and content end to end.",
		levels: {
			team: 1,
			files: 2,
			boards: 3,
			events: 3,
			observability: 2,
			config: 1,
			content: 3,
		},
	},
	{
		name: "Administrator",
		description: "Everything except ownership transfer.",
		elevation: "admin",
	},
];

export function permissionsFromTemplate(
	template: RoleTemplate,
): RolePermissions {
	if (template.elevation) {
		return applyElevation(new RolePermissions(), template.elevation);
	}
	return ACCESS_LADDERS.reduce(
		(acc, ladder) => applyLevel(acc, ladder, template.levels?.[ladder.id] ?? 0),
		new RolePermissions(),
	);
}

export function templateLevel(
	template: RoleTemplate,
	ladder: AccessLadder,
): number {
	if (template.elevation) return ladder.levels.length - 1;
	return template.levels?.[ladder.id] ?? 0;
}

/** Escalating fills on the app's primary — intensity tracks consequence. */
export const TONE_STEP_CLASS: Record<LadderTone, string> = {
	none: "text-muted-foreground",
	view: "bg-primary/10 text-foreground",
	run: "bg-primary/25 text-foreground",
	edit: "bg-primary/45 text-foreground",
	manage: "bg-primary text-primary-foreground",
};

export const TONE_GAUGE_CLASS: Record<LadderTone, string> = {
	none: "bg-muted-foreground/25",
	view: "bg-primary/35",
	run: "bg-primary/60",
	edit: "bg-primary/80",
	manage: "bg-primary",
};
