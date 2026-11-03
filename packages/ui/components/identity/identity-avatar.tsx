"use client";

import {
	type BotIdentity,
	type BotOwnerIdentity,
	type UserIdentity,
	useUserIdentity,
} from "../../hooks/use-user-lookup";
import { cn } from "../../lib/utils";
import { BotAvatar } from "../bots/bot-avatar";
import { Skeleton } from "../ui/skeleton";
import { UserAvatar } from "../ui/user-avatar";

export interface IdentityAvatarProps {
	/** A user sub or a bot id. */
	subject: string;
	/** Pixels; 16 to 128. */
	size?: number;
	className?: string;
	/** Bots hold still unless asked, because these sit in dense lists. */
	animated?: boolean;
}

export interface ResolvedIdentityAvatarProps {
	identity: UserIdentity;
	size?: number;
	className?: string;
	animated?: boolean;
	title?: string;
}

const OWNER_BADGE_RATIO = 0.4;
/** Small bots keep a legible badge instead of losing the owner. */
const OWNER_BADGE_MIN_PX = 10;
/** Narrower badges fit one initial, not two. */
const OWNER_BADGE_TWO_INITIALS = 20;
/** Narrower badges are a plain disc: no initial is readable. */
const OWNER_BADGE_ONE_INITIAL = 12;
const BOT_MIN_SIZE = 16;

/** "Scout, Felix's bot": a bot is never named without whose it is. */
export function identityAccessibleName(identity: UserIdentity): string {
	return identity.bot && identity.subtitle
		? `${identity.label}, ${identity.subtitle}`
		: identity.label;
}

/**
 * A stored sub as a face. A person is their avatar; a bot is its own look with
 * its owner's picture in the corner, so everyone sees that an agent acted for
 * that person.
 */
export function IdentityAvatar({
	subject,
	size = 32,
	className,
	animated = false,
}: Readonly<IdentityAvatarProps>) {
	const identity = useUserIdentity(subject);

	if (identity.isPending) {
		return (
			<Skeleton
				className={cn("shrink-0 rounded-full", className)}
				style={{ width: size, height: size }}
			/>
		);
	}

	return (
		<ResolvedIdentityAvatar
			identity={identity}
			size={size}
			className={className}
			animated={animated}
			title={identityAccessibleName(identity)}
		/>
	);
}

/** `IdentityAvatar` for a surface that already resolved the identity. */
export function ResolvedIdentityAvatar({
	identity,
	size = 32,
	className,
	animated = false,
	title,
}: Readonly<ResolvedIdentityAvatarProps>) {
	if (!identity.bot) {
		return (
			<UserAvatar
				avatarUrl={identity.avatarUrl}
				initials={identity.initials}
				label={identity.label}
				size={size}
				className={className}
				title={title}
			/>
		);
	}

	return (
		<BotIdentityAvatar
			bot={identity.bot}
			name={identityAccessibleName(identity)}
			size={Math.max(BOT_MIN_SIZE, size)}
			className={className}
			animated={animated}
			title={title}
		/>
	);
}

function BotIdentityAvatar({
	bot,
	name,
	size,
	className,
	animated,
	title,
}: Readonly<{
	bot: BotIdentity;
	name: string;
	size: number;
	className?: string;
	animated: boolean;
	title?: string;
}>) {
	return (
		<span
			role="img"
			aria-label={name}
			title={title}
			data-bot-deleted={bot.deleted || undefined}
			className={cn(
				"relative inline-flex shrink-0 [&_.fl-bot-badges]:hidden",
				bot.deleted && "opacity-60 grayscale",
				className,
			)}
			style={{ width: size, height: size }}
		>
			<BotAvatar
				appearance={bot.appearance}
				size={size}
				animated={animated}
				decorative
			/>
			<OwnerBadge owner={bot.owner} size={size} />
		</span>
	);
}

function OwnerBadge({
	owner,
	size,
}: Readonly<{ owner: BotOwnerIdentity; size: number }>) {
	const badge = Math.max(
		OWNER_BADGE_MIN_PX,
		Math.round(size * OWNER_BADGE_RATIO),
	);
	const ring = Math.max(1, Math.round(size / 32));
	let initials = owner.initials;
	if (owner.isPending || badge < OWNER_BADGE_ONE_INITIAL) initials = "";
	else if (badge < OWNER_BADGE_TWO_INITIALS) initials = initials.slice(0, 1);

	return (
		<UserAvatar
			avatarUrl={owner.avatarUrl}
			initials={initials}
			label={owner.label}
			size={badge}
			className="absolute border-background"
			style={{ right: -ring, bottom: -ring, borderWidth: ring }}
		/>
	);
}
