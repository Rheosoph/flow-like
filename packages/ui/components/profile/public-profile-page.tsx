"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	CalendarDays,
	Loader2,
	Mail,
	Package,
	Pencil,
	RefreshCw,
} from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { useInfiniteInvoke, useInvoke } from "../../hooks/use-invoke";
import type { IApp } from "../../lib/schema/app/app";
import { IAppSearchSort } from "../../lib/schema/app/app-search-query";
import type { IMetadata } from "../../lib/schema/bit/bit";
import {
	userDisplayName,
	userHandle,
	userInitials,
} from "../../lib/user-display";
import { useBackend } from "../../state/backend-state";
import type { IUserLookup } from "../../state/backend-state/types";
import { Alert, AlertDescription } from "../ui/alert";
import { AppCard } from "../ui/app-card";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";

const PLACEHOLDERS = ["one", "two", "three", "four", "five", "six"];

export function PublicProfileSkeleton() {
	return (
		<main
			className="flex-1 min-h-0 overflow-y-auto bg-background"
			aria-busy="true"
		>
			<div className="mx-auto max-w-6xl space-y-8 px-4 py-8 sm:px-6 lg:px-8">
				<div className="flex items-center gap-5 border-b pb-8">
					<Skeleton className="size-20 shrink-0 rounded-xl" />
					<div className="min-w-0 flex-1 space-y-3">
						<Skeleton className="h-4 w-24" />
						<Skeleton className="h-8 w-64 max-w-full" />
						<Skeleton className="h-4 w-40 max-w-full" />
					</div>
				</div>
				<Skeleton className="h-20 w-full max-w-3xl" />
				<AppsSkeleton />
			</div>
		</main>
	);
}

function AppsSkeleton() {
	return (
		<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3" aria-busy="true">
			{PLACEHOLDERS.map((id) => (
				<div key={id} className="space-y-3">
					<Skeleton className="h-44 w-full rounded-lg" />
					<Skeleton className="h-4 w-3/4" />
					<Skeleton className="h-4 w-1/2" />
				</div>
			))}
		</div>
	);
}

function ProfileUnavailable({
	onRetry,
	isRetrying,
	missingId = false,
}: {
	onRetry?: () => void;
	isRetrying?: boolean;
	missingId?: boolean;
}) {
	const { t } = useTranslation("common");
	return (
		<main className="flex flex-1 min-h-0 items-center justify-center overflow-auto bg-background p-6">
			<div className="w-full max-w-md space-y-4 rounded-xl border bg-card p-6">
				<AlertCircle
					className="size-6 text-muted-foreground"
					aria-hidden="true"
				/>
				<h1 className="text-xl font-semibold">
					{t("profileUnavailable", "Profile unavailable")}
				</h1>
				<p className="text-sm text-muted-foreground">
					{missingId
						? t(
								"choosePublicProfile",
								"Open a person's profile from an app or sign in to view your own.",
							)
						: t(
								"profileUnavailableDescription",
								"This profile could not be loaded. It may be unavailable, or the connection may have failed.",
							)}
				</p>
				<div className="flex flex-wrap gap-2">
					{onRetry && (
						<Button onClick={onRetry} disabled={isRetrying}>
							<RefreshCw
								className={isRetrying ? "size-4 animate-spin" : "size-4"}
								aria-hidden="true"
							/>
							{t("retry", "Retry")}
						</Button>
					)}
					<Button variant="outline" asChild>
						<Link href="/store">{t("browseApps", "Browse apps")}</Link>
					</Button>
				</div>
			</div>
		</main>
	);
}

export interface PublicProfileContentProps {
	user: IUserLookup;
	apps: [IApp, IMetadata | undefined][];
	isOwnProfile: boolean;
	hasNextPage?: boolean;
	fetchNextPage: () => void;
	isFetchingNextPage: boolean;
	isAppsLoading: boolean;
	appsError: Error | null;
	onRetryApps: () => void;
	isRetryingApps?: boolean;
	profileRefreshFailed?: boolean;
	onRetryProfile?: () => void;
}

export function PublicProfileContent({
	user,
	apps,
	isOwnProfile,
	hasNextPage,
	fetchNextPage,
	isFetchingNextPage,
	isAppsLoading,
	appsError,
	onRetryApps,
	isRetryingApps,
	profileRefreshFailed,
	onRetryProfile,
}: PublicProfileContentProps) {
	const { t, i18n } = useTranslation("common");
	const name = userDisplayName(user, t("unknownUser", "Unknown user"));
	const handle = userHandle(user);
	const date = user.created_at ? new Date(user.created_at) : undefined;
	const joined =
		date && !Number.isNaN(date.getTime())
			? date.toLocaleDateString(i18n.language, {
					month: "long",
					year: "numeric",
				})
			: undefined;
	const appCount =
		hasNextPage || appsError
			? t("profileAppsLoaded", "{{count}} loaded", { count: apps.length })
			: t(
					"profileAppsCount",
					apps.length === 1 ? "{{count}} app" : "{{count}} apps",
					{ count: apps.length },
				);

	return (
		<main className="flex-1 min-h-0 overflow-y-auto bg-background">
			<div className="mx-auto max-w-6xl space-y-8 px-4 py-6 sm:px-6 sm:py-8 lg:px-8">
				<header className="flex flex-col gap-6 border-b pb-6 sm:pb-8">
					<div className="flex flex-col gap-5 sm:flex-row sm:items-center">
						<Avatar className="size-20 shrink-0 rounded-xl border bg-card sm:size-24">
							<AvatarImage src={user.avatar_url} alt={name} />
							<AvatarFallback className="rounded-xl text-2xl font-semibold">
								{userInitials(name)}
							</AvatarFallback>
						</Avatar>
						<div className="min-w-0 flex-1 space-y-2">
							<p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
								{t("publicProfile", "Public profile")}
							</p>
							<h1 className="break-words text-3xl font-semibold tracking-tight sm:text-4xl">
								{name}
							</h1>
							{handle && (
								<p className="break-words text-sm text-muted-foreground">
									@{handle}
								</p>
							)}
							{joined && (
								<p className="flex items-center gap-2 text-sm text-muted-foreground">
									<CalendarDays
										className="size-3.5 shrink-0"
										aria-hidden="true"
									/>
									{t("profileJoinedDate", "Joined {{date}}", { date: joined })}
								</p>
							)}
						</div>
						<div className="flex flex-wrap gap-2 self-start sm:self-center">
							{isOwnProfile && (
								<Button asChild variant="outline">
									<Link href="/account">
										<Pencil className="size-4" aria-hidden="true" />
										{t("editProfile", "Edit profile")}
									</Link>
								</Button>
							)}
							{user.email && !isOwnProfile && (
								<Button asChild variant="outline">
									<a href={`mailto:${encodeURIComponent(user.email)}`}>
										<Mail className="size-4" aria-hidden="true" />
										{t("email", "Email")}
									</a>
								</Button>
							)}
						</div>
					</div>
					{isOwnProfile && (
						<p className="text-sm text-muted-foreground">
							{t(
								"yourPublicProfileHint",
								"This is how your profile appears to other people on this hub.",
							)}
						</p>
					)}
				</header>

				{profileRefreshFailed && (
					<Alert>
						<AlertCircle className="size-4" />
						<AlertDescription className="flex flex-wrap items-center gap-3">
							{t(
								"profileRefreshFailed",
								"Your last loaded profile is shown. The latest details could not be refreshed.",
							)}
							<Button size="sm" variant="outline" onClick={onRetryProfile}>
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				)}

				<section
					className="max-w-3xl space-y-3"
					aria-labelledby="profile-about"
				>
					<h2 id="profile-about" className="text-base font-semibold">
						{t("about", "About")}
					</h2>
					{user.description?.trim() ? (
						<p className="whitespace-pre-wrap break-words text-sm leading-6 text-muted-foreground">
							{user.description}
						</p>
					) : (
						<p className="text-sm leading-6 text-muted-foreground">
							{isOwnProfile
								? t(
										"addProfileBio",
										"Add a short bio in your account settings to introduce yourself.",
									)
								: t(
										"thisUserHasNotAddedProfileDetailsYet",
										"This user has not added profile details yet.",
									)}
						</p>
					)}
				</section>

				{user.additional_info?.trim() && (
					<section
						className="max-w-3xl space-y-3"
						aria-labelledby="profile-additional"
					>
						<h2 id="profile-additional" className="text-base font-semibold">
							{t("additionalInformation", "Additional information")}
						</h2>
						<p className="whitespace-pre-wrap break-words text-sm leading-6 text-muted-foreground">
							{user.additional_info}
						</p>
					</section>
				)}

				<section
					className="space-y-5 border-t pt-6"
					aria-labelledby="profile-apps"
				>
					<div className="flex flex-wrap items-center justify-between gap-3">
						<div className="space-y-1">
							<h2
								id="profile-apps"
								className="text-xl font-semibold tracking-tight"
							>
								{t("publishedApps", "Published apps")}
							</h2>
							<p className="text-sm text-muted-foreground">
								{t(
									"publicAppsSharedByDisplayname",
									"Public apps shared by {{displayName}}.",
									{ displayName: name },
								)}
							</p>
						</div>
						{!isAppsLoading && (!appsError || apps.length > 0) && (
							<Badge variant="secondary" asChild>
								<output>{appCount}</output>
							</Badge>
						)}
					</div>

					{appsError && (
						<Alert variant="destructive">
							<AlertCircle className="size-4" />
							<AlertDescription className="flex flex-wrap items-center gap-3">
								{t(
									"profileAppsLoadFailed",
									"Apps could not be loaded. Please try again.",
								)}
								<Button
									variant="outline"
									size="sm"
									onClick={onRetryApps}
									disabled={isRetryingApps}
								>
									{t("retry", "Retry")}
								</Button>
							</AlertDescription>
						</Alert>
					)}
					{isAppsLoading ? (
						<AppsSkeleton />
					) : apps.length > 0 ? (
						<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
							{apps.map(([app, metadata]) => (
								<AppCard
									key={app.id}
									app={app}
									metadata={metadata}
									variant="extended"
									className="w-full"
									href={`/store?id=${encodeURIComponent(app.id)}`}
								/>
							))}
						</div>
					) : !appsError ? (
						<div className="rounded-xl border border-dashed px-6 py-12 text-center">
							<Package
								className="mx-auto size-9 text-muted-foreground"
								aria-hidden="true"
							/>
							<h3 className="mt-4 font-semibold">
								{t("noPublishedAppsYet", "No published apps yet")}
							</h3>
							<p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground">
								{isOwnProfile
									? t(
											"yourPublishedAppsHint",
											"Apps you publish to the store will appear here.",
										)
									: t(
											"displaynameHasNotPublishedAnyAppsToTheStore",
											"{{displayName}} has not published any apps to the store.",
											{ displayName: name },
										)}
							</p>
						</div>
					) : null}
					{hasNextPage && !appsError && (
						<div className="flex justify-center">
							<Button
								variant="outline"
								onClick={fetchNextPage}
								disabled={isFetchingNextPage}
							>
								{isFetchingNextPage && (
									<Loader2 className="size-4 animate-spin" aria-hidden="true" />
								)}
								{isFetchingNextPage
									? t("loadingMore", "Loading more")
									: t("loadMoreApps", "Load more apps")}
							</Button>
						</div>
					)}
				</section>
			</div>
		</main>
	);
}

export function PublicProfilePage() {
	const params = useSearchParams();
	const auth = useAuth();
	const sub = params.get("sub")?.trim() || auth.user?.profile.sub || "";
	const backend = useBackend();
	const user = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		[sub],
		Boolean(sub),
	);
	const apps = useInfiniteInvoke(
		backend.appState.searchApps,
		backend.appState,
		[
			undefined,
			undefined,
			undefined,
			undefined,
			sub,
			IAppSearchSort.BestRated,
			undefined,
		],
		50,
		Boolean(sub && user.data),
	);
	const combinedApps = useMemo(() => {
		const unique = new Map<string, [IApp, IMetadata | undefined]>();
		for (const item of apps.data?.pages.flat() ?? [])
			unique.set(item[0].id, item);
		return [...unique.values()];
	}, [apps.data]);

	if (!sub && auth.isLoading) return <PublicProfileSkeleton />;
	if (!sub) return <ProfileUnavailable missingId />;
	if (user.isPending) return <PublicProfileSkeleton />;
	if (!user.data)
		return (
			<ProfileUnavailable
				onRetry={() => void user.refetch()}
				isRetrying={user.isFetching}
			/>
		);
	return (
		<PublicProfileContent
			user={user.data}
			apps={combinedApps}
			isOwnProfile={
				auth.isAuthenticated && user.data.id === auth.user?.profile.sub
			}
			hasNextPage={apps.hasNextPage}
			fetchNextPage={() => void apps.fetchNextPage()}
			isFetchingNextPage={apps.isFetchingNextPage}
			isAppsLoading={apps.isPending}
			appsError={apps.error}
			onRetryApps={() => void apps.refetch()}
			isRetryingApps={apps.isFetching}
			profileRefreshFailed={user.isError}
			onRetryProfile={() => void user.refetch()}
		/>
	);
}
