"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AtSign,
	CalendarDays,
	ExternalLink,
	IdCard,
	Mail,
	UserRound,
} from "lucide-react";
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
import { Avatar, AvatarFallback, AvatarImage } from "../../ui/avatar";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../../ui/hover-card";
import { RelativeTime } from "../../ui/relative-time";
import { Skeleton } from "../../ui/skeleton";
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

const avatarSizeClasses: Record<string, string> = {
	xs: "h-5 w-5",
	sm: "h-6 w-6",
	md: "h-8 w-8",
	lg: "h-10 w-10",
	xl: "h-14 w-14",
	"2xl": "h-16 w-16",
};

const avatarFallbackClasses: Record<string, string> = {
	xs: "text-[9px]",
	sm: "text-[10px]",
	md: "text-xs",
	lg: "text-sm",
	xl: "text-base",
	"2xl": "text-lg",
};

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

function ProfileAvatar({
	avatarUrl,
	initials,
	label,
	size,
	className,
}: Readonly<{
	avatarUrl?: string | null;
	initials: string;
	label: string;
	size: string;
	className?: string;
}>) {
	const sizeClass = avatarSizeClasses[size] ?? avatarSizeClasses.md;
	const fallbackClass = avatarFallbackClasses[size] ?? avatarFallbackClasses.md;

	return (
		<Avatar className={cn(sizeClass, "shrink-0", className)}>
			<AvatarImage src={avatarUrl ?? ""} alt={label} />
			<AvatarFallback className={fallbackClass}>{initials}</AvatarFallback>
		</Avatar>
	);
}

function DetailRow({
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

function ProfileHoverContent({
	userId,
	label,
	subtitle,
	avatarUrl,
	initials,
	description,
	createdAt,
	email,
	showEmail,
	showUserId,
	showProfileLink,
	isLoading,
}: Readonly<{
	userId?: string;
	label: string;
	subtitle: string | null;
	avatarUrl?: string | null;
	initials: string;
	description?: string | null;
	createdAt?: string | number | null;
	email?: string | null;
	showEmail: boolean;
	showUserId: boolean;
	showProfileLink: boolean;
	isLoading: boolean;
}>) {
	const { t } = useTranslation("common");
	return (
		<div className="min-w-0">
			<div className="border-b bg-muted/30 p-4">
				<div className="flex min-w-0 items-start gap-3">
					<ProfileAvatar
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
								{t('viewProfile', 'View profile')}
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
							<DetailRow icon={<Mail className="h-3.5 w-3.5" />} label="Email">
								<span className="block truncate" title={email}>
									{email}
								</span>
							</DetailRow>
						) : null}
						{createdAt ? (
							<DetailRow
								icon={<CalendarDays className="h-3.5 w-3.5" />}
								label="Joined"
							>
								<RelativeTime value={createdAt} />
							</DetailRow>
						) : null}
						{showUserId && userId ? (
							<DetailRow
								icon={<IdCard className="h-3.5 w-3.5" />}
								label={t('userId', 'User ID')}
							>
								<code className="block truncate rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
									{userId}
								</code>
							</DetailRow>
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

export function A2UIUserProfile({
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
		<ProfileAvatar
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
								<span className="truncate">{t('viewProfile', 'View profile')}</span>
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
				<ProfileHoverContent
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
