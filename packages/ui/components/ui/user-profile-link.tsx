"use client";

import { ArrowUpRight, CalendarDays, IdCard, Mail, UserRound } from "lucide-react";
import { useMemo } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IUserLookup } from "../../state/backend-state/types";
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

function deriveInitials(label?: string | null) {
	if (!label) return "??";
	const parts = label.trim().split(/\s+/).filter(Boolean);
	if (parts.length === 0) return "??";
	if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
	return `${parts[0][0] ?? ""}${
		parts[parts.length - 1][0] ?? ""
	}`.toUpperCase();
}

function preferredLabel(
	lookup?: IUserLookup | null,
	props?: Pick<
		UserProfileLinkProps,
		| "name"
		| "email"
		| "username"
		| "preferredUsername"
		| "userId"
		| "fallbackLabel"
	>,
) {
	return (
		lookup?.name ??
		props?.name ??
		lookup?.preferred_username ??
		props?.preferredUsername ??
		lookup?.username ??
		props?.username ??
		lookup?.email ??
		props?.email ??
		props?.userId ??
		props?.fallbackLabel ??
		"Unknown user"
	);
}

function secondaryLabel(
	lookup?: IUserLookup | null,
	props?: Pick<
		UserProfileLinkProps,
		"email" | "username" | "preferredUsername" | "userId"
	>,
) {
	const handle =
		lookup?.preferred_username ??
		props?.preferredUsername ??
		lookup?.username ??
		props?.username;
	if (handle) return `@${handle}`;
	return lookup?.email ?? props?.email ?? props?.userId ?? null;
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
	const backend = useBackend();
	const lookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		userId ? [userId] : ["__noop__"],
		Boolean(userId),
	);

	const label = preferredLabel(lookup.data, {
		name,
		email,
		username,
		preferredUsername,
		userId,
		fallbackLabel,
	});
	const visibleLabel =
		compact && label.length > 22 ? `${label.slice(0, 22)}...` : label;
	const subtitle = secondaryLabel(lookup.data, {
		email,
		username,
		preferredUsername,
		userId,
	});
	const resolvedAvatar = lookup.data?.avatar_url ?? avatarUrl ?? "";
	const resolvedDescription = lookup.data?.description ?? description;
	const resolvedCreatedAt = lookup.data?.created_at ?? createdAt;
	const initials = useMemo(() => deriveInitials(label), [label]);

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

	return (
		<HoverCard openDelay={120} closeDelay={120}>
			<HoverCardTrigger asChild>
				<a
					href={`/profile?sub=${encodeURIComponent(userId)}`}
					className={cn(
						"group inline-flex min-w-0 items-center gap-1.5 rounded-md text-xs font-medium transition-colors hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
						muted ? "text-muted-foreground" : "text-foreground",
						className,
					)}
				>
					{showAvatar && (
						<Avatar className={cn("h-4 w-4", avatarClassName)}>
							<AvatarImage src={resolvedAvatar} alt={label} />
							<AvatarFallback className="text-[8px]">
								{initials}
							</AvatarFallback>
						</Avatar>
					)}
					{lookup.isLoading ? (
						<Skeleton className="h-3 w-16" />
					) : (
						<span className="truncate group-hover:underline">
							{visibleLabel}
						</span>
					)}
				</a>
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
							<a
								href={`/profile?sub=${encodeURIComponent(userId)}`}
								className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
							>
								View profile
								<ArrowUpRight className="h-3 w-3" />
							</a>
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
										Email
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
										Joined
									</span>
									<RelativeTime
										value={resolvedCreatedAt}
										className="text-right"
									/>
								</div>
							) : null}
							<div className="flex items-center justify-between gap-3">
								<span className="inline-flex items-center gap-1.5 text-muted-foreground">
									<IdCard className="h-3.5 w-3.5" />
									User ID
								</span>
								<code className="max-w-44 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
									{userId}
								</code>
							</div>
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
