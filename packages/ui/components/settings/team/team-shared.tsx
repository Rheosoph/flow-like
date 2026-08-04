"use client";

import type { LucideIcon } from "lucide-react";
import { SearchIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useMemo } from "react";
import { useInfiniteInvoke, useInvoke } from "../../../hooks/use-invoke";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Input } from "../../ui/input";

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
 * Aggregates every access-related count the team page shows above the fold.
 * Every query here is also used by the individual sections, so react-query
 * serves them from one cache entry instead of refetching per section.
 */
export function useTeamOverview(appId: string): ITeamOverview {
	const backend = useBackend();
	const enabled = appId.length > 0;

	const team = useInfiniteInvoke(
		backend.teamState.getTeam,
		backend.teamState,
		[appId],
		50,
		enabled,
	);
	const joinRequests = useInfiniteInvoke(
		backend.teamState.getJoinRequests,
		backend.teamState,
		[appId],
		50,
		enabled,
	);
	const roles = useInvoke(
		backend.roleState.getRoles,
		backend.roleState,
		[appId],
		enabled,
	);
	const links = useInvoke(
		backend.teamState.getInviteLinks,
		backend.teamState,
		[appId],
		enabled,
	);
	const apiKeys = useInvoke(
		backend.apiKeyState.getApiKeys,
		backend.apiKeyState,
		[appId],
		enabled,
	);
	const connections = useInvoke(
		backend.teamState.getAppConnections,
		backend.teamState,
		[appId],
		enabled,
	);

	return useMemo(() => {
		const members = team.data?.pages.flat() ?? [];
		const roleList = roles.data?.[1] ?? [];
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

		const incoming = connections.data?.incoming ?? [];
		const outgoing = connections.data?.outgoing ?? [];
		const pendingAppRequestCount = incoming.filter(
			(connection) => connection.status === "PENDING",
		).length;
		const connectedAppCount =
			incoming.filter((connection) => connection.status === "ACTIVE").length +
			outgoing.filter((connection) => connection.status === "ACTIVE").length;

		const keys = apiKeys.data ?? [];
		const now = Date.now();
		const joinRequestCount = joinRequests.data?.pages.flat().length ?? 0;

		return {
			memberCount: members.length,
			memberCountExact: !team.hasNextPage,
			editorCount,
			viewerCount: Math.max(members.length - editorCount, 0),
			joinRequestCount,
			inviteLinkCount: links.data?.length ?? 0,
			apiKeyCount: keys.length,
			expiredKeyCount: keys.filter(
				(key) => key.valid_until && key.valid_until * 1000 < now,
			).length,
			connectedAppCount,
			pendingAppRequestCount,
			needsReviewCount: joinRequestCount + pendingAppRequestCount,
			isLoading:
				team.isLoading ||
				roles.isLoading ||
				links.isLoading ||
				apiKeys.isLoading ||
				connections.isLoading,
		};
	}, [
		team.data,
		team.hasNextPage,
		team.isLoading,
		joinRequests.data,
		roles.data,
		roles.isLoading,
		links.data,
		links.isLoading,
		apiKeys.data,
		apiKeys.isLoading,
		connections.data,
		connections.isLoading,
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
			? "border-primary/40 bg-primary/5 hover:border-primary hover:bg-primary/10"
			: "border-border/60 hover:border-border hover:bg-muted/40",
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
