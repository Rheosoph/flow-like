"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BellIcon,
	BlocksIcon,
	ClockIcon,
	KeyIcon,
	LinkIcon,
	type LucideIcon,
	ShieldIcon,
	UserPlusIcon,
	UsersIcon,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { AppConnectionManagement } from "./app-connection-management";
import { InviteManagement, InviteUserDialog } from "./invite-managment";
import { TeamJoinManagement } from "./join-management";
import {
	CountPill,
	TEAM_ACTION_GRADIENT,
	type TeamSectionKey,
	useTeamOverview,
} from "./team-shared";
import { TechnicalUserManagement } from "./technical-user-management";
import { UserManagement } from "./user-managements";

interface RailItem {
	key: TeamSectionKey;
	label: string;
	icon: LucideIcon;
	count: number;
	attention?: boolean;
}

export function TeamManagementPage() {
	const { t } = useTranslation("settings");
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const [section, setSection] = useState<TeamSectionKey>("members");
	const overview = useTeamOverview(appId);

	const rail = useMemo<readonly RailItem[][]>(
		() => [
			[
				{
					key: "members",
					label: t('people', 'People'),
					icon: UsersIcon,
					count: overview.memberCount,
				},
				{
					key: "requests",
					label: t('joinRequests', 'Join requests'),
					icon: ClockIcon,
					count: overview.joinRequestCount,
					attention: overview.joinRequestCount > 0,
				},
				{
					key: "invites",
					label: t('invitesLinks', 'Invites & links'),
					icon: LinkIcon,
					count: overview.inviteLinkCount,
				},
			],
			[
				{
					key: "keys",
					label: t('apiKeys', 'API keys'),
					icon: KeyIcon,
					count: overview.apiKeyCount,
				},
				{
					key: "connections",
					label: t('connectedApps', 'Connected apps'),
					icon: BlocksIcon,
					count: overview.connectedAppCount + overview.pendingAppRequestCount,
					attention: overview.pendingAppRequestCount > 0,
				},
			],
		],
		[overview],
	);

	if (!appId) {
		return (
			<div className="p-10 text-center text-muted-foreground">
				{t('noAppSelected', 'No app selected.')}
			</div>
		);
	}

	return (
		<div className="flex flex-col gap-5 pb-8">
			<header className="flex flex-wrap items-start justify-between gap-4">
				<div className="min-w-0">
					<h1 className="text-xl font-semibold tracking-tight">{t('access', 'Access')}</h1>
					<p className="mt-0.5 text-sm text-muted-foreground">
						{t('whoAndWhatCanReachThisApp', 'Who and what can reach this app.')}
					</p>
				</div>
				<div className="flex shrink-0 items-center gap-2">
					<Button variant="outline" asChild>
						<a href={`/library/config/roles?id=${appId}`}>
							<ShieldIcon className="size-4" />
							{t('roles', 'Roles')}
						</a>
					</Button>
					<InviteUserDialog
						appId={appId}
						trigger={
							<Button className={TEAM_ACTION_GRADIENT}>
								<UserPlusIcon className="size-4" />
								{t('invitePeople', 'Invite people')}
							</Button>
						}
					/>
				</div>
			</header>

			<div className="grid grid-cols-2 gap-2.5 lg:grid-cols-4">
				<StatTile
					icon={UsersIcon}
					label="People"
					value={`${overview.memberCount}${overview.memberCountExact ? "" : "+"}`}
					note={t('editorcountCanEditViewercountReadonly', '{{editorCount}} can edit · {{viewerCount}} read-only', { editorCount: overview.editorCount, viewerCount: overview.viewerCount })}
					onClick={() => setSection("members")}
				/>
				<StatTile
					icon={BellIcon}
					label={t('needsReview', 'Needs review')}
					value={overview.needsReviewCount}
					note={
						overview.needsReviewCount === 0
							? t('nothingWaiting', 'Nothing waiting')
							: [
									t('countPeople', {
										defaultValue_one: '{{count}} person',
										defaultValue_other: '{{count}} people',
										count: overview.joinRequestCount,
									}),
									t('countApps', {
										defaultValue_one: '{{count}} App',
										defaultValue_other: '{{count}} Apps',
										count: overview.pendingAppRequestCount,
									}),
								].join(' · ')
					}
					attention={overview.needsReviewCount > 0}
					onClick={() =>
						setSection(
							overview.joinRequestCount > 0 ? "requests" : "connections",
						)
					}
				/>
				<StatTile
					icon={KeyIcon}
					label={t('apiKeys', 'API keys')}
					value={overview.apiKeyCount}
					note={
						overview.expiredKeyCount > 0
							? t('expiredkeycountExpired', '{{expiredKeyCount}} expired', { expiredKeyCount: overview.expiredKeyCount })
							: t('allValid', 'All valid')
					}
					onClick={() => setSection("keys")}
				/>
				<StatTile
					icon={BlocksIcon}
					label={t('connectedApps', 'Connected apps')}
					value={overview.connectedAppCount}
					note={
						overview.pendingAppRequestCount > 0
							? t('pendingapprequestcountAwaitingApproval', '{{pendingAppRequestCount}} awaiting approval', { pendingAppRequestCount: overview.pendingAppRequestCount })
							: t('noPendingRequests', 'No pending requests')
					}
					onClick={() => setSection("connections")}
				/>
			</div>

			<div className="grid items-start gap-6 md:grid-cols-[212px_minmax(0,1fr)]">
				<nav
					aria-label={t('accessSections', 'Access sections')}
					className="flex gap-1 overflow-x-auto md:sticky md:top-0 md:flex-col md:overflow-visible"
				>
					{rail.map((group, index) => (
						<div
							key={group[0].key}
							className="flex gap-1 md:flex-col md:gap-0.5"
						>
							<span
								className={cn(
									"hidden px-2.5 pb-1 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground md:block",
									index === 0 ? "pt-0" : "pt-3",
								)}
							>
								{t('machines', { defaultValue_zero: 'People', defaultValue_other: 'Machines', count: index })}
							</span>
							{group.map((item) => (
								<RailButton
									key={item.key}
									item={item}
									active={section === item.key}
									onSelect={() => setSection(item.key)}
								/>
							))}
						</div>
					))}
				</nav>

				<div className="min-w-0">
					{section === "members" && <UserManagement appId={appId} />}
					{section === "requests" && <TeamJoinManagement appId={appId} />}
					{section === "invites" && <InviteManagement appId={appId} />}
					{section === "keys" && <TechnicalUserManagement appId={appId} />}
					{section === "connections" && (
						<AppConnectionManagement appId={appId} />
					)}
				</div>
			</div>
		</div>
	);
}

function RailButton({
	item,
	active,
	onSelect,
}: Readonly<{ item: RailItem; active: boolean; onSelect: () => void }>) {
	const Icon = item.icon;
	return (
		<button
			type="button"
			onClick={onSelect}
			aria-current={active}
			className={cn(
				"flex shrink-0 items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm transition-colors",
				active
					? "bg-primary/10 font-medium text-primary shadow-[inset_2px_0_0_var(--primary)]"
					: "text-muted-foreground hover:bg-muted hover:text-foreground",
			)}
		>
			<Icon className="size-4 shrink-0" />
			<span className="flex-1 whitespace-nowrap md:whitespace-normal">
				{item.label}
			</span>
			<CountPill
				value={item.count}
				tone={item.attention ? "attention" : "neutral"}
			/>
		</button>
	);
}

function StatTile({
	icon: Icon,
	label,
	value,
	note,
	attention = false,
	onClick,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	value: string | number;
	note: string;
	attention?: boolean;
	onClick: () => void;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			className={cn(
				"flex flex-col items-start gap-0.5 rounded-xl border px-3.5 py-3 text-left transition-colors",
				attention
					? "border-primary/45 bg-primary/5 hover:border-primary hover:bg-primary/10"
					: "border-border/60 bg-card hover:border-border hover:bg-muted/40",
			)}
		>
			<span
				className={cn(
					"flex items-center gap-1.5 text-xs",
					attention ? "text-primary" : "text-muted-foreground",
				)}
			>
				<Icon className="size-3.5" />
				{label}
			</span>
			<span
				className={cn(
					"text-2xl font-semibold tabular-nums tracking-tight",
					attention && "text-primary",
				)}
			>
				{value}
			</span>
			<span className="text-[11px] text-muted-foreground">{note}</span>
		</button>
	);
}
