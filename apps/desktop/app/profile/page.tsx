"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	Alert,
	AlertDescription,
	AppCard,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	type IApp,
	type IMetadata,
	Skeleton,
	useBackend,
	useInfiniteInvoke,
	useInvoke,
	userDisplayName,
	userHandle,
	userInitials,
} from "@flow-like/flow-like-ui";
import { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import type { IUserLookup } from "@flow-like/flow-like-ui/state/backend-state/types";
import { motion } from "framer-motion";
import {
	AlertCircle,
	BadgeCheck,
	CalendarDays,
	CircleUserRound,
	Loader2,
	type LucideIcon,
	Mail,
	Package,
	PanelsTopLeft,
	UserRound,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { type ReactNode, useMemo } from "react";

const fadeIn = {
	hidden: { opacity: 0, y: 12 },
	visible: { opacity: 1, y: 0 },
};

const APP_SKELETON_IDS = [
	"app-skeleton-one",
	"app-skeleton-two",
	"app-skeleton-three",
	"app-skeleton-four",
	"app-skeleton-five",
	"app-skeleton-six",
];

function formatDate(value: string) {
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return "Unknown";
	return date.toLocaleDateString("en-US", {
		year: "numeric",
		month: "short",
		day: "numeric",
	});
}

const ProfileSkeleton = () => (
	<main className="flex-1 min-h-0 overflow-auto bg-background">
		<section className="border-b bg-muted/25">
			<div className="mx-auto max-w-6xl px-4 py-6 sm:px-6 lg:px-8">
				<div className="grid gap-y-4 sm:grid-cols-[96px_minmax(0,1fr)] sm:gap-x-5">
					<Skeleton className="h-4 w-24 sm:col-start-2" />
					<Skeleton className="h-24 w-24 rounded-lg sm:row-start-2" />
					<div className="min-w-0 space-y-3 sm:col-start-2">
						<Skeleton className="h-8 w-64 max-w-full" />
						<Skeleton className="h-5 w-40 max-w-full" />
						<div className="flex flex-wrap gap-2">
							<Skeleton className="h-7 w-32" />
							<Skeleton className="h-7 w-28" />
						</div>
						<div className="grid gap-2 pt-1 sm:grid-cols-3">
							<Skeleton className="h-12 w-full" />
							<Skeleton className="h-12 w-full" />
							<Skeleton className="h-12 w-full" />
						</div>
					</div>
				</div>
			</div>
		</section>
		<div className="mx-auto max-w-6xl space-y-6 px-4 py-6 sm:px-6 lg:px-8">
			<div className="space-y-6 sm:pl-[116px]">
				<Skeleton className="h-20 w-full" />
				<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
					{APP_SKELETON_IDS.map((id) => (
						<div key={id} className="space-y-3">
							<Skeleton className="h-44 w-full rounded-lg" />
							<Skeleton className="h-4 w-3/4" />
							<Skeleton className="h-4 w-1/2" />
						</div>
					))}
				</div>
			</div>
		</div>
	</main>
);

const ProfileError = ({ error }: { error: string }) => (
	<main className="flex flex-1 min-h-0 items-center justify-center overflow-auto bg-background p-4">
		<motion.div
			initial={{ opacity: 0, y: 8 }}
			animate={{ opacity: 1, y: 0 }}
			className="w-full max-w-md"
		>
			<Alert className="border-destructive/30 bg-destructive/5">
				<AlertCircle className="h-4 w-4" />
				<AlertDescription>
					{error || i18next.t('failedToLoadUserProfile', 'Failed to load user profile')}
				</AlertDescription>
			</Alert>
		</motion.div>
	</main>
);

function ProfileFact({
	icon: Icon,
	label,
	value,
}: {
	icon: LucideIcon;
	label: string;
	value: string;
}) {
	return (
		<div className="min-w-0 rounded-md border bg-background/70 px-3 py-2">
			<div className="flex items-center gap-2 text-xs font-medium uppercase text-muted-foreground">
				<Icon className="h-3.5 w-3.5 shrink-0" />
				<span className="truncate">{label}</span>
			</div>
			<div className="mt-1 truncate text-sm font-semibold">{value}</div>
		</div>
	);
}

function TextSection({
	icon: Icon,
	title,
	children,
}: {
	icon: LucideIcon;
	title: string;
	children: ReactNode;
}) {
	return (
		<section className="border-b pb-5">
			<div className="mb-3 flex items-center gap-2">
				<Icon className="h-4 w-4 text-muted-foreground" />
				<h2 className="text-sm font-semibold uppercase text-muted-foreground">
					{title}
				</h2>
			</div>
			<div className="max-w-4xl text-sm leading-6 text-foreground/85">
				{children}
			</div>
		</section>
	);
}

function EmptyAppsState({ displayName }: { displayName: string }) {
	const { t } = useTranslation("common");
	return (
		<div className="rounded-lg border border-dashed p-8 text-center sm:p-12">
			<Package className="mx-auto h-10 w-10 text-muted-foreground" />
			<h3 className="mt-4 text-base font-semibold">{t('noPublishedAppsYet', 'No published apps yet')}</h3>
			<p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground">{t('displaynameHasNotPublishedAnyAppsToTheStore', '{{displayName}} has not published any apps to the store.', { displayName })}</p>
		</div>
	);
}

function AppsLoadingGrid() {
	return (
		<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
			{APP_SKELETON_IDS.map((id) => (
				<div key={id} className="space-y-3">
					<Skeleton className="h-44 w-full rounded-lg" />
					<Skeleton className="h-4 w-3/4" />
					<Skeleton className="h-4 w-1/2" />
				</div>
			))}
		</div>
	);
}

const ProfileContent = ({
	user,
	apps,
	hasNextPage,
	fetchNextPage,
	isFetchingNextPage,
	isAppsLoading,
	appsError,
}: {
	user: IUserLookup;
	apps: [IApp, IMetadata | undefined][];
	hasNextPage?: boolean;
	fetchNextPage: () => void;
	isFetchingNextPage: boolean;
	isAppsLoading: boolean;
	appsError: Error | null;
}) => {
	const { t } = useTranslation("common");
	const router = useRouter();
	const displayName = userDisplayName(user, "Unknown user");
	const username = userHandle(user);
	const initials = userInitials(displayName, "??");
	const joinDate = formatDate(user.created_at);
	const hasDescription = Boolean(user.description?.trim());
	const hasAdditionalInfo = Boolean(user.additional_info?.trim());
	const appCountLabel = isAppsLoading
		? "Loading"
		: `${apps.length} ${apps.length === 1 ? "app" : "apps"}`;

	return (
		<main className="flex-1 min-h-0 overflow-auto bg-background">
			<section className="border-b bg-muted/25">
				<motion.div
					variants={fadeIn}
					initial="hidden"
					animate="visible"
					transition={{ duration: 0.25 }}
					className="mx-auto max-w-6xl px-4 py-6 sm:px-6 lg:px-8"
				>
					<div className="grid gap-y-4 sm:grid-cols-[96px_minmax(0,1fr)] sm:gap-x-5">
						<div className="inline-flex items-center gap-2 text-xs font-semibold uppercase text-muted-foreground sm:col-start-2">
							<CircleUserRound className="h-3.5 w-3.5" />
							{t('publicProfile', 'Public profile')}
						</div>

						<Avatar className="h-24 w-24 shrink-0 rounded-lg border bg-card shadow-sm sm:row-start-2">
							<AvatarImage src={user.avatar_url} alt={displayName} />
							<AvatarFallback className="rounded-lg text-2xl font-semibold">
								{initials}
							</AvatarFallback>
						</Avatar>

						<div className="min-w-0 space-y-3 sm:col-start-2">
							<div className="space-y-1.5">
								<h1 className="break-words text-3xl font-semibold tracking-tight sm:text-4xl">
									{displayName}
								</h1>
								{username && (
									<p className="break-words text-base text-muted-foreground">{`@${username}`}</p>
								)}
							</div>

							<div className="flex flex-wrap gap-2">
								<Badge variant="secondary">
									<BadgeCheck />
									{t('flowlikeUser', 'Flow-Like user')}
								</Badge>
								{user.email && (
									<Badge variant="outline">
										<Mail />
										{t('contactAvailable', 'Contact available')}
									</Badge>
								)}
							</div>

							<div className="grid gap-2 pt-1 sm:grid-cols-3 lg:max-w-2xl">
								<ProfileFact
									icon={Package}
									label="Published"
									value={appCountLabel}
								/>
								<ProfileFact
									icon={CalendarDays}
									label="Joined"
									value={joinDate}
								/>
								<ProfileFact
									icon={Mail}
									label="Contact"
									value={user.email ? "Available" : t('notListed', 'Not listed')}
								/>
							</div>
						</div>
					</div>
				</motion.div>
			</section>

			<motion.div
				variants={fadeIn}
				initial="hidden"
				animate="visible"
				transition={{ duration: 0.25, delay: 0.05 }}
				className="mx-auto max-w-6xl space-y-6 px-4 py-6 sm:px-6 lg:px-8"
			>
				<div className="min-w-0 space-y-5 sm:pl-[116px]">
					{hasDescription && (
						<TextSection icon={UserRound} title={t('about', 'About')}>
							<p>{user.description}</p>
						</TextSection>
					)}

					{hasAdditionalInfo && (
						<TextSection icon={PanelsTopLeft} title={t('additionalInformation', 'Additional information')}>
							<p>{user.additional_info}</p>
						</TextSection>
					)}

					{!hasDescription && !hasAdditionalInfo && (
						<TextSection icon={UserRound} title={t('about', 'About')}>
							<p className="text-muted-foreground">
								{t('thisUserHasNotAddedProfileDetailsYet', 'This user has not added profile details yet.')}
							</p>
						</TextSection>
					)}

					<section className="space-y-5">
						<div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
							<div>
								<div className="flex items-center gap-2">
									<Package className="h-5 w-5 text-muted-foreground" />
									<h2 className="text-xl font-semibold tracking-tight">
										{t('publishedApps', 'Published apps')}
									</h2>
								</div>
								<p className="mt-1 text-sm text-muted-foreground">{t('publicAppsSharedByDisplayname', 'Public apps shared by {{displayName}}.', { displayName })}</p>
							</div>
							<Badge variant="secondary">{appCountLabel}</Badge>
						</div>

						{appsError && (
							<Alert className="border-destructive/30 bg-destructive/5">
								<AlertCircle className="h-4 w-4" />
								<AlertDescription>{t('failedToLoadAppsMessage', 'Failed to load apps: {{message}}', { message: appsError.message })}</AlertDescription>
							</Alert>
						)}

						{isAppsLoading ? (
							<AppsLoadingGrid />
						) : apps.length === 0 ? (
							<EmptyAppsState displayName={displayName} />
						) : (
							<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
								{apps.map(([app, metadata]) => (
									<AppCard
										key={app.id}
										app={app}
										variant="extended"
										metadata={metadata}
										className="w-full"
										onClick={() => router.push(`/store?id=${app.id}`)}
										href={`/store?id=${app.id}`}
									/>
								))}
							</div>
						)}

						{hasNextPage && (
							<div className="flex justify-center pt-2">
								<Button
									onClick={() => fetchNextPage()}
									disabled={isFetchingNextPage}
									variant="outline"
								>
									{isFetchingNextPage ? (
										<>
											<Loader2 className="h-4 w-4 animate-spin" />
											{t('loadingMore', 'Loading more')}
										</>
									) : (
										t('loadMoreApps', 'Load more apps')
									)}
								</Button>
							</div>
						)}
					</section>
				</div>
			</motion.div>
		</main>
	);
};

export default function ProfilePage() {
	const params = useSearchParams();
	const sub = params.get("sub") || "";
	const backend = useBackend();
	const user = useInvoke(backend.userState.lookupUser, backend.userState, [
		sub,
	]);
	const {
		data: apps,
		hasNextPage,
		fetchNextPage,
		isFetchingNextPage,
		isLoading: isAppsLoading,
		error: appsError,
	} = useInfiniteInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		undefined,
		undefined,
		undefined,
		sub,
		IAppSearchSort.BestRated,
		undefined,
	]);

	const combinedApps = useMemo(() => {
		if (!apps) return [];
		return apps.pages.flat();
	}, [apps]);

	if (user.isFetching) {
		return <ProfileSkeleton />;
	}

	if (user.error) {
		return <ProfileError error={user.error.message} />;
	}

	if (!user.data) {
		return <ProfileError error="User not found" />;
	}

	return (
		<ProfileContent
			user={user.data}
			apps={combinedApps}
			hasNextPage={hasNextPage}
			fetchNextPage={fetchNextPage}
			isFetchingNextPage={isFetchingNextPage}
			isAppsLoading={isAppsLoading}
			appsError={appsError}
		/>
	);
}
