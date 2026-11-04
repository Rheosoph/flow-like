"use client";

import { useTranslation } from "@flow-like/locales";
import { ExternalLink, IdCard } from "lucide-react";
import type {
	BotIdentity,
	BotOwnerIdentity,
	UserIdentity,
} from "../../hooks/use-user-lookup";
import { userInitials } from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { Skeleton } from "../ui/skeleton";
import { UserAvatar } from "../ui/user-avatar";
import { ResolvedIdentityAvatar } from "./identity-avatar";

export interface BotIdentityCardProps {
	/** A resolved identity whose `bot` is set; anything else renders nothing. */
	identity: UserIdentity;
	showId?: boolean;
	/** Links the owner to their profile; a bot has no profile page of its own. */
	showProfileLink?: boolean;
}

const SMALL_BADGE = "px-1.5 py-0 text-[10px]";

/**
 * The card behind every bot hover: the bot, then the person it acts for, so a
 * reader sees whose rights an agent used without opening anything.
 */
export function BotIdentityCard({
	identity,
	showId = true,
	showProfileLink = true,
}: Readonly<BotIdentityCardProps>) {
	const { t } = useTranslation("bots");
	const bot = identity.bot;
	if (!bot) return null;

	return (
		<div className="min-w-0" data-slot="bot-identity-card">
			<div className="flex min-w-0 items-start gap-3 border-b bg-muted/30 p-4">
				<ResolvedIdentityAvatar
					identity={identity}
					size={48}
					animated={!bot.deleted}
				/>
				<div className="min-w-0 flex-1">
					<div className="flex min-w-0 items-center gap-1.5">
						<span className="truncate font-semibold" title={identity.label}>
							{identity.label}
						</span>
						<BotKindBadge bot={bot} />
					</div>
					{bot.description && (
						<p className="mt-1 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
							{bot.description}
						</p>
					)}
				</div>
			</div>
			<BotOwnerSection
				owner={bot.owner}
				isOwn={bot.isOwn}
				showProfileLink={showProfileLink}
			/>
			{showId && identity.accountId && (
				<div className="flex min-w-0 items-center justify-between gap-3 border-t px-4 py-2.5 text-xs">
					<span className="inline-flex shrink-0 items-center gap-1.5 text-muted-foreground">
						<IdCard className="h-3.5 w-3.5" />
						{t("identity.botId", "Bot ID")}
					</span>
					<code
						className="min-w-0 truncate rounded bg-muted px-1.5 py-0.5 font-mono"
						title={identity.accountId}
					>
						{identity.accountId}
					</code>
				</div>
			)}
		</div>
	);
}

function BotKindBadge({ bot }: Readonly<{ bot: BotIdentity }>) {
	const { t } = useTranslation("bots");
	if (bot.deleted) {
		return (
			<Badge
				variant="outline"
				className={cn(SMALL_BADGE, "text-muted-foreground")}
			>
				{t("identity.deleted", "Deleted bot")}
			</Badge>
		);
	}
	return (
		<Badge variant="secondary" className={SMALL_BADGE}>
			{bot.system ? t("crew.builtIn", "Built in") : t("identity.bot", "Bot")}
		</Badge>
	);
}

function BotOwnerSection({
	owner,
	isOwn,
	showProfileLink,
}: Readonly<{
	owner: BotOwnerIdentity;
	isOwn: boolean;
	showProfileLink: boolean;
}>) {
	const { t } = useTranslation("bots");
	const you = t("crew.you", "You");
	// Signed out, a local bot's owner is this device's user, who has no account to name.
	const anonymousSelf = isOwn && !owner.accountId;
	const name = anonymousSelf ? you : owner.label;
	const unknown = !owner.accountId && !isOwn;

	return (
		<div className="p-4">
			<div className="mb-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
				{t("identity.actsFor", "Acts for")}
			</div>
			<div className="flex min-w-0 items-start gap-3">
				<UserAvatar
					avatarUrl={owner.avatarUrl}
					initials={
						owner.isPending
							? ""
							: anonymousSelf
								? userInitials(you, "?")
								: owner.initials
					}
					label={name}
					size="md"
				/>
				<div className="min-w-0 flex-1">
					{owner.isPending ? (
						<div className="space-y-1.5 pt-0.5">
							<Skeleton className="h-4 w-28 max-w-full" />
							<Skeleton className="h-3 w-20 max-w-full" />
						</div>
					) : (
						<>
							<div className="flex min-w-0 items-center gap-1.5">
								<span
									className={cn(
										"truncate text-sm font-medium",
										unknown && "font-mono text-muted-foreground",
									)}
									title={name}
								>
									{name}
								</span>
								{isOwn && !anonymousSelf && (
									<Badge variant="outline" className={SMALL_BADGE}>
										{you}
									</Badge>
								)}
							</div>
							{owner.subtitle && (
								<div
									className="truncate text-xs text-muted-foreground"
									title={owner.subtitle}
								>
									{owner.subtitle}
								</div>
							)}
							{showProfileLink && owner.accountId && (
								<a
									href={`/profile?sub=${encodeURIComponent(owner.accountId)}`}
									className="mt-1.5 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
								>
									{t("common:viewProfile", "View profile")}
									<ExternalLink className="h-3 w-3" />
								</a>
							)}
						</>
					)}
				</div>
			</div>
		</div>
	);
}
