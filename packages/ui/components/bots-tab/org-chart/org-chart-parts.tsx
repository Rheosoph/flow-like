"use client";

import { useTranslation } from "@flow-like/locales";
import { StarIcon } from "lucide-react";
import { useUserIdentity } from "../../../hooks/use-user-lookup";
import type { AskStatus } from "../../../lib/bot-teams/org-model";
import { cn } from "../../../lib/utils";
import type { IOrgBot } from "../../../state/backend-state/bot-team-state";
import { BotAvatar } from "../../bots/bot-avatar";
import { UserAvatar } from "../../ui/user-avatar";
import { useOrgChart } from "./org-chart-context";
export { TeamTile } from "./team-tile";

export function PersonName({
	userId,
	className,
}: Readonly<{ userId: string; className?: string }>) {
	const identity = useUserIdentity(userId);
	return <span className={className}>{identity.label}</span>;
}

export function PersonFace({
	userId,
	size = 24,
}: Readonly<{ userId: string; size?: number }>) {
	const identity = useUserIdentity(userId);
	return (
		<UserAvatar
			avatarUrl={identity.avatarUrl}
			initials={identity.initials}
			label={identity.label}
			size={size}
		/>
	);
}

export function OrgBotAvatar({
	bot,
	size = 24,
}: Readonly<{ bot: IOrgBot; size?: number }>) {
	const { me } = useOrgChart();
	const owner = useUserIdentity(bot.owner_id);
	const badge = Math.max(10, Math.round(size * 0.42));
	return (
		<span
			className="relative inline-flex shrink-0"
			style={{ width: size, height: size }}
		>
			<BotAvatar
				appearance={bot.appearance}
				size={size}
				animated={false}
				decorative
			/>
			{bot.owner_id !== me && (
				<span className="absolute -right-1 -bottom-1 rounded-full ring-2 ring-card">
					<UserAvatar
						avatarUrl={owner.avatarUrl}
						initials={owner.initials}
						label={owner.label}
						size={badge}
					/>
				</span>
			)}
		</span>
	);
}

const ASK_CLASS: Record<Exclude<AskStatus, "self" | "none">, string> = {
	direct: "border-good-line bg-good-bg text-good",
	ok: "border-warning-line bg-warning-bg text-warning",
	via: "border-transparent bg-muted text-muted-foreground",
	blocked: "border-critical-line bg-critical-bg text-critical",
	later: "border-dashed border-border text-muted-foreground",
};

export function AskBadge({
	status,
	className,
}: Readonly<{
	status: Exclude<AskStatus, "self" | "none">;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const label = {
		direct: t("teams.org.ask.direct", "Direct"),
		ok: t("teams.org.ask.ok", "With OK"),
		via: t("teams.org.ask.via", "Via lead"),
		blocked: t("teams.org.ask.blocked", "Never"),
		later: t("teams.org.ask.later", "Later"),
	}[status];
	return (
		<span
			className={cn(
				"inline-flex shrink-0 items-center rounded-full border px-1.5 text-[10px] leading-4 font-medium",
				ASK_CLASS[status],
				className,
			)}
		>
			{label}
		</span>
	);
}

export function LeadStar({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	return (
		<StarIcon
			role="img"
			aria-label={t("teams.org.lead", "Lead")}
			className={cn(
				"size-3 shrink-0 fill-warning-solid text-warning-solid",
				className,
			)}
		/>
	);
}
