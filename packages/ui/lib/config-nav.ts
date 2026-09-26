import {
	BadgeEuroIcon,
	ChartAreaIcon,
	CloudOffIcon,
	CogIcon,
	CopyIcon,
	CrownIcon,
	DatabaseIcon,
	FolderClosedIcon,
	GlobeIcon,
	LayersIcon,
	LayoutGridIcon,
	type LucideProps,
	PackageIcon,
	PaletteIcon,
	ScrollTextIcon,
	SendIcon,
	ServerIcon,
	SparklesIcon,
	SquarePenIcon,
	UserIcon,
	UsersRoundIcon,
	WorkflowIcon,
} from "lucide-react";
import type { ForwardRefExoticComponent, RefAttributes } from "react";
import { IAppVisibility } from "../types";
import { RolePermissions } from "./permission/role-permission";

export type ConfigNavIcon = ForwardRefExoticComponent<
	Omit<LucideProps, "ref"> & RefAttributes<SVGSVGElement>
>;

export interface INavigationItem {
	href: string;
	label: string;
	icon: ConfigNavIcon;
	description: string;
	group: string;
	visibilities?: IAppVisibility[];
	requiresPaid?: boolean;
	disabled?: boolean;
	devOnly?: boolean;
	/**
	 * Visibilities where the section stays in the nav but is locked: hiding it
	 * outright reads as "this feature does not exist". Clicking a locked row
	 * offers the visibility change that unlocks it.
	 */
	lockedVisibilities?: IAppVisibility[];
	/** Copy for the unlock dialog. */
	lockedReason?: string;
	/** Visibility the unlock dialog switches to. */
	unlockVisibility?: IAppVisibility;
	/**
	 * Permissions that let the caller open this section, any one of which is
	 * enough — the same `ensure_any_permission!` shape the server uses. Omitted
	 * where every member may read the section.
	 */
	permissions?: RolePermissions[];
	/** Host feature the section needs; hosts without it never list the section. */
	hostCapability?: string;
}

/**
 * Why a section cannot be opened. The two kinds are deliberately different
 * affordances, not two shades of the same one: a visibility lock is a door the
 * owner can open from here, a permission lock is someone else's decision and
 * offers nothing to click but an explanation.
 */
export type SectionLock =
	| {
			kind: "visibility";
			reason: string;
			/** Visibility the unlock dialog switches to. */
			target: IAppVisibility;
	  }
	| {
			kind: "permission";
			reason: string;
			/** Any one of these would open the section. */
			missing: RolePermissions[];
	  };

export interface INavigationItemState extends INavigationItem {
	lock?: SectionLock;
}

/** Labels are built per render so a language switch relabels the config nav. */
export function buildNavigationItems(
	t: (key: string, defaultValue: string) => string,
): INavigationItem[] {
	const groups = {
		general: t("general", "General"),
		build: t("build", "Build"),
		data: t("data", "Data"),
		collaborate: t("collaborate", "Collaborate"),
		insights: t("insights", "Insights"),
	};

	return [
		{
			href: "/library/config",
			label: t("dashboard", "Dashboard"),
			icon: SquarePenIcon,
			description: t(
				"overviewStatsAndGettingStarted",
				"Overview, stats, and getting started",
			),
			group: groups.general,
		},
		{
			href: "/library/config/setup",
			label: t("setup", "Setup"),
			icon: CogIcon,
			description: t(
				"whatThisAppNeedsFromYouPlusItsAppwideDefaults",
				"What this app needs from you, plus its app-wide defaults",
			),
			group: groups.general,
			permissions: [RolePermissions.ReadBoards],
		},
		{
			href: "/library/config/appearance",
			label: t("appearance", "Appearance"),
			icon: PaletteIcon,
			description: t(
				"oneStylesheetForEveryPageInThisApp",
				"One stylesheet for every page in this app",
			),
			group: groups.general,
			permissions: [RolePermissions.Owner],
		},
		{
			href: "/library/config/flows",
			label: t("flows", "Flows"),
			icon: WorkflowIcon,
			description: t(
				"businessLogicAndWorkflowDefinitions",
				"Business logic and workflow definitions",
			),
			group: groups.build,
			devOnly: true,
			permissions: [RolePermissions.ReadBoards],
		},
		{
			href: "/library/config/pages",
			label: t("events", "Events"),
			icon: SparklesIcon,
			description: t(
				"eventsPagesAndPathbasedNavigation",
				"Events, pages, and path-based navigation",
			),
			group: groups.build,
			devOnly: true,
			permissions: [RolePermissions.ListEvents],
		},
		{
			href: "/library/config/templates",
			label: t("templates", "Templates"),
			icon: CopyIcon,
			description: t("reusableFlowTemplates", "Reusable Flow templates"),
			group: groups.build,
			devOnly: true,
			permissions: [RolePermissions.ReadTemplates],
		},
		{
			href: "/library/config/widgets",
			label: t("widgets", "Widgets"),
			icon: LayoutGridIcon,
			description: t(
				"reusableUiComponentsAndWidgets",
				"Reusable UI components and widgets",
			),
			group: groups.build,
			devOnly: true,
			permissions: [RolePermissions.ReadWidgets],
		},
		{
			href: "/library/config/devices",
			label: t("projectDevices", "Devices"),
			icon: ServerIcon,
			description: t(
				"projectDevicesDescription",
				"Deploy this project to standalone devices and inspect replica health",
			),
			group: groups.build,
			permissions: [RolePermissions.ReadBoards],
		},
		{
			href: "/library/config/storage",
			label: t("storage", "Storage"),
			icon: FolderClosedIcon,
			description: t(
				"dataStorageAndFileManagement",
				"Data storage and file management",
			),
			group: groups.data,
			permissions: [RolePermissions.ReadFiles],
		},
		{
			href: "/library/config/user-storage",
			label: t("userStorage", "User Storage"),
			icon: UserIcon,
			description: t(
				"browseAndSearchYourPrivateAppFiles",
				"Browse and search your private app files",
			),
			group: groups.data,
			devOnly: true,
			permissions: [RolePermissions.ReadFiles],
		},
		{
			href: "/library/config/explore",
			label: t("dataStudio", "Data Studio"),
			icon: DatabaseIcon,
			description: t(
				"modelExploreOperateAndShareProjectData",
				"Model, explore, operate, and share project data",
			),
			group: groups.data,
			devOnly: true,
			permissions: [RolePermissions.ReadFiles, RolePermissions.ReadDatabase],
		},
		{
			href: "/library/config/offline",
			label: t("offlineAccess", "Offline access"),
			icon: CloudOffIcon,
			description: t(
				"offlineAccessNavDescription",
				"Keep tables available on this device and sync changes made while offline",
			),
			group: groups.data,
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
				IAppVisibility.Private,
			],
			permissions: [RolePermissions.ExecuteEvents],
			hostCapability: "offlineWrites",
		},
		{
			href: "/library/config/packages",
			label: t("packages", "Packages"),
			icon: PackageIcon,
			description: t(
				"manageWasmPackagesForThisApp",
				"Manage WASM packages for this app",
			),
			group: groups.data,
			devOnly: true,
			permissions: [RolePermissions.ReadBoards],
		},
		{
			href: "/library/config/team",
			label: t("team", "Team"),
			icon: UsersRoundIcon,
			description: t(
				"manageTeamMembersAndPermissions",
				"Manage team members and permissions",
			),
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
			],
			lockedVisibilities: [IAppVisibility.Private],
			lockedReason: t(
				"aPrivateProjectIsSyncedToYourAccountOnlySwitchToPrototypeToInviteCollaboratorsAssignRolesAndShareALink",
				"A private project is synced to your account only. Switch to Prototype to invite collaborators, assign roles and share a link.",
			),
			unlockVisibility: IAppVisibility.Prototype,
			group: groups.collaborate,
			permissions: [RolePermissions.ReadTeam],
		},
		{
			href: "/library/config/suites",
			label: t("suites", "Suites"),
			icon: LayersIcon,
			description: t(
				"bundleThisAppWithRelatedAppsIntoOneStoreListing",
				"Bundle this app with related apps into one store listing",
			),
			// A suite is presentation, not membership — private apps curate them too.
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
				IAppVisibility.Private,
			],
			group: groups.collaborate,
			devOnly: true,
			permissions: [RolePermissions.ReadTeam],
		},
		{
			href: "/library/config/roles",
			label: t("roles", "Roles"),
			icon: CrownIcon,
			description: t(
				"defineUserRolesAndAccessLevels",
				"Define user roles and access levels",
			),
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
			],
			group: groups.collaborate,
			devOnly: true,
			permissions: [RolePermissions.ReadRoles],
		},
		{
			href: "/library/config/sales",
			label: t("monetization", "Monetization"),
			icon: BadgeEuroIcon,
			description: t(
				"monetizationNavDescription",
				"Store sales, flow payments, pricing and payment settings",
			),
			// Flow payments work at any visibility; only a local-only app has no
			// server to take money through.
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
				IAppVisibility.Private,
			],
			group: groups.insights,
			permissions: [RolePermissions.Owner],
		},
		{
			href: "/library/config/analytics",
			label: t("analytics", "Analytics"),
			icon: ChartAreaIcon,
			description: t(
				"performanceMetricsAndInsights",
				"Performance metrics and insights",
			),
			group: groups.insights,
			devOnly: true,
			permissions: [RolePermissions.ReadAnalytics],
		},
		{
			href: "/library/config/audit",
			label: t("audit:auditTrail", "Audit trail"),
			icon: ScrollTextIcon,
			description: t(
				"audit:navDescription",
				"Tamper-evident record of every change, and its export",
			),
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
				IAppVisibility.Private,
			],
			group: groups.insights,
			permissions: [RolePermissions.Owner],
		},
		{
			href: "/library/config/endpoints",
			label: t("endpoints", "Endpoints"),
			icon: GlobeIcon,
			description: t(
				"apiEndpointsAndIntegrations",
				"API endpoints and integrations",
			),
			group: groups.insights,
			devOnly: true,
		},
		{
			href: "/library/config/publication",
			label: t("publication", "Publication"),
			icon: SendIcon,
			description: t(
				"trackPublicationReviewStatusAndAuditorFeedback",
				"Track publication review status and auditor feedback",
			),
			group: groups.insights,
			devOnly: true,
			permissions: [RolePermissions.Admin],
		},
	];
}

export function isConfigRouteActive(
	itemHref: string,
	currentRoute: string,
): boolean {
	if (itemHref === "/library/config") {
		return currentRoute === "/library/config";
	}
	return currentRoute.startsWith(itemHref);
}

export interface ResolveNavOptions {
	visibility: IAppVisibility;
	developerMode: boolean;
	isPaid: boolean;
	/** Falls back to "allowed" while the caller's role is still unknown. */
	can: (...permissions: RolePermissions[]) => boolean;
	permissionLockReason: (item: INavigationItem) => string;
	/** Capabilities of the host app; sections needing a missing one are left out. */
	hostCapabilities?: ReadonlySet<string>;
}

/**
 * The single filter both config layouts run. A section survives when its
 * visibility, paywall and developer-mode gates pass; it survives *locked* when
 * a gate is one the user could lift here (visibility) or one worth naming
 * rather than hiding (permission).
 *
 * Hiding a section the account simply cannot read would be defensible, but it
 * reads as "this app has no Team", which sends people hunting for a feature
 * that is right there behind a role they can ask for.
 */
export function resolveNavigationItems(
	items: INavigationItem[],
	options: ResolveNavOptions,
): INavigationItemState[] {
	const {
		visibility,
		developerMode,
		isPaid,
		can,
		permissionLockReason,
		hostCapabilities,
	} = options;

	return items
		.filter(
			(item) =>
				(!item.hostCapability ||
					hostCapabilities?.has(item.hostCapability) === true) &&
				(!item.devOnly || developerMode) &&
				(!item.visibilities ||
					item.visibilities.includes(visibility) ||
					item.lockedVisibilities?.includes(visibility)) &&
				(!item.requiresPaid || isPaid),
		)
		.map((item) => {
			// Visibility wins: on a private app the Team tab is not a permission
			// problem, and offering "ask an admin" would be advice to nobody.
			if (item.visibilities && !item.visibilities.includes(visibility)) {
				return {
					...item,
					lock: {
						kind: "visibility",
						reason: item.lockedReason ?? item.description,
						target: item.unlockVisibility ?? IAppVisibility.Prototype,
					},
				} satisfies INavigationItemState;
			}

			if (item.permissions && !can(...item.permissions)) {
				return {
					...item,
					lock: {
						kind: "permission",
						reason: permissionLockReason(item),
						missing: item.permissions,
					},
				} satisfies INavigationItemState;
			}

			return item;
		});
}
