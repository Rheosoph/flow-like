import {
	BarChart3,
	BookOpen,
	Code2,
	Crown,
	Database,
	Eye,
	FileText,
	type LucideIcon,
	Pencil,
	Play,
	Route,
	ScrollText,
	Settings,
	Shield,
	SquareKanban,
	Users,
	Zap,
} from "lucide-react";
import { RolePermissions } from "../../../lib/permission/role-permission";

export interface PermissionEntry {
	permission: RolePermissions;
	label: string;
	description: string;
	icon: LucideIcon;
}

export interface PermissionGroup {
	id: string;
	label: string;
	description: string;
	icon: LucideIcon;
	permissions: PermissionEntry[];
}

export const PERMISSION_GROUPS: PermissionGroup[] = [
	{
		id: "system",
		label: "System",
		description: "Top-level system access",
		icon: Shield,
		permissions: [
			{
				permission: RolePermissions.Owner,
				label: "Owner",
				description: "Full ownership — only one per app",
				icon: Crown,
			},
			{
				permission: RolePermissions.Admin,
				label: "Administrator",
				description: "Full administrative access",
				icon: Shield,
			},
		],
	},
	{
		id: "team",
		label: "Team & Access",
		description: "Manage team members and API access",
		icon: Users,
		permissions: [
			{
				permission: RolePermissions.ReadTeam,
				label: "View Team",
				description: "See team members and their roles",
				icon: Users,
			},
			{
				permission: RolePermissions.ReadRoles,
				label: "View Roles",
				description: "See role definitions and permissions",
				icon: Eye,
			},
			{
				permission: RolePermissions.InvokeApi,
				label: "Invoke API",
				description: "Call external and internal APIs",
				icon: Code2,
			},
		],
	},
	{
		id: "files",
		label: "Files & Data",
		description: "Read and write files and metadata",
		icon: FileText,
		permissions: [
			{
				permission: RolePermissions.ReadFiles,
				label: "Read Files",
				description: "Download and view files",
				icon: Eye,
			},
			{
				permission: RolePermissions.WriteFiles,
				label: "Write Files",
				description: "Upload, update, and delete files",
				icon: Pencil,
			},
			{
				permission: RolePermissions.ReadDatabase,
				label: "Read Databases",
				description:
					"Query and list databases without file access — ideal for connected apps",
				icon: Database,
			},
			{
				permission: RolePermissions.WriteDatabase,
				label: "Write Databases",
				description: "Insert, update, and manage databases without file access",
				icon: Database,
			},
			{
				permission: RolePermissions.WriteMeta,
				label: "Write Metadata",
				description: "Edit file and entity metadata",
				icon: Database,
			},
		],
	},
	{
		id: "boards",
		label: "Workflows",
		description: "Flow board access and execution",
		icon: SquareKanban,
		permissions: [
			{
				permission: RolePermissions.ReadBoards,
				label: "View Workflows",
				description: "Open and inspect flow boards",
				icon: Eye,
			},
			{
				permission: RolePermissions.ExecuteBoards,
				label: "Run Workflows",
				description: "Execute flow boards",
				icon: Play,
			},
			{
				permission: RolePermissions.WriteBoards,
				label: "Edit Workflows",
				description: "Create, modify, and delete flow boards",
				icon: Pencil,
			},
		],
	},
	{
		id: "events",
		label: "Events",
		description: "Event pipeline access and execution",
		icon: Zap,
		permissions: [
			{
				permission: RolePermissions.ListEvents,
				label: "List Events",
				description: "Browse available events",
				icon: ScrollText,
			},
			{
				permission: RolePermissions.ReadEvents,
				label: "View Events",
				description: "Inspect event details and payloads",
				icon: Eye,
			},
			{
				permission: RolePermissions.ExecuteEvents,
				label: "Trigger Events",
				description: "Execute and trigger events",
				icon: Play,
			},
			{
				permission: RolePermissions.WriteEvents,
				label: "Manage Events",
				description: "Create, modify, and delete events",
				icon: Pencil,
			},
		],
	},
	{
		id: "observability",
		label: "Observability",
		description: "Logs, analytics, and monitoring",
		icon: BarChart3,
		permissions: [
			{
				permission: RolePermissions.ReadLogs,
				label: "View Logs",
				description: "Access application and execution logs",
				icon: ScrollText,
			},
			{
				permission: RolePermissions.ReadAnalytics,
				label: "View Analytics",
				description: "Access dashboards and usage statistics",
				icon: BarChart3,
			},
		],
	},
	{
		id: "config",
		label: "Configuration",
		description: "App settings and system configuration",
		icon: Settings,
		permissions: [
			{
				permission: RolePermissions.ReadConfig,
				label: "View Config",
				description: "Read application configuration",
				icon: Eye,
			},
			{
				permission: RolePermissions.WriteConfig,
				label: "Edit Config",
				description: "Modify application configuration",
				icon: Pencil,
			},
		],
	},
	{
		id: "content",
		label: "Content",
		description: "Templates, courses, widgets, and routes",
		icon: BookOpen,
		permissions: [
			{
				permission: RolePermissions.ReadTemplates,
				label: "View Templates",
				description: "Browse and inspect templates",
				icon: Eye,
			},
			{
				permission: RolePermissions.WriteTemplates,
				label: "Edit Templates",
				description: "Create and modify templates",
				icon: Pencil,
			},
			{
				permission: RolePermissions.ReadCourses,
				label: "View Courses",
				description: "Browse and access courses",
				icon: Eye,
			},
			{
				permission: RolePermissions.WriteCourses,
				label: "Edit Courses",
				description: "Create and modify courses",
				icon: Pencil,
			},
			{
				permission: RolePermissions.ReadWidgets,
				label: "View Widgets",
				description: "Browse and inspect widgets",
				icon: Eye,
			},
			{
				permission: RolePermissions.WriteWidgets,
				label: "Edit Widgets",
				description: "Create and modify widgets",
				icon: Pencil,
			},
			{
				permission: RolePermissions.WriteRoutes,
				label: "Manage Routes",
				description: "Create and modify application routes",
				icon: Route,
			},
		],
	},
];

export const ALL_PERMISSIONS = PERMISSION_GROUPS.flatMap((g) =>
	g.permissions.map((p) => p.permission),
);

export function countGroupPermissions(
	group: PermissionGroup,
	current: RolePermissions,
): { active: number; total: number } {
	const total = group.permissions.length;
	const active = group.permissions.filter((p) =>
		current.contains(p.permission),
	).length;
	return { active, total };
}

export function countTotalPermissions(current: RolePermissions): {
	active: number;
	total: number;
} {
	const total = ALL_PERMISSIONS.length;
	const active = ALL_PERMISSIONS.filter((p) => current.contains(p)).length;
	return { active, total };
}

export function getPermissionLabel(perm: RolePermissions): string | undefined {
	for (const group of PERMISSION_GROUPS) {
		for (const entry of group.permissions) {
			if (entry.permission.equals(perm)) return entry.label;
		}
	}
	return undefined;
}
