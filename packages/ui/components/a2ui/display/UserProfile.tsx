"use client";

import { useTranslation } from "@flow-like/locales";
import { AtSign, ExternalLink, IdCard, Mail, UserRound } from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import {
	userAvatarUrl,
	userDisplayName,
	userHandle,
	userInitials,
	userSecondaryLabel,
} from "../../../lib/user-display";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type { IUserLookup } from "../../../state/backend-state/types";
import { resolveAccountId } from "../../../state/backend-state/user-state";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../../ui/hover-card";
import { Skeleton } from "../../ui/skeleton";
import { UserAvatar, UserProfileHoverContent } from "../../ui/user-identity";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, UserProfileComponent } from "../types";

type UserProfileVariant = "avatar" | "chip" | "row" | "detailed" | "card";
type StringLikeValue =
	| BoundValue
	| string
	| number
	| boolean
	| null
	| undefined;

const PROFILE_VARIANTS = new Set<UserProfileVariant>([
	"avatar",
	"chip",
	"row",
	"detailed",
	"card",
]);

function cleanString(value: unknown): string | undefined {
	if (value === null || value === undefined) return undefined;
	const stringValue = String(value).trim();
	return stringValue.length > 0 ? stringValue : undefined;
}

function resolveValue(
	value: StringLikeValue,
	resolve: (value: BoundValue, fallback?: unknown) => unknown,
): unknown {
	if (value === null || value === undefined) return undefined;
	if (typeof value !== "object") return value;
	return resolve(value);
}

function resolveString(
	value: StringLikeValue,
	resolve: (value: BoundValue, fallback?: unknown) => unknown,
	fallback?: string,
): string | undefined {
	return cleanString(resolveValue(value, resolve)) ?? fallback;
}

function resolveBool(
	value: StringLikeValue,
	resolve: (value: BoundValue, fallback?: unknown) => unknown,
): boolean | undefined {
	const resolved = resolveValue(value, resolve);
	if (typeof resolved === "boolean") return resolved;
	if (typeof resolved === "string") {
		if (resolved.toLowerCase() === "true") return true;
		if (resolved.toLowerCase() === "false") return false;
	}
	return undefined;
}

function normalizeVariant(value: string | undefined): UserProfileVariant {
	if (value && PROFILE_VARIANTS.has(value as UserProfileVariant)) {
		return value as UserProfileVariant;
	}
	return "row";
}

function secondaryLabel(
	lookup: IUserLookup | null | undefined,
	userId: string | undefined,
	showEmail: boolean,
) {
	const handle = userHandle(lookup);
	if (handle) return `@${handle}`;
	if (showEmail) {
		const secondary = userSecondaryLabel(lookup);
		if (secondary) return secondary;
	}
	return userId ?? null;
}

function avatarSizeForVariant(
	variant: UserProfileVariant,
	configuredSize: string | undefined,
) {
	if (configuredSize) return configuredSize;
	if (variant === "avatar") return "md";
	if (variant === "chip") return "sm";
	if (variant === "card") return "xl";
	if (variant === "detailed") return "lg";
	return "md";
}

export function A2UIUserProfile({
	elementRef,
	component,
	style,
}: ComponentProps<UserProfileComponent>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const { resolve } = useData();
	const userId = resolveString(
		component.value as StringLikeValue,
		resolve,
		undefined,
	);
	const variant = normalizeVariant(
		resolveString(component.variant as StringLikeValue, resolve),
	);
	const configuredSize = resolveString(
		component.avatarSize as StringLikeValue,
		resolve,
	);
	const fallbackLabel =
		resolveString(component.fallbackLabel as StringLikeValue, resolve) ??
		"Unknown user";
	const showEmail =
		resolveBool(component.showEmail as StringLikeValue, resolve) ?? true;
	const showDescription =
		resolveBool(component.showDescription as StringLikeValue, resolve) ?? true;
	const explicitShowUserId = resolveBool(
		component.showUserId as StringLikeValue,
		resolve,
	);
	const showUserId =
		explicitShowUserId ?? (variant === "card" || variant === "detailed");
	const showProfileLink =
		resolveBool(component.showProfileLink as StringLikeValue, resolve) ?? true;
	const showHover =
		resolveBool(component.showHover as StringLikeValue, resolve) ??
		variant !== "card";
	const muted =
		resolveBool(component.muted as StringLikeValue, resolve) ?? false;
	const avatarSize = avatarSizeForVariant(variant, configuredSize);

	const lookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		userId ? [userId] : ["__noop__"],
		Boolean(userId),
	);

	// A "local" sub means the executing user was not authenticated: the lookup
	// resolves it to the current user, so use the account id it returns.
	const resolvedUserId = resolveAccountId(lookup.data?.id, userId);

	const label = userDisplayName(lookup.data, resolvedUserId ?? fallbackLabel);
	const subtitle = secondaryLabel(lookup.data, resolvedUserId, showEmail);
	const avatarUrl = userAvatarUrl(lookup.data) ?? "";
	const description = showDescription ? lookup.data?.description : undefined;
	const createdAt = lookup.data?.created_at;
	const email = lookup.data?.email;
	const initials = useMemo(() => userInitials(label, "??"), [label]);
	const rootStyle = resolveInlineStyle(style);
	const rootClassName = resolveStyle(style);

	const missingUser = !userId;
	const disabledLabel = missingUser ? fallbackLabel : label;

	const avatar = (
		<UserAvatar
			avatarUrl={avatarUrl}
			initials={initials}
			label={disabledLabel}
			size={avatarSize}
		/>
	);

	let content: ReactNode;

	if (variant === "avatar") {
		content = (
			<span
				ref={elementRef}
				className={cn(
					"inline-flex min-w-0 items-center justify-center",
					muted && "opacity-75",
					rootClassName,
				)}
				style={rootStyle}
				title={disabledLabel}
			>
				{missingUser ? (
					<span className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-muted text-muted-foreground">
						<UserRound className="h-4 w-4" />
					</span>
				) : (
					avatar
				)}
			</span>
		);
	} else if (variant === "chip") {
		content = (
			<span
				ref={elementRef}
				className={cn(
					"inline-flex max-w-full items-center gap-1.5 rounded-full border bg-background px-1.5 py-1 text-xs shadow-sm",
					muted ? "text-muted-foreground" : "text-foreground",
					rootClassName,
				)}
				style={rootStyle}
			>
				{missingUser ? <UserRound className="h-3.5 w-3.5 shrink-0" /> : avatar}
				<span className="min-w-0 truncate" title={disabledLabel}>
					{lookup.isLoading && userId ? (
						<Skeleton className="h-3 w-16" />
					) : (
						disabledLabel
					)}
				</span>
			</span>
		);
	} else if (variant === "card") {
		content = (
			<div
				ref={elementRef}
				className={cn(
					"w-full max-w-sm rounded-lg border bg-card p-4 text-card-foreground shadow-sm",
					rootClassName,
				)}
				style={rootStyle}
			>
				<div className="flex min-w-0 items-start gap-4">
					{missingUser ? (
						<span className="inline-flex h-14 w-14 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
							<UserRound className="h-6 w-6" />
						</span>
					) : (
						avatar
					)}
					<div className="min-w-0 flex-1">
						{lookup.isLoading && userId ? (
							<div className="space-y-2 pt-1">
								<Skeleton className="h-5 w-3/4" />
								<Skeleton className="h-4 w-1/2" />
							</div>
						) : (
							<>
								<div className="truncate text-base font-semibold" title={label}>
									{disabledLabel}
								</div>
								{subtitle && (
									<div
										className="truncate text-sm text-muted-foreground"
										title={subtitle}
									>
										{subtitle}
									</div>
								)}
							</>
						)}
					</div>
				</div>
				{description ? (
					<p className="mt-4 line-clamp-3 text-sm leading-relaxed text-muted-foreground">
						{description}
					</p>
				) : null}
				{!missingUser && !lookup.isLoading && (
					<div className="mt-4 grid min-w-0 gap-2 border-t pt-3 text-xs">
						{showEmail && email ? (
							<div className="flex min-w-0 items-center gap-2 text-muted-foreground">
								<Mail className="h-3.5 w-3.5 shrink-0" />
								<span className="truncate" title={email}>
									{email}
								</span>
							</div>
						) : null}
						{showUserId && resolvedUserId ? (
							<div className="flex min-w-0 items-center gap-2 text-muted-foreground">
								<IdCard className="h-3.5 w-3.5 shrink-0" />
								<code
									className="min-w-0 truncate font-mono"
									title={resolvedUserId}
								>
									{resolvedUserId}
								</code>
							</div>
						) : null}
						{showProfileLink && resolvedUserId ? (
							<a
								href={`/profile?sub=${encodeURIComponent(resolvedUserId)}`}
								className="inline-flex min-w-0 items-center gap-1 font-medium text-primary hover:underline"
							>
								<span className="truncate">
									{t("viewProfile", "View profile")}
								</span>
								<ExternalLink className="h-3 w-3 shrink-0" />
							</a>
						) : null}
					</div>
				)}
			</div>
		);
	} else {
		const isDetailed = variant === "detailed";
		const rowDescription = description ?? subtitle;

		content = (
			<div
				ref={elementRef}
				className={cn(
					"flex min-w-0 max-w-full items-center gap-3 rounded-lg",
					isDetailed ? "border bg-card p-3 shadow-sm" : "p-1",
					muted ? "text-muted-foreground" : "text-foreground",
					rootClassName,
				)}
				style={rootStyle}
			>
				{missingUser ? (
					<span className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
						<UserRound className="h-4 w-4" />
					</span>
				) : (
					avatar
				)}
				<div className="min-w-0 flex-1">
					{lookup.isLoading && userId ? (
						<div className="space-y-1.5">
							<Skeleton className="h-4 w-32 max-w-full" />
							{isDetailed ? <Skeleton className="h-3 w-48 max-w-full" /> : null}
						</div>
					) : (
						<>
							<div
								className={cn(
									"truncate font-medium",
									isDetailed ? "text-sm" : "text-sm",
								)}
								title={disabledLabel}
							>
								{disabledLabel}
							</div>
							{rowDescription && (
								<div
									className={cn(
										"text-xs text-muted-foreground",
										isDetailed ? "line-clamp-2" : "truncate",
									)}
									title={rowDescription}
								>
									{rowDescription}
								</div>
							)}
							{isDetailed && !description && subtitle && (
								<div className="mt-1 inline-flex max-w-full items-center gap-1 text-xs text-muted-foreground">
									<AtSign className="h-3 w-3 shrink-0" />
									<span className="truncate">{subtitle.replace(/^@/, "")}</span>
								</div>
							)}
						</>
					)}
				</div>
			</div>
		);
	}

	if (!userId || !showHover) return <>{content}</>;

	return (
		<HoverCard openDelay={120} closeDelay={120}>
			<HoverCardTrigger asChild>{content}</HoverCardTrigger>
			<HoverCardContent
				align="start"
				className="w-80 max-w-[calc(100vw-2rem)] p-0"
			>
				<UserProfileHoverContent
					userId={resolvedUserId}
					label={label}
					subtitle={subtitle}
					avatarUrl={avatarUrl}
					initials={initials}
					description={description}
					createdAt={createdAt}
					email={email}
					showEmail={showEmail}
					showUserId={showUserId}
					showProfileLink={showProfileLink}
					isLoading={lookup.isLoading}
				/>
			</HoverCardContent>
		</HoverCard>
	);
}
