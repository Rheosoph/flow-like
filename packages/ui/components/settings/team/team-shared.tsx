"use client";

import { useTranslation } from "@flow-like/locales";
import type { LucideIcon } from "lucide-react";
import { SearchIcon, TriangleAlertIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useMemo } from "react";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInfiniteInvoke, useInvoke } from "../../../hooks/use-invoke";
import { apiErrorMessage } from "../../../lib/api-error";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { asArray } from "../../../lib/response-shape";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Input } from "../../ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

export const TEAM_SECTION_KEYS = [
	"members",
	"requests",
	"invites",
	"keys",
	"connections",
] as const;

export type TeamSectionKey = (typeof TEAM_SECTION_KEYS)[number];

export type TeamTone = "neutral" | "attention" | "success" | "danger" | "owner";

/**
 * The one emphasised action treatment: the page CTA and the primary action of a
 * section. Everything else is a plain `outline` button.
 */
export const TEAM_ACTION_GRADIENT =
	"bg-linear-to-r from-primary to-tertiary hover:from-primary/85 hover:to-tertiary/85";

/**
 * What the signed-in account may do on this project, resolved once per section
 * and threaded down instead of re-asked in every leaf.
 *
 * The three levels the access surface mixes: `ReadTeam` (members, connections,
 * suites), `ReadRoles` (role names) and `Admin` (join requests, invites, API
 * keys, every write). Owner-only reads sit behind {@link ITeamAccess.canOwn}.
 */
export interface ITeamAccess {
	/** The role is still resolving — render a skeleton, not a denial. */
	isLoading: boolean;
	/** The role resolved; the flags below are real server bits. */
	known: boolean;
	roleName?: string;
	canReadTeam: boolean;
	canReadRoles: boolean;
	canAdminister: boolean;
	/** Passes an `Owner` check — `Admin` satisfies it, exactly as the server does. */
	canOwn: boolean;
	/** Ready-made explanations for a control this role may not use. */
	adminReason: string;
	ownerReason: string;
}

/**
 * Reads the overview could not make. A denied read is not a zero, so anything
 * derived from one has to render as unknown rather than as a count.
 */
export interface ITeamOverviewGaps {
	/** Member list — `ReadTeam`. */
	members: boolean;
	/** Role names, and with them the editor/viewer split — `ReadRoles`. */
	roles: boolean;
	/** Join requests — `Admin`. */
	joinRequests: boolean;
	/** Invite links — `Admin`. */
	inviteLinks: boolean;
	/** API keys — `Admin`. */
	apiKeys: boolean;
	/** Connected apps — `ReadTeam`. */
	connections: boolean;
}

export interface ITeamOverview {
	/** Members loaded so far. `memberCountExact` says whether more pages remain. */
	memberCount: number;
	memberCountExact: boolean;
	editorCount: number;
	viewerCount: number;
	joinRequestCount: number;
	inviteLinkCount: number;
	apiKeyCount: number;
	expiredKeyCount: number;
	connectedAppCount: number;
	pendingAppRequestCount: number;
	/** Join requests plus incoming app access requests — everything awaiting a decision. */
	needsReviewCount: number;
	isLoading: boolean;
	access: ITeamAccess;
	/** Every count whose source is listed here is unknown, not zero. */
	unavailable: ITeamOverviewGaps;
	/**
	 * The subset of {@link ITeamOverview.unavailable} the role itself explains.
	 * The rest failed to load, which is a different sentence: telling an admin
	 * whose request timed out that their role is too low is its own lie.
	 */
	denied: ITeamOverviewGaps;
}

const WRITE_PERMISSIONS = [
	RolePermissions.Owner,
	RolePermissions.Admin,
	RolePermissions.WriteBoards,
	RolePermissions.WriteConfig,
	RolePermissions.WriteFiles,
	RolePermissions.WriteMeta,
];

/**
 * Resolves what this account may do on the project once, for the whole
 * section. The underlying role request is shared by react-query, so calling
 * this at the top of each section costs one request for the page.
 */
export function useTeamAccess(appId: string): ITeamAccess {
	const { t } = useTranslation("settings");
	const permissions = useAppPermissions(appId);

	return useMemo(
		() => ({
			isLoading: permissions.isLoading,
			known: permissions.known,
			roleName: permissions.roleName,
			canReadTeam: permissions.can(RolePermissions.ReadTeam),
			canReadRoles: permissions.can(RolePermissions.ReadRoles),
			canAdminister: permissions.can(RolePermissions.Admin),
			canOwn: permissions.can(RolePermissions.Owner),
			adminReason: t(
				"onlyProjectAdminsCanChangeThisAskAnOwnerOrAdminToUpdateYourRole",
				"Only project admins can change this. Ask an owner or admin to update your role.",
			),
			ownerReason: t(
				"onlyTheProjectOwnerCanChangeThis",
				"Only the project owner can change this.",
			),
		}),
		[permissions, t],
	);
}

/**
 * Wraps a control this role may not use so the reason is discoverable.
 *
 * A disabled button emits no pointer events at all, so the tooltip hangs on a
 * wrapper and the locked control is taken out of the hit test entirely —
 * otherwise the wrapper never sees the hover that should explain the lock.
 */
export function TeamActionLock({
	locked,
	reason,
	children,
	className,
}: Readonly<{
	locked: boolean;
	reason: string;
	children: ReactNode;
	className?: string;
}>) {
	if (!locked) return <>{children}</>;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span
					className={cn(
						"inline-flex cursor-not-allowed *:pointer-events-none",
						className,
					)}
				>
					{children}
				</span>
			</TooltipTrigger>
			<TooltipContent>{reason}</TooltipContent>
		</Tooltip>
	);
}

/**
 * A read that failed rather than one that was refused.
 *
 * `PermissionNotice` names a missing permission, which is the wrong diagnosis
 * for a dropped connection or a project this device cannot reach — and it
 * sends the reader to an admin who has nothing to grant. This keeps the same
 * shape and says what the server actually said.
 */
export function TeamReadError({
	title,
	error,
	className,
}: Readonly<{
	title: string;
	error: unknown;
	className?: string;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div
			className={cn(
				"flex items-start gap-3 rounded-lg border bg-muted/30 p-3 text-sm",
				className,
			)}
		>
			<TriangleAlertIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
			<div className="min-w-0 flex-1 space-y-1">
				<p className="font-medium text-foreground">{title}</p>
				<p className="text-xs text-muted-foreground">
					{apiErrorMessage(
						error,
						t(
							"thisCouldNotBeLoadedRightNow",
							"This could not be loaded right now. Check your connection and try again.",
						),
					)}
				</p>
			</div>
		</div>
	);
}

/**
 * Aggregates every access-related count the team page shows above the fold.
 * Every query here is also used by the individual sections, so react-query
 * serves them from one cache entry instead of refetching per section.
 *
 * Each read fires only when the role may make it: a denial would otherwise
 * come back as a 403 and land in the tiles as a confident zero.
 */
export function useTeamOverview(appId: string): ITeamOverview {
	const backend = useBackend();
	const access = useTeamAccess(appId);
	const enabled = appId.length > 0 && !access.isLoading;

	const team = useInfiniteInvoke(
		backend.teamState.getTeam,
		backend.teamState,
		[appId],
		50,
		enabled && access.canReadTeam,
	);
	const joinRequests = useInfiniteInvoke(
		backend.teamState.getJoinRequests,
		backend.teamState,
		[appId],
		50,
		enabled && access.canAdminister,
	);
	const roles = useInvoke(
		backend.roleState.getRoles,
		backend.roleState,
		[appId],
		enabled && access.canReadRoles,
	);
	const links = useInvoke(
		backend.teamState.getInviteLinks,
		backend.teamState,
		[appId],
		enabled && access.canAdminister,
	);
	const apiKeys = useInvoke(
		backend.apiKeyState.getApiKeys,
		backend.apiKeyState,
		[appId],
		enabled && access.canAdminister,
	);
	const connections = useInvoke(
		backend.teamState.getAppConnections,
		backend.teamState,
		[appId],
		enabled && access.canReadTeam,
	);

	return useMemo(() => {
		const members = team.data?.pages.flatMap((page) => asArray(page)) ?? [];
		const roleList = asArray(roles.data?.[1]);
		const writableRoleIds = new Set(
			roleList
				.filter((role) => {
					const permission = new RolePermissions(BigInt(role.permissions));
					return WRITE_PERMISSIONS.some((flag) => permission.contains(flag));
				})
				.map((role) => role.id),
		);
		const editorCount = members.filter((member) =>
			writableRoleIds.has(member.role_id),
		).length;

		const incoming = asArray(connections.data?.incoming);
		const outgoing = asArray(connections.data?.outgoing);
		const pendingAppRequestCount = incoming.filter(
			(connection) => connection.status === "PENDING",
		).length;
		const connectedAppCount =
			incoming.filter((connection) => connection.status === "ACTIVE").length +
			outgoing.filter((connection) => connection.status === "ACTIVE").length;

		const keys = asArray(apiKeys.data);
		const now = Date.now();
		const joinRequestCount =
			joinRequests.data?.pages.flatMap((page) => asArray(page)).length ?? 0;

		return {
			memberCount: members.length,
			memberCountExact: !team.hasNextPage,
			editorCount,
			viewerCount: Math.max(members.length - editorCount, 0),
			joinRequestCount,
			inviteLinkCount: asArray(links.data).length,
			apiKeyCount: keys.length,
			expiredKeyCount: keys.filter(
				(key) => key.valid_until && key.valid_until * 1000 < now,
			).length,
			connectedAppCount,
			pendingAppRequestCount,
			needsReviewCount: joinRequestCount + pendingAppRequestCount,
			isLoading:
				access.isLoading ||
				team.isLoading ||
				roles.isLoading ||
				links.isLoading ||
				apiKeys.isLoading ||
				connections.isLoading,
			access,
			unavailable: {
				members: !access.canReadTeam || team.isError,
				roles: !access.canReadRoles || roles.isError,
				joinRequests: !access.canAdminister || joinRequests.isError,
				inviteLinks: !access.canAdminister || links.isError,
				apiKeys: !access.canAdminister || apiKeys.isError,
				connections: !access.canReadTeam || connections.isError,
			},
			denied: {
				members: !access.canReadTeam,
				roles: !access.canReadRoles,
				joinRequests: !access.canAdminister,
				inviteLinks: !access.canAdminister,
				apiKeys: !access.canAdminister,
				connections: !access.canReadTeam,
			},
		};
	}, [
		access,
		team.data,
		team.hasNextPage,
		team.isLoading,
		team.isError,
		joinRequests.data,
		joinRequests.isError,
		roles.data,
		roles.isLoading,
		roles.isError,
		links.data,
		links.isLoading,
		links.isError,
		apiKeys.data,
		apiKeys.isLoading,
		apiKeys.isError,
		connections.data,
		connections.isLoading,
		connections.isError,
	]);
}

/**
 * Vertical rhythm for one block inside a section pane. Sections stack with
 * `space-y-8`; nothing wraps itself in a Card — the pane is the surface.
 */
export function TeamSection({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<section className={cn("flex flex-col gap-3", className)}>
			{children}
		</section>
	);
}

export function SectionHeading({
	icon: Icon,
	title,
	description,
	count,
	countTone = "neutral",
	actions,
}: Readonly<{
	icon: LucideIcon;
	title: string;
	description?: string;
	count?: number;
	countTone?: "neutral" | "attention";
	actions?: ReactNode;
}>) {
	return (
		<div className="flex items-start justify-between gap-4">
			<div className="min-w-0">
				<h3 className="flex items-center gap-2 text-[15px] font-semibold tracking-tight">
					<Icon className="size-4 text-muted-foreground" />
					{title}
					{typeof count === "number" && (
						<CountPill value={count} tone={countTone} />
					)}
				</h3>
				{description && (
					<p className="mt-0.5 max-w-[62ch] text-xs text-muted-foreground">
						{description}
					</p>
				)}
			</div>
			{actions && (
				<div className="flex shrink-0 items-center gap-2">{actions}</div>
			)}
		</div>
	);
}

export function CountPill({
	value,
	tone = "neutral",
	className,
}: Readonly<{
	value: number;
	tone?: "neutral" | "attention";
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 text-[11px] font-semibold tabular-nums",
				tone === "attention"
					? "bg-primary text-primary-foreground"
					: "bg-muted text-muted-foreground",
				className,
			)}
		>
			{value}
		</span>
	);
}

const CHIP_TONES: Record<TeamTone, string> = {
	neutral: "border-border bg-muted text-muted-foreground",
	owner: "border-primary/35 bg-primary/10 text-primary",
	attention: "border-primary/35 bg-primary/10 text-primary",
	success:
		"border-emerald-600/30 bg-emerald-600/10 text-emerald-700 dark:border-emerald-400/30 dark:text-emerald-400",
	danger: "border-destructive/30 bg-destructive/10 text-destructive",
};

export function StatusChip({
	tone = "neutral",
	icon: Icon,
	pip = false,
	children,
	className,
}: Readonly<{
	tone?: TeamTone;
	icon?: LucideIcon;
	pip?: boolean;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex h-5.25 items-center gap-1.5 rounded-full border px-2 text-[11px] font-medium",
				CHIP_TONES[tone],
				className,
			)}
		>
			{pip && <span className="size-1.5 rounded-full bg-current" />}
			{Icon && <Icon className="size-3" />}
			{children}
		</span>
	);
}

/**
 * The one row shell every list in the team page uses. Keeping it a class
 * factory rather than a component lets each section keep its own markup while
 * the surface, radius and hover behaviour stay identical everywhere.
 */
export function teamRowClass(
	options: Readonly<{
		attention?: boolean;
		muted?: boolean;
		align?: "center" | "start";
	}> = {},
): string {
	const { attention = false, muted = false, align = "center" } = options;
	return cn(
		"group/row flex gap-3 rounded-xl border bg-card px-3 py-2.5 transition-colors",
		align === "center" ? "items-center" : "items-start",
		attention
			? `border-primary/40 bg-primary/5 hover:border-primary hover:bg-primary/10`
			: `border-border/60 hover:border-border hover:bg-muted/40`,
		muted && "opacity-70",
	);
}

export const TEAM_ROW_TITLE =
	"flex flex-wrap items-center gap-2 text-sm font-medium tracking-tight";
export const TEAM_ROW_META =
	"mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs text-muted-foreground";
export const TEAM_ROW_HANDLE = "text-xs font-normal text-muted-foreground";
/** One-line secondary text under a row title — descriptions, purposes. */
export const TEAM_ROW_DESCRIPTION =
	"mt-0.5 truncate text-xs text-muted-foreground";

/** Leading square for rows that have no avatar — invite links, API keys. */
export function TeamRowIcon({
	icon: Icon,
	className,
}: Readonly<{ icon: LucideIcon; className?: string }>) {
	return (
		<div
			className={cn(
				"flex size-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted text-muted-foreground",
				className,
			)}
		>
			<Icon className="size-4" />
		</div>
	);
}

/** Free text the requester attached to a pending row. */
export function TeamRowNote({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="mt-2 rounded-lg border border-border/60 bg-muted/40 px-3 py-2 text-xs leading-relaxed text-muted-foreground">
			{children}
		</p>
	);
}

/** Row controls stay quiet until the row is hovered or focused. */
export function TeamRowActions({
	children,
	always = false,
	className,
}: Readonly<{ children: ReactNode; always?: boolean; className?: string }>) {
	return (
		<div
			className={cn(
				"flex shrink-0 items-center gap-1.5 transition-opacity",
				!always &&
					"opacity-50 group-hover/row:opacity-100 group-focus-within/row:opacity-100",
				className,
			)}
		>
			{children}
		</div>
	);
}

export function TeamSearchInput({
	value,
	onChange,
	placeholder,
	className,
}: Readonly<{
	value: string;
	onChange: (value: string) => void;
	placeholder: string;
	className?: string;
}>) {
	return (
		<div className={cn("relative min-w-45 flex-1", className)}>
			<SearchIcon className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
			<Input
				value={value}
				onChange={(event) => onChange(event.target.value)}
				placeholder={placeholder}
				className="h-9 pl-9"
			/>
		</div>
	);
}

export function TeamToolbar({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<div className={cn("flex flex-wrap items-center gap-2", className)}>
			{children}
		</div>
	);
}

/** Small print under a list — counts, hints, protocol notes. */
export function TeamHint({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<p className={cn("text-xs text-muted-foreground", className)}>{children}</p>
	);
}

/** Inline advisory strip, e.g. "1 key has expired". */
export function TeamCallout({
	icon: Icon,
	tone = "neutral",
	children,
}: Readonly<{
	icon: LucideIcon;
	tone?: "neutral" | "attention";
	children: ReactNode;
}>) {
	return (
		<div
			className={cn(
				"flex items-start gap-2.5 rounded-xl border px-3 py-2.5 text-xs",
				tone === "attention"
					? "border-primary/35 bg-primary/5 text-foreground"
					: "border-border/60 bg-card text-muted-foreground",
			)}
		>
			<Icon
				className={cn(
					"mt-px size-4 shrink-0",
					tone === "attention" ? "text-primary" : "text-muted-foreground",
				)}
			/>
			<div className="min-w-0">{children}</div>
		</div>
	);
}
