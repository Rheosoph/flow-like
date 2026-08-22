"use client";

import { useTranslation } from "@flow-like/locales";
import { CalendarDays, ExternalLink, IdCard, Mail } from "lucide-react";
import type { ReactNode } from "react";
import {
	type UserIdentity,
	useUserIdentity,
} from "../../hooks/use-user-lookup";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { HoverCard, HoverCardContent, HoverCardTrigger } from "./hover-card";
import { RelativeTime } from "./relative-time";
import { Skeleton } from "./skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

export type UserAvatarSize = "xs" | "sm" | "md" | "lg" | "xl" | "2xl";

const AVATAR_SIZE_CLASSES: Record<string, string> = {
	xs: "h-5 w-5",
	sm: "h-6 w-6",
	md: "h-8 w-8",
	lg: "h-10 w-10",
	xl: "h-14 w-14",
	"2xl": "h-16 w-16",
};

const AVATAR_FALLBACK_CLASSES: Record<string, string> = {
	xs: "text-[9px]",
	sm: "text-[10px]",
	md: "text-xs",
	lg: "text-sm",
	xl: "text-base",
	"2xl": "text-lg",
};

/**
 * The avatar every identity surface renders, so a user is the same size and
 * shape in a table cell, a hover card and an a2ui page.
 */
export function UserAvatar({
	avatarUrl,
	initials,
	label,
	size = "md",
	className,
}: Readonly<{
	avatarUrl?: string | null;
	initials: string;
	label: string;
	size?: string;
	className?: string;
}>) {
	return (
		<Avatar
			className={cn(
				AVATAR_SIZE_CLASSES[size] ?? AVATAR_SIZE_CLASSES.md,
				"shrink-0",
				className,
			)}
		>
			<AvatarImage src={avatarUrl ?? ""} alt={label} />
			<AvatarFallback
				className={AVATAR_FALLBACK_CLASSES[size] ?? AVATAR_FALLBACK_CLASSES.md}
			>
				{initials}
			</AvatarFallback>
		</Avatar>
	);
}

export function UserDetailRow({
	icon,
	label,
	children,
}: Readonly<{
	icon: ReactNode;
	label: string;
	children: ReactNode;
}>) {
	return (
		<div className="grid min-w-0 grid-cols-[6.5rem_minmax(0,1fr)] items-start gap-3 text-sm">
			<span className="inline-flex min-w-0 items-center gap-1.5 text-muted-foreground">
				{icon}
				<span className="truncate">{label}</span>
			</span>
			<div className="min-w-0 text-right">{children}</div>
		</div>
	);
}

export interface UserProfileHoverContentProps {
	userId?: string;
	label: string;
	subtitle?: string | null;
	avatarUrl?: string | null;
	initials: string;
	description?: string | null;
	createdAt?: string | number | null;
	email?: string | null;
	showEmail?: boolean;
	showUserId?: boolean;
	showProfileLink?: boolean;
	isLoading?: boolean;
}

/**
 * The card behind every user hover — a2ui profiles, inline profile links and
 * database cells all open this one, so an identity never reads differently
 * depending on where it was hovered.
 */
export function UserProfileHoverContent({
	userId,
	label,
	subtitle,
	avatarUrl,
	initials,
	description,
	createdAt,
	email,
	showEmail = true,
	showUserId = true,
	showProfileLink = true,
	isLoading = false,
}: Readonly<UserProfileHoverContentProps>) {
	const { t } = useTranslation("common");

	return (
		<div className="min-w-0">
			<div className="border-b bg-muted/30 p-4">
				<div className="flex min-w-0 items-start gap-3">
					<UserAvatar
						avatarUrl={avatarUrl}
						initials={initials}
						label={label}
						size="lg"
					/>
					<div className="min-w-0 flex-1">
						<div className="truncate font-semibold" title={label}>
							{label}
						</div>
						{subtitle && (
							<div
								className="truncate text-xs text-muted-foreground"
								title={subtitle}
							>
								{subtitle}
							</div>
						)}
						{showProfileLink && userId && (
							<a
								href={`/profile?sub=${encodeURIComponent(userId)}`}
								className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
							>
								{t("viewProfile", "View profile")}
								<ExternalLink className="h-3 w-3" />
							</a>
						)}
					</div>
				</div>
			</div>
			<div className="grid gap-3 p-4 text-sm">
				{isLoading ? (
					<div className="space-y-2">
						<Skeleton className="h-4 w-3/4" />
						<Skeleton className="h-4 w-1/2" />
						<Skeleton className="h-4 w-2/3" />
					</div>
				) : (
					<>
						{showEmail && email ? (
							<UserDetailRow
								icon={<Mail className="h-3.5 w-3.5" />}
								label={t("email", "Email")}
							>
								<span className="block truncate" title={email}>
									{email}
								</span>
							</UserDetailRow>
						) : null}
						{createdAt ? (
							<UserDetailRow
								icon={<CalendarDays className="h-3.5 w-3.5" />}
								label={t("joined", "Joined")}
							>
								<RelativeTime value={createdAt} />
							</UserDetailRow>
						) : null}
						{showUserId && userId ? (
							<UserDetailRow
								icon={<IdCard className="h-3.5 w-3.5" />}
								label={t("userId", "User ID")}
							>
								<code className="block truncate rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
									{userId}
								</code>
							</UserDetailRow>
						) : null}
						{description ? (
							<p className="line-clamp-3 rounded-md bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
								{description}
							</p>
						) : null}
					</>
				)}
			</div>
		</div>
	);
}

/**
 * Why a stored id is showing as itself. A lookup that never completed knows
 * nothing about the account, so it must not claim the account does not exist.
 */
function unresolvedReason(
	identity: UserIdentity,
	t: (key: string, defaultValue: string) => string,
): string {
	return identity.isError
		? t("couldNotResolveThisAccount", "Could not resolve this account")
		: t("noAccountMatchesThisId", "No account matches this id");
}

const INLINE_TAG_BASE =
	"inline-flex min-w-0 max-w-full items-center gap-1.5 text-left";

export interface UserInlineTagProps {
	/** The stored account id, exactly as the record holds it. */
	userId: string;
	/** Applied to the trigger, so a host can match its own cell chrome. */
	className?: string;
	avatarClassName?: string;
	/** Makes the tag clickable — a cell uses this to open its own detail view. */
	onClick?: () => void;
	align?: "start" | "center" | "end";
}

/**
 * A stored account id read as the person it points at.
 *
 * Resolution is best-effort by design: until the directory answers the tag holds
 * its shape with a skeleton, and if it never answers — deleted account, foreign
 * tenant, offline — it falls back to the id it actually stores, which is what
 * someone reading raw records needs to see anyway.
 */
export function UserInlineTag({
	userId,
	className,
	avatarClassName,
	onClick,
	align = "start",
}: Readonly<UserInlineTagProps>) {
	const { t } = useTranslation("common");
	const identity = useUserIdentity(userId);

	if (identity.isPending) {
		// The skeleton is sized like a resolved tag, so the column it sits in does
		// not resize under the reader when the lookup lands.
		const pendingContent = (
			<>
				<Skeleton className={cn("h-4 w-4 rounded-full", avatarClassName)} />
				<Skeleton className="h-3 w-28 max-w-full" />
			</>
		);
		const pendingClassName = cn(INLINE_TAG_BASE, className);
		return onClick ? (
			<button type="button" onClick={onClick} className={pendingClassName}>
				{pendingContent}
			</button>
		) : (
			<span className={pendingClassName}>{pendingContent}</span>
		);
	}

	if (!identity.isResolved) {
		const fallbackClassName = cn(
			INLINE_TAG_BASE,
			"truncate font-mono text-muted-foreground",
			className,
		);
		return (
			<Tooltip>
				<TooltipTrigger asChild>
					{onClick ? (
						<button
							type="button"
							onClick={onClick}
							className={fallbackClassName}
						>
							{userId}
						</button>
					) : (
						<span className={fallbackClassName}>{userId}</span>
					)}
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					{unresolvedReason(identity, t)}
				</TooltipContent>
			</Tooltip>
		);
	}

	const triggerClassName = cn(INLINE_TAG_BASE, className);
	const triggerContent = (
		<>
			<UserAvatar
				avatarUrl={identity.avatarUrl}
				initials={identity.initials}
				label={identity.label}
				size="xs"
				className={cn("h-4 w-4", avatarClassName)}
			/>
			<span className="min-w-0 truncate">{identity.label}</span>
		</>
	);

	return (
		<HoverCard openDelay={200} closeDelay={120}>
			<HoverCardTrigger asChild>
				{onClick ? (
					<button type="button" onClick={onClick} className={triggerClassName}>
						{triggerContent}
					</button>
				) : (
					<span className={triggerClassName}>{triggerContent}</span>
				)}
			</HoverCardTrigger>
			<HoverCardContent align={align} className="w-80 p-0">
				<UserProfileHoverContent
					userId={identity.accountId}
					label={identity.label}
					subtitle={identity.subtitle}
					avatarUrl={identity.avatarUrl}
					initials={identity.initials}
					description={identity.user?.description}
					createdAt={identity.user?.created_at}
					email={identity.user?.email}
				/>
			</HoverCardContent>
		</HoverCard>
	);
}

/** The same identity, opened out for a detail pane rather than a cell. */
export function UserIdentityCard({
	userId,
	className,
}: Readonly<{ userId: string; className?: string }>) {
	const { t } = useTranslation("common");
	const identity = useUserIdentity(userId);

	return (
		<div className={cn("space-y-3", className)}>
			{identity.isResolved ? (
				<div className="max-w-sm overflow-hidden rounded-md border">
					<UserProfileHoverContent
						userId={identity.accountId}
						label={identity.label}
						subtitle={identity.subtitle}
						avatarUrl={identity.avatarUrl}
						initials={identity.initials}
						description={identity.user?.description}
						createdAt={identity.user?.created_at}
						email={identity.user?.email}
						isLoading={identity.isPending}
					/>
				</div>
			) : (
				<p className="text-sm text-muted-foreground">
					{identity.isPending
						? t("loading", "Loading…")
						: unresolvedReason(identity, t)}
				</p>
			)}
			<code className="block break-all rounded-md bg-muted px-3 py-2 font-mono text-xs">
				{userId}
			</code>
		</div>
	);
}
