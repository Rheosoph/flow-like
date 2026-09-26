"use client";

import { usePaymentDistribution } from "../payments/use-payments";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	ArrowLeft,
	BookOpen,
	Check,
	Download,
	ExternalLink,
	FileCode,
	Github,
	Globe,
	HelpCircle,
	KeyRound,
	Loader2,
	LogIn,
	Package,
	RefreshCw,
	Settings,
	Shield,
	ShoppingCart,
	Star,
	Tag,
	Target,
	User,
} from "lucide-react";
import Link from "next/link";
import { useInvoke } from "../../hooks/use-invoke";
import { getErrorMessage } from "../../lib/error-message";
import {
	readManifestWidgetBundleHash,
	readManifestWidgets,
} from "../../lib/package-widgets";
import { isMaintainer } from "../../lib/permission/wasm-package-permission";
import { asArray } from "../../lib/response-shape";
import {
	type PackageMeta,
	PackageStatus,
	type PackageVersion,
	type RegistryEntry,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	EmptyState,
	RelativeTime,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	TextEditor,
} from "../ui";
import {
	type CompileStatus,
	PackageStatusBadge,
} from "../ui/package-status-badge";
import { PackageReviewsTab } from "./package-reviews-tab";
import { packageWorkspaceHref } from "./package-workspace/workspace-href";
import { WidgetCardGrid } from "./widget-card";
import { WidgetNetworkAccessCard } from "./widget-network-access";

function PermissionBadge({
	label,
	enabled,
}: { label: string; enabled: boolean }) {
	return (
		<Badge variant={enabled ? "default" : "outline"} className="gap-1">
			{enabled ? <Check className="h-3 w-3" /> : null}
			{label}
		</Badge>
	);
}

function PackageMarkdown({ content }: { content: string }) {
	return (
		<div className="text-sm leading-7 text-foreground/90 [&_a]:font-medium [&_a]:text-primary [&_a]:underline [&_a]:decoration-primary/50 [&_a]:underline-offset-4 [&_a:hover]:decoration-primary [&_code]:rounded [&_code]:bg-muted/70 [&_code]:px-1.5 [&_code]:py-0.5 [&_h1]:mb-2 [&_h1]:mt-7 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:tracking-tight [&_h1:first-of-type]:mt-0 [&_h2]:mb-2 [&_h2]:mt-6 [&_h2]:text-xl [&_h2]:font-semibold [&_h2:first-of-type]:mt-0 [&_h3]:mb-1.5 [&_h3]:mt-5 [&_h3]:text-lg [&_h3]:font-semibold [&_h3:first-of-type]:mt-0 [&_li]:my-1 [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:py-0.5 [&_ul]:my-3 [&_ul]:list-disc [&_ul]:pl-6">
			<TextEditor initialContent={content} isMarkdown />
		</div>
	);
}

function NodeCard({
	node,
}: {
	node: { id: string; name: string; description: string; category: string };
}) {
	return (
		<Card>
			<CardHeader className="pb-2">
				<div className="flex items-start justify-between">
					<CardTitle className="text-sm font-medium">{node.name}</CardTitle>
					<Badge variant="outline" className="text-xs">
						{node.category}
					</Badge>
				</div>
			</CardHeader>
			<CardContent>
				<p className="text-xs text-muted-foreground">{node.description}</p>
			</CardContent>
		</Card>
	);
}

function VersionRow({
	version,
	isLatest,
}: {
	version: PackageVersion;
	isLatest: boolean;
}) {
	const { t } = useTranslation("store");
	const isPending = version.status === PackageStatus.PendingReview;
	const isRejected = version.status === PackageStatus.Rejected;
	const isDisabled = version.status === PackageStatus.Disabled;

	return (
		<div className="flex items-center justify-between gap-3 py-2 border-b last:border-0">
			<div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
				<code className="text-sm font-mono">{version.version}</code>
				{isLatest && <Badge variant="secondary">{t("latest", "Latest")}</Badge>}
				{isPending && (
					<Badge variant="outline">
						{t("pendingReview", "Pending Review")}
					</Badge>
				)}
				{isRejected && (
					<Badge variant="destructive">{t("rejected", "Rejected")}</Badge>
				)}
				{isDisabled && (
					<Badge variant="secondary">{t("disabled", "Disabled")}</Badge>
				)}
				{version.yanked && (
					<Badge variant="destructive">{t("yanked", "Yanked")}</Badge>
				)}
			</div>
			<RelativeTime
				className="shrink-0 text-sm text-muted-foreground"
				value={version.publishedAt}
			/>
		</div>
	);
}

export interface PackageDetailViewProps {
	pkg: RegistryEntry | null | undefined;
	isLoading: boolean;
	loadError?: unknown;
	onRetry?: () => void;
	installedVersion: string | null | undefined;
	onBack: () => void;
	onInstall: (version?: string) => void;
	onUninstall: () => void;
	isInstalling?: boolean;
	isUninstalling?: boolean;
	compileStatus?: CompileStatus;
	price?: number;
	visibility?: string;
	priceLabel?: string;
	hasAccess?: boolean;
	isPurchasing?: boolean;
	isRequesting?: boolean;
	awaitingCheckout?: boolean;
	onBuy?: () => void;
	onRequestAccess?: () => void;
	onGetOrBuy?: () => void;
	/** Set while the viewer's session has expired and nothing could be loaded without it. */
	onSignIn?: () => void;
	currentUserPermission?: number;
	fetcher?: GenericFetcher;
	auth?: unknown;
}

export function PackageDetailView(props: PackageDetailViewProps) {
	const purchasingAllowed = usePaymentDistribution();
	const { t } = useTranslation("store");
	const {
		pkg,
		isLoading,
		loadError,
		onRetry,
		installedVersion,
		onBack,
		onInstall,
		onUninstall,
		isInstalling,
		isUninstalling,
		compileStatus,
		price,
		visibility,
		priceLabel,
		hasAccess,
		isPurchasing,
		isRequesting,
		awaitingCheckout,
		onBuy,
		onRequestAccess,
		onGetOrBuy,
		onSignIn,
		currentUserPermission,
		fetcher,
		auth,
	} = props;

	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const { data: meta } = useQuery<PackageMeta | null>({
		queryKey: ["package-meta", pkg?.id],
		queryFn: async () => {
			if (!profile.data || !pkg?.id || !fetcher) return null;
			try {
				return await fetcher<PackageMeta>(
					profile.data.hub_profile,
					`registry/package/${pkg.id}/meta`,
					{ method: "GET" },
					auth,
				);
			} catch {
				return null;
			}
		},
		enabled: !!profile.data && !!pkg?.id && !!fetcher,
	});

	if (isLoading) {
		return (
			<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
				<div className="mx-auto w-full max-w-5xl space-y-6">
					<div className="flex items-center gap-4">
						<Skeleton className="h-9 w-24" />
					</div>
					<Skeleton className="h-32 w-full" />
					<Skeleton className="h-64 w-full" />
				</div>
			</main>
		);
	}

	if (!pkg) {
		const back = {
			label: t("backToPackages", "Back to packages"),
			onClick: onBack,
		};
		const primary = onSignIn
			? { label: t("common:signIn", "Sign in"), onClick: onSignIn }
			: onRetry
				? { label: t("tryAgain", "Try again"), onClick: onRetry }
				: undefined;
		const copy = onSignIn
			? {
					title: t("common:yourSessionExpired", "Your session expired"),
					description: t(
						"common:signInAgainToSeeThisPackage",
						"Sign in again to see this package.",
					),
				}
			: loadError
				? {
						title: t("couldntLoadThisPackage", "Couldn't load this package"),
						description: t(
							"valCheckYourConnectionOrSignInIfThePackageIsPrivate",
							"{{val}} — check your connection, or sign in if the package is private.",
							{ val: getErrorMessage(loadError) },
						),
					}
				: {
						title: t("packageNotFound", "Package not found"),
						description: t(
							"thisPackageDoesntExistIsNoLongerPublishedOrYouDontHaveAccessToIt",
							"This package doesn't exist, is no longer published, or you don't have access to it.",
						),
					};

		return (
			<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
				<div className="mx-auto w-full max-w-5xl space-y-6">
					<Button variant="ghost" size="sm" onClick={onBack} className="gap-2">
						<ArrowLeft className="h-4 w-4" />
						{t("back", "Back")}
					</Button>
					<div className="flex justify-center">
						<EmptyState
							icons={[onSignIn ? LogIn : Package]}
							title={copy.title}
							description={copy.description}
							action={primary ? [primary, back] : [back]}
						/>
					</div>
				</div>
			</main>
		);
	}

	const manifest = pkg.manifest;
	const versions = asArray(pkg.versions);
	const widgets = readManifestWidgets(manifest);
	const canManage =
		currentUserPermission != null && isMaintainer(currentUserPermission);
	const hasPendingVersion = versions.some(
		(version) => version.status === PackageStatus.PendingReview,
	);
	const latestVersion =
		versions.find(
			(v) =>
				!v.yanked &&
				v.status !== PackageStatus.Rejected &&
				v.status !== PackageStatus.Disabled,
		)?.version ?? versions[0]?.version;
	const widgetBundleHash =
		readManifestWidgetBundleHash(manifest) ??
		versions.find((v) => v.version === latestVersion)?.widgetBundleHash ??
		undefined;
	const canInstallForReview =
		canManage &&
		!!latestVersion &&
		(hasPendingVersion || pkg.status === PackageStatus.PendingReview);
	const isInstallable =
		pkg.status === PackageStatus.Active ||
		pkg.status === PackageStatus.Deprecated ||
		canInstallForReview;
	const isInstalled = !!installedVersion;
	const hasUpdate =
		isInstallable &&
		isInstalled &&
		!!latestVersion &&
		installedVersion !== latestVersion;
	const unavailableActionLabel =
		pkg.status === PackageStatus.PendingReview
			? t("pendingReview", "Pending Review")
			: pkg.status === PackageStatus.Disabled
				? t("disabled", "Disabled")
				: pkg.status === PackageStatus.Rejected
					? t("rejected", "Rejected")
					: t("unavailable", "Unavailable");
	const unavailableActionState =
		pkg.status === PackageStatus.PendingReview
			? t("awaitingReview", "awaiting review")
			: pkg.status === PackageStatus.Disabled
				? t("disabledState", "disabled")
				: pkg.status === PackageStatus.Rejected
					? t("rejectedState", "rejected")
					: t("unavailableState", "unavailable");
	const unavailableActionMessage = isInstalled
		? t(
				"updatesAreUnavailableWhileThisPackageIsVal",
				"Updates are unavailable while this package is {{val}}.",
				{ val: unavailableActionState },
			)
		: t(
				"installIsUnavailableWhileThisPackageIsVal",
				"Install is unavailable while this package is {{val}}.",
				{ val: unavailableActionState },
			);

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-5xl space-y-6">
				{/* Back Button */}
				<Button variant="ghost" onClick={onBack} className="gap-2">
					<ArrowLeft className="h-4 w-4" />
					{t("back", "Back")}
				</Button>

				{/* Header Card */}
				<Card className="relative overflow-hidden bg-card/75">
					{meta?.thumbnail && (
						<>
							<img
								src={meta.thumbnail}
								alt=""
								className="absolute inset-0 h-full w-full scale-[1.02] object-cover opacity-[0.18] saturate-125 dark:opacity-[0.26]"
							/>
							<div className="absolute inset-0 bg-linear-to-r from-card via-card/85 to-card/55" />
						</>
					)}
					<CardHeader className="relative z-10">
						<div className="flex flex-col md:flex-row md:items-start md:justify-between gap-4">
							<div className="flex items-start gap-4">
								<Avatar className="h-14 w-14 rounded-lg">
									{meta?.icon ? (
										<AvatarImage
											src={meta.icon}
											alt={meta.name ?? manifest.name}
											className="object-cover"
										/>
									) : null}
									<AvatarFallback className="rounded-lg bg-muted">
										<Package className="h-7 w-7" />
									</AvatarFallback>
								</Avatar>
								<div>
									<div className="flex items-center gap-2 flex-wrap">
										<CardTitle className="text-2xl">
											{meta?.name || manifest.name}
										</CardTitle>
										{pkg.status === PackageStatus.Disabled && (
											<Badge variant="destructive" className="gap-1">
												{t("disabled", "Disabled")}
											</Badge>
										)}
										{pkg.status === PackageStatus.PendingReview && (
											<Badge variant="secondary" className="gap-1">
												{t("pendingReview", "Pending Review")}
											</Badge>
										)}
										{pkg.status === PackageStatus.Rejected && (
											<Badge variant="destructive" className="gap-1">
												{t("rejected", "Rejected")}
											</Badge>
										)}
										{pkg.verified && (
											<Badge variant="secondary" className="gap-1">
												<Shield className="h-3 w-3" />
												{t("verified", "Verified")}
											</Badge>
										)}
									</div>
									<CardDescription className="mt-1">
										{meta?.description || manifest.description}
									</CardDescription>
									<div className="flex flex-wrap items-center gap-x-4 gap-y-1 mt-2 text-sm text-muted-foreground">
										<span className="flex items-center gap-1">
											<Tag className="h-4 w-4" />
											{`v${latestVersion}`}
										</span>
										<span className="flex items-center gap-1">
											<Download className="h-4 w-4" />
											{(pkg.downloadCount ?? 0).toLocaleString()} downloads
										</span>
										{(pkg.ratingCount ?? 0) > 0 && (
											<span className="flex items-center gap-1">
												<Star className="h-4 w-4 text-yellow-500 fill-yellow-500" />
												{(pkg.avgRating ?? 0).toFixed(1)}
												<span className="text-xs">{`(${pkg.ratingCount})`}</span>
											</span>
										)}
										{price != null && price > 0 ? (
											<span className="flex items-center gap-1 font-medium">
												{priceLabel}
											</span>
										) : priceLabel ? (
											<span className="flex items-center gap-1 font-medium">
												{t("free", "Free")}
											</span>
										) : null}
										{compileStatus && compileStatus !== "idle" && (
											<PackageStatusBadge status={compileStatus} />
										)}
									</div>
								</div>
							</div>

							<div className="flex flex-col gap-2">
								{isInstalled ? (
									<>
										<div className="flex items-center gap-2 text-sm text-muted-foreground">
											<Check className="h-4 w-4 text-green-500" />
											{t(
												"installedVinstalledversion",
												"Installed v{{installedVersion}}",
												{ installedVersion },
											)}
										</div>
										{hasUpdate && (
											<Button
												onClick={() => onInstall(latestVersion)}
												disabled={isInstalling}
											>
												{isInstalling ? (
													<RefreshCw className="mr-2 h-4 w-4 animate-spin" />
												) : (
													<RefreshCw className="mr-2 h-4 w-4" />
												)}
												{t(
													"updateToVlatestversion",
													"Update to v{{latestVersion}}",
													{ latestVersion },
												)}
											</Button>
										)}
										{!hasUpdate &&
											!isInstallable &&
											installedVersion !== latestVersion && (
												<p className="max-w-xs text-sm text-muted-foreground">
													{unavailableActionMessage}
												</p>
											)}
										<Button
											variant="destructive"
											onClick={onUninstall}
											disabled={isUninstalling}
										>
											{t("uninstall", "Uninstall")}
										</Button>
									</>
								) : hasAccess === false &&
									visibility === "public_request_access" ? (
									<Button onClick={onRequestAccess} disabled={isRequesting}>
										{isRequesting ? (
											t("requesting", "Requesting...")
										) : (
											<>
												<KeyRound className="mr-2 h-4 w-4" />
												{t("requestAccess", "Request access")}
											</>
										)}
									</Button>
								) : hasAccess === false &&
									price != null &&
									price > 0 &&
									!purchasingAllowed ? (
									<p className="text-sm text-muted-foreground">
										{t(
											"purchaseUnavailable",
											"Purchasing is unavailable in this app distribution.",
										)}
									</p>
								) : hasAccess === false &&
									price != null &&
									price > 0 &&
									awaitingCheckout ? (
									<div className="flex max-w-xs flex-col gap-1">
										<Button disabled>
											<Loader2 className="mr-2 h-4 w-4 animate-spin" />
											{t("awaitingPayment", "Waiting for payment...")}
										</Button>
										<p className="text-xs text-muted-foreground">
											{t(
												"awaitingPaymentDescription",
												"Finish checkout in your browser. Install unlocks here once the payment is confirmed.",
											)}
										</p>
									</div>
								) : hasAccess === false && price != null && price > 0 ? (
									<Button onClick={onBuy} disabled={isPurchasing}>
										{isPurchasing ? (
											t("processing", "Processing...")
										) : (
											<>
												<ShoppingCart className="mr-2 h-4 w-4" />
												{priceLabel || `€${(price / 100).toFixed(2)}`}
											</>
										)}
									</Button>
								) : hasAccess === false ? (
									<Button onClick={onGetOrBuy} disabled={isRequesting}>
										{isRequesting ? (
											t("processing", "Processing...")
										) : (
											<>
												<Download className="mr-2 h-4 w-4" />
												{t("get", "Get")}
											</>
										)}
									</Button>
								) : !isInstallable ? (
									<>
										<Button disabled>{unavailableActionLabel}</Button>
										<p className="max-w-xs text-sm text-muted-foreground">
											{unavailableActionMessage}
										</p>
									</>
								) : (
									<Button
										onClick={() => onInstall(undefined)}
										disabled={isInstalling}
									>
										{isInstalling ? (
											<RefreshCw className="mr-2 h-4 w-4 animate-spin" />
										) : (
											<Download className="mr-2 h-4 w-4" />
										)}
										{canInstallForReview
											? t("installForTesting", "Install for testing")
											: t("install", "Install")}
									</Button>
								)}
								{canManage && (
									<Button variant="outline" asChild>
										<Link href={packageWorkspaceHref({ id: pkg.id })}>
											<Settings className="mr-2 h-4 w-4" />
											{t("common:managePackage", "Manage package")}
										</Link>
									</Button>
								)}
							</div>
						</div>
					</CardHeader>
				</Card>

				{/* Main Content */}
				<Tabs defaultValue="overview" className="w-full">
					<TabsList className="h-auto flex-wrap justify-start">
						<TabsTrigger value="overview">
							{t("overview", "Overview")}
						</TabsTrigger>
						<TabsTrigger value="nodes">
							{t("nodesLength", "Nodes ({{length}})", {
								length: pkg.nodes?.length ?? 0,
							})}
						</TabsTrigger>
						{widgets.length > 0 && (
							<TabsTrigger value="widgets">
								{t("widgetsLength", "Widgets ({{length}})", {
									length: widgets.length,
								})}
							</TabsTrigger>
						)}
						<TabsTrigger value="permissions">
							{t("permissions", "Permissions")}
						</TabsTrigger>
						<TabsTrigger value="versions">
							{t("versionsLength", "Versions ({{length}})", {
								length: versions.length,
							})}
						</TabsTrigger>
						<TabsTrigger value="reviews">{t("reviews", "Reviews")}</TabsTrigger>
					</TabsList>

					<TabsContent value="overview" className="space-y-4">
						{/* Long Description */}
						{meta?.longDescription && (
							<Card className="gap-3">
								<CardHeader>
									<CardTitle className="text-base">
										{t("about", "About")}
									</CardTitle>
								</CardHeader>
								<CardContent>
									<PackageMarkdown content={meta.longDescription} />
								</CardContent>
							</Card>
						)}

						{/* Use Case */}
						{meta?.useCase && (
							<Card className="border-border/60">
								<CardHeader className="pb-3">
									<div className="flex items-center gap-3">
										<div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
											<Target className="h-4 w-4" />
										</div>
										<div className="min-w-0">
											<CardTitle className="text-base">
												{t("useCase", "Use Case")}
											</CardTitle>
											<CardDescription>
												{t(
													"whereThisPackageFitsBest",
													"Where this package fits best",
												)}
											</CardDescription>
										</div>
									</div>
								</CardHeader>
								<CardContent>
									<p className="whitespace-pre-wrap break-words text-sm leading-6 text-foreground/90">
										{meta.useCase}
									</p>
								</CardContent>
							</Card>
						)}

						<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
							{/* Info Card */}
							<Card className="md:col-span-2">
								<CardHeader>
									<CardTitle className="text-base">
										{t("packageInformation", "Package Information")}
									</CardTitle>
								</CardHeader>
								<CardContent className="space-y-4">
									{(() => {
										const tags = meta?.tags?.length
											? meta.tags
											: manifest.keywords;
										if (!tags.length) return null;
										return (
											<div>
												<h4 className="text-sm font-medium mb-2">
													{t("tags", "Tags")}
												</h4>
												<div className="flex flex-wrap gap-1">
													{tags.map((t) => (
														<Badge key={t} variant="outline">
															{t}
														</Badge>
													))}
												</div>
											</div>
										);
									})()}

									{(manifest.authors?.length ?? 0) > 0 && (
										<div>
											<h4 className="text-sm font-medium mb-2">
												{t("authors", "Authors")}
											</h4>
											<div className="flex flex-wrap gap-2">
												{manifest.authors?.map((author) => (
													<div
														key={`${author.name}:${author.email ?? ""}:${author.url ?? ""}`}
														className="flex items-center gap-1 text-sm"
													>
														<User className="h-4 w-4 text-muted-foreground" />
														{author.url ? (
															<a
																href={author.url}
																target="_blank"
																rel="noopener noreferrer"
																className="hover:underline"
															>
																{author.name}
															</a>
														) : (
															<span>{author.name}</span>
														)}
													</div>
												))}
											</div>
										</div>
									)}

									{manifest.license && (
										<div>
											<h4 className="text-sm font-medium mb-2">
												{t("license", "License")}
											</h4>
											<Badge variant="outline">{manifest.license}</Badge>
										</div>
									)}
								</CardContent>
							</Card>

							{/* Links Card */}
							<Card>
								<CardHeader>
									<CardTitle className="text-base">
										{t("links", "Links")}
									</CardTitle>
								</CardHeader>
								<CardContent className="space-y-3">
									{manifest.repository && (
										<a
											href={manifest.repository}
											target="_blank"
											rel="noopener noreferrer"
											className="flex items-center gap-2 text-sm hover:underline"
										>
											<Github className="h-4 w-4" />
											{t("repository", "Repository")}
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{(meta?.website || manifest.homepage) && (
										<a
											href={meta?.website || manifest.homepage}
											target="_blank"
											rel="noopener noreferrer"
											className="flex items-center gap-2 text-sm hover:underline"
										>
											<Globe className="h-4 w-4" />
											{t("website", "Website")}
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{meta?.docsUrl && (
										<a
											href={meta.docsUrl}
											target="_blank"
											rel="noopener noreferrer"
											className="flex items-center gap-2 text-sm hover:underline"
										>
											<BookOpen className="h-4 w-4" />
											{t("documentation", "Documentation")}
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{meta?.supportUrl && (
										<a
											href={meta.supportUrl}
											target="_blank"
											rel="noopener noreferrer"
											className="flex items-center gap-2 text-sm hover:underline"
										>
											<HelpCircle className="h-4 w-4" />
											{t("support", "Support")}
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{!manifest.repository &&
										!manifest.homepage &&
										!meta?.website &&
										!meta?.docsUrl &&
										!meta?.supportUrl && (
											<p className="text-sm text-muted-foreground">
												{t(
													"noExternalLinksProvided",
													"No external links provided",
												)}
											</p>
										)}
								</CardContent>
							</Card>
						</div>

						{/* Stats Card */}
						<Card>
							<CardHeader>
								<CardTitle className="text-base">
									{t("statistics", "Statistics")}
								</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="grid grid-cols-2 md:grid-cols-4 gap-4">
									<div>
										<p className="text-2xl font-bold">
											{(pkg.downloadCount ?? 0).toLocaleString()}
										</p>
										<p className="text-sm text-muted-foreground">
											{t("totalDownloads", "Total Downloads")}
										</p>
									</div>
									<div>
										<p className="text-2xl font-bold">{versions.length}</p>
										<p className="text-sm text-muted-foreground">
											{t("versions", "Versions")}
										</p>
									</div>
									<div>
										<p className="text-2xl font-bold">
											{pkg.nodes?.length ?? 0}
										</p>
										<p className="text-sm text-muted-foreground">
											{t("nodes2", "Nodes")}
										</p>
									</div>
									<div>
										<p className="text-2xl font-bold">
											{(pkg.ratingCount ?? 0) > 0
												? (pkg.avgRating ?? 0).toFixed(1)
												: "N/A"}
										</p>
										<p className="text-sm text-muted-foreground">
											{t("avgRating", "Avg Rating")}
											{(pkg.ratingCount ?? 0) > 0 && ` (${pkg.ratingCount})`}
										</p>
									</div>
								</div>
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="nodes" className="space-y-4">
						{!pkg.nodes?.length ? (
							<Card className="p-8 text-center">
								<FileCode className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
								<h3 className="font-semibold">
									{t("noNodesDeclared", "No nodes declared")}
								</h3>
								<p className="text-muted-foreground text-sm">
									{t(
										"thisPackageDoesnapostHaveAnyExtractedNodesYet",
										"This package doesn't have any extracted nodes yet",
									)}
								</p>
							</Card>
						) : (
							<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
								{pkg.nodes?.map((node) => (
									<NodeCard key={node.id} node={node} />
								))}
							</div>
						)}
					</TabsContent>

					{widgets.length > 0 && (
						<TabsContent value="widgets" className="space-y-4">
							<WidgetCardGrid
								widgets={widgets}
								packageId={pkg.id}
								packageVersion={latestVersion}
								bundleHash={widgetBundleHash}
							/>
						</TabsContent>
					)}

					<TabsContent value="permissions" className="space-y-4">
						<Card>
							<CardHeader>
								<CardTitle className="text-base">
									{t("resourceLimits", "Resource Limits")}
								</CardTitle>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="grid grid-cols-2 gap-4">
									<div>
										<p className="text-sm font-medium">
											{t("memory", "Memory")}
										</p>
										<Badge variant="outline" className="mt-1">
											{manifest.permissions?.memory}
										</Badge>
									</div>
									<div>
										<p className="text-sm font-medium">
											{t("timeout", "Timeout")}
										</p>
										<Badge variant="outline" className="mt-1">
											{manifest.permissions?.timeout}
										</Badge>
									</div>
								</div>
							</CardContent>
						</Card>

						<Card>
							<CardHeader>
								<CardTitle className="text-base">
									{t("capabilities", "Capabilities")}
								</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="flex flex-wrap gap-2">
									<PermissionBadge
										label={t("httpRequests", "HTTP Requests")}
										enabled={manifest.permissions?.network?.httpEnabled}
									/>
									<PermissionBadge
										label={t("websocket", "WebSocket")}
										enabled={manifest.permissions?.network?.websocketEnabled}
									/>
									<PermissionBadge
										label={t("nodeStorage", "Node Storage")}
										enabled={manifest.permissions?.filesystem?.nodeStorage}
									/>
									<PermissionBadge
										label={t("userStorage", "User Storage")}
										enabled={manifest.permissions?.filesystem?.userStorage}
									/>
									<PermissionBadge
										label={t("variables", "Variables")}
										enabled={manifest.permissions?.variables}
									/>
									<PermissionBadge
										label={t("cache", "Cache")}
										enabled={manifest.permissions?.cache}
									/>
									<PermissionBadge
										label={t("streaming", "Streaming")}
										enabled={manifest.permissions?.streaming}
									/>
									<PermissionBadge
										label="A2UI"
										enabled={manifest.permissions?.a2ui}
									/>
									<PermissionBadge
										label={t("modelsLlm", "Models/LLM")}
										enabled={manifest.permissions?.models}
									/>
								</div>

								{manifest.permissions?.network?.httpEnabled &&
									(manifest.permissions?.network?.allowedHosts?.length ?? 0) >
										0 && (
										<div className="mt-4">
											<p className="text-sm font-medium mb-2">
												{t("allowedHosts", "Allowed Hosts")}
											</p>
											<div className="flex flex-wrap gap-1">
												{manifest.permissions?.network?.allowedHosts?.map(
													(host) => (
														<Badge
															key={host}
															variant="outline"
															className="font-mono text-xs"
														>
															{host}
														</Badge>
													),
												)}
											</div>
										</div>
									)}

								{(manifest.permissions?.oauthScopes?.length ?? 0) > 0 && (
									<div className="mt-4">
										<p className="text-sm font-medium mb-2">
											{t("oauthScopes", "OAuth Scopes")}
										</p>
										{manifest.permissions?.oauthScopes?.map((oauth) => (
											<div
												key={`${oauth.provider}:${oauth.scopes.join(",")}:${oauth.reason}`}
												className="p-3 rounded-lg bg-muted mt-2"
											>
												<div className="flex items-center gap-2">
													<Badge>{oauth.provider}</Badge>
													{oauth.required && (
														<Badge variant="destructive">
															{t("required", "Required")}
														</Badge>
													)}
												</div>
												<p className="text-sm mt-1">{oauth.reason}</p>
												<div className="flex flex-wrap gap-1 mt-2">
													{oauth.scopes.map((scope) => (
														<Badge
															key={scope}
															variant="outline"
															className="font-mono text-xs"
														>
															{scope}
														</Badge>
													))}
												</div>
											</div>
										))}
									</div>
								)}
							</CardContent>
						</Card>

						<WidgetNetworkAccessCard widgets={widgets} />
					</TabsContent>

					<TabsContent value="versions" className="space-y-4">
						<Card>
							<CardHeader>
								<CardTitle className="text-base">
									{t("versionHistory", "Version History")}
								</CardTitle>
							</CardHeader>
							<CardContent>
								{versions.length === 0 ? (
									<p className="text-sm text-muted-foreground">
										{t("noVersionsAvailable", "No versions available")}
									</p>
								) : (
									<div className="divide-y">
										{versions.map((v, idx) => (
											<VersionRow
												key={v.version}
												version={v}
												isLatest={idx === 0}
											/>
										))}
									</div>
								)}
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="reviews" className="space-y-4">
						<PackageReviewsTab packageId={pkg.id} />
					</TabsContent>
				</Tabs>
			</div>
		</main>
	);
}
