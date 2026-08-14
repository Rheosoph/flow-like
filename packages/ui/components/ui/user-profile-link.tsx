"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpRight,
	CalendarDays,
	IdCard,
	Mail,
	UserRound,
} from "lucide-react";
import { useMemo } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import {
	type UserDisplayLike,
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IUserLookup } from "../../state/backend-state/types";
import { resolveAccountId } from "../../state/backend-state/user-state";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { HoverCard, HoverCardContent, HoverCardTrigger } from "./hover-card";
import { RelativeTime } from "./relative-time";
import { Skeleton } from "./skeleton";

export interface UserProfileLinkProps {
	userId?: string | null;
	name?: string | null;
	email?: string | null;
	username?: string | null;
	preferredUsername?: string | null;
	avatarUrl?: string | null;
	description?: string | null;
	createdAt?: string | number | null;
	fallbackLabel?: string;
	className?: string;
	avatarClassName?: string;
	showAvatar?: boolean;
	compact?: boolean;
	muted?: boolean;
}

/** Lookup wins per field, props fill the gaps. */
function mergeUser(
	lookup?: IUserLookup | null,
	props?: Pick<
		UserProfileLinkProps,
		"name" | "email" | "username" | "preferredUsername" | "avatarUrl"
	>,
): UserDisplayLike {
	return {
		name: lookup?.name ?? props?.name,
		preferred_username: lookup?.preferred_username ?? props?.preferredUsername,
		username: lookup?.username ?? props?.username,
		email: lookup?.email ?? props?.email,
		avatar_url: lookup?.avatar_url ?? props?.avatarUrl,
	};
}

export function UserProfileLink({
	userId,
	name,
	email,
	username,
	preferredUsername,
	avatarUrl,
	description,
	createdAt,
	fallbackLabel = "Unknown user",
	className,
	avatarClassName,
	showAvatar = true,
	compact = false,
	muted = false,
}: Readonly<UserProfileLinkProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const lookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		userId ? [userId] : ["__noop__"],
		Boolean(userId),
	);

	// A "local" sub means the executing user was not authenticated: the lookup
	// resolves it to the current user, so use the account id it returns.
	const resolvedUserId = resolveAccountId(lookup.data?.id, userId);

	const user = mergeUser(lookup.data, {
		name,
		email,
		username,
		preferredUsername,
		avatarUrl,
	});
	const label = userDisplayName(user, resolvedUserId ?? fallbackLabel);
	const visibleLabel =
		compact && label.length > 22 ? `${label.slice(0, 22)}...` : label;
	const subtitle = userSecondaryLabel(user) ?? resolvedUserId ?? null;
	const resolvedAvatar = userAvatarUrl(user) ?? "";
	const resolvedDescription = lookup.data?.description ?? description;
	const resolvedCreatedAt = lookup.data?.created_at ?? createdAt;
	const initials = useMemo(() => userInitials(label, "??"), [label]);

	if (!userId) {
		return (
			<span
				className={cn(
					"inline-flex min-w-0 items-center gap-1.5 text-xs",
					muted && "text-muted-foreground",
					className,
				)}
			>
				{showAvatar && <UserRound className="h-3.5 w-3.5 shrink-0" />}
				<span className="truncate">{fallbackLabel}</span>
			</span>
		);
	}

	const triggerClassName = cn(
		t('groupInlineflexMinw0ItemscenterGap15RoundedmdTextxsFontmediumTransitioncolorsHovertextprimaryFocusvisibleoutlinenoneFocusvisiblering2FocusvisibleringringFocusvisibleringoffset2', 'group inline-flex min-w-0 items-center gap-1.5 rounded-md text-xs font-medium transition-colors hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2'),
		muted ? "text-muted-foreground" : "text-foreground",
		className,
	);

	const triggerContent = (
		<>
			{showAvatar && (
				<Avatar className={cn("h-4 w-4", avatarClassName)}>
					<AvatarImage src={resolvedAvatar} alt={label} />
					<AvatarFallback className="text-[8px]">{initials}</AvatarFallback>
				</Avatar>
			)}
			{lookup.isLoading ? (
				<Skeleton className="h-3 w-16" />
			) : (
				<span className="truncate group-hover:underline">{visibleLabel}</span>
			)}
		</>
	);

	return (
		<HoverCard openDelay={120} closeDelay={120}>
			<HoverCardTrigger asChild>
				{resolvedUserId ? (
					<a
						href={`/profile?sub=${encodeURIComponent(resolvedUserId)}`}
						className={triggerClassName}
					>
						{triggerContent}
					</a>
				) : (
					<span className={triggerClassName}>{triggerContent}</span>
				)}
			</HoverCardTrigger>
			<HoverCardContent align="start" className="w-80 p-0">
				<div className="border-b bg-muted/30 p-4">
					<div className="flex items-start gap-3">
						<Avatar className="h-12 w-12">
							<AvatarImage src={resolvedAvatar} alt={label} />
							<AvatarFallback>{initials}</AvatarFallback>
						</Avatar>
						<div className="min-w-0 flex-1">
							<div className="truncate font-semibold">{label}</div>
							{subtitle && (
								<div className="truncate text-xs text-muted-foreground">
									{subtitle}
								</div>
							)}
							{resolvedUserId && (
								<a
									href={`/profile?sub=${encodeURIComponent(resolvedUserId)}`}
									className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
								>
									{t('viewProfile', 'View profile')}
									<ArrowUpRight className="h-3 w-3" />
								</a>
							)}
						</div>
					</div>
				</div>
				<div className="grid gap-3 p-4 text-sm">
					{lookup.isLoading && (
						<div className="space-y-2">
							<Skeleton className="h-4 w-3/4" />
							<Skeleton className="h-4 w-1/2" />
							<Skeleton className="h-4 w-2/3" />
						</div>
					)}
					{!lookup.isLoading && (
						<>
							{email || lookup.data?.email ? (
								<div className="flex items-center justify-between gap-3">
									<span className="inline-flex items-center gap-1.5 text-muted-foreground">
										<Mail className="h-3.5 w-3.5" />
										{t('email', 'Email')}
									</span>
									<span className="truncate text-right">
										{lookup.data?.email ?? email}
									</span>
								</div>
							) : null}
							{resolvedCreatedAt ? (
								<div className="flex items-center justify-between gap-3">
									<span className="inline-flex items-center gap-1.5 text-muted-foreground">
										<CalendarDays className="h-3.5 w-3.5" />
										{t('joined', 'Joined')}
									</span>
									<RelativeTime
										value={resolvedCreatedAt}
										className="text-right"
									/>
								</div>
							) : null}
							{resolvedUserId ? (
								<div className="flex items-center justify-between gap-3">
									<span className="inline-flex items-center gap-1.5 text-muted-foreground">
										<IdCard className="h-3.5 w-3.5" />
										{t('userId', 'User ID')}
									</span>
									<code className="max-w-44 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
										{resolvedUserId}
									</code>
								</div>
							) : null}
							{resolvedDescription ? (
								<p className="rounded-md bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
									{resolvedDescription}
								</p>
							) : null}
						</>
					)}
				</div>
			</HoverCardContent>
		</HoverCard>
	);
}
