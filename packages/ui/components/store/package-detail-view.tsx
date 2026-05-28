"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
	Package,
	RefreshCw,
	RotateCcw,
	Send,
	Settings,
	Shield,
	ShoppingCart,
	Star,
	Tag,
	Target,
	Trash2,
	User,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import {
	isMaintainer,
	isOwner,
} from "../../lib/permission/wasm-package-permission";
import {
	type PackageMeta,
	type PackageReview,
	type PackageVersion,
	PackageStatus,
	type RegistryEntry,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
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
import { PackageAccessTab } from "./package-access-tab";
import { PackageMetaTab } from "./package-meta-tab";
import { PackageReviewsTab } from "./package-reviews-tab";
import { PackageUsersContainer } from "./package-users-container";

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
	canInstall,
	installedVersion,
	isInstalling,
	onInstall,
}: {
	version: PackageVersion;
	isLatest: boolean;
	canInstall: boolean;
	installedVersion?: string | null;
	isInstalling?: boolean;
	onInstall?: (version: string) => void;
}) {
	const isInstalled = installedVersion === version.version;
	const isPending = version.status === PackageStatus.PendingReview;
	const isRejected = version.status === PackageStatus.Rejected;
	const isDisabled = version.status === PackageStatus.Disabled;
	const isVersionInstallable = !version.yanked && !isRejected && !isDisabled;

	return (
		<div className="flex items-center justify-between gap-3 py-2 border-b last:border-0">
			<div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
				<code className="text-sm font-mono">{version.version}</code>
				{isLatest && <Badge variant="secondary">Latest</Badge>}
				{isPending && <Badge variant="outline">Pending Review</Badge>}
				{isRejected && <Badge variant="destructive">Rejected</Badge>}
				{isDisabled && <Badge variant="secondary">Disabled</Badge>}
				{version.yanked && <Badge variant="destructive">Yanked</Badge>}
			</div>
			<div className="flex shrink-0 items-center gap-3">
				<RelativeTime
					className="text-sm text-muted-foreground"
					value={version.publishedAt}
				/>
				{canInstall && isVersionInstallable && onInstall && (
					<Button
						size="sm"
						variant={isInstalled ? "secondary" : "outline"}
						disabled={isInstalled || isInstalling}
						onClick={() => onInstall(version.version)}
					>
						{isInstalled ? (
							<Check className="mr-2 h-3.5 w-3.5" />
						) : (
							<Download className="mr-2 h-3.5 w-3.5" />
						)}
						{isInstalled
							? "Installed"
							: isPending
								? "Install for testing"
								: "Install"}
					</Button>
				)}
			</div>
		</div>
	);
}

function formatReviewAction(action: PackageReview["action"]) {
	return action.replaceAll("_", " ");
}

function getReviewerLabel(review: PackageReview) {
	return (
		review.reviewer?.name ?? review.reviewer?.username ?? review.reviewerId
	);
}

function PublicationReviewCard({
	packageId,
	status,
	fetcher,
	auth,
}: {
	packageId: string;
	status: RegistryEntry["status"];
	fetcher: GenericFetcher;
	auth?: unknown;
}) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const reviewQuery = useQuery({
		queryKey: ["package-publication-reviews", packageId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return fetcher<PackageReview[]>(
				profile.data.hub_profile,
				`registry/package/${packageId}/publication-reviews`,
				{ method: "GET" },
				auth,
			);
		},
		enabled: !!profile.data,
		retry: false,
	});

	const reviews = reviewQuery.data ?? [];
	const statusLabel =
		status === PackageStatus.PendingReview
			? "Pending review"
			: status === PackageStatus.Disabled
				? "Review outcome available"
				: "Review history";

	return (
		<Card className="border-amber-500/30 bg-amber-500/5">
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<RefreshCw className="h-4 w-4" />
					Publication Review
				</CardTitle>
				<CardDescription>
					Current status: {statusLabel}. Submission events and auditor comments
					appear here for package maintainers.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{reviewQuery.isLoading ? (
					<Skeleton className="h-24 w-full" />
				) : reviewQuery.isError ? (
					<p className="text-sm text-destructive">
						{reviewQuery.error?.message ?? "Failed to load review history"}
					</p>
				) : reviews.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						No publication review events recorded yet.
					</p>
				) : (
					<div className="space-y-3">
						{reviews.map((review) => {
							const reviewerLabel = getReviewerLabel(review);
							const reviewerInitial = reviewerLabel.charAt(0).toUpperCase();

							return (
								<div
									key={review.id}
									className="rounded-lg border bg-background/80 p-4"
								>
									<div className="flex items-start gap-3">
										<Avatar className="h-9 w-9">
											{review.reviewer?.avatar ? (
												<AvatarImage
													src={review.reviewer.avatar}
													alt={reviewerLabel}
												/>
											) : null}
											<AvatarFallback>{reviewerInitial}</AvatarFallback>
										</Avatar>
										<div className="min-w-0 flex-1 space-y-1">
											<div className="flex flex-wrap items-center gap-2">
												<span className="font-medium capitalize">
													{formatReviewAction(review.action)}
												</span>
												<span className="text-sm text-muted-foreground">
													by {reviewerLabel}
												</span>
												<span className="text-sm text-muted-foreground">
													<RelativeTime value={review.createdAt} />
												</span>
											</div>
											{review.comment && (
												<p className="text-sm text-muted-foreground">
													{review.comment}
												</p>
											)}
										</div>
									</div>
								</div>
							);
						})}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function PublicationRequestCard({
	packageId,
	fetcher,
	auth,
}: {
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
}) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const requestMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return fetcher<{ message: string }>(
				profile.data.hub_profile,
				`registry/package/${packageId}/request-publication`,
				{ method: "POST" },
				auth,
			);
		},
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["registry-package", packageId],
			});
			queryClient.invalidateQueries({
				queryKey: ["admin", "packages"],
			});
			queryClient.invalidateQueries({
				queryKey: ["admin", "packages", "publications"],
			});
		},
	});

	return (
		<Card className="border-primary/30 bg-primary/5">
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<Send className="h-4 w-4" />
					Request Publication
				</CardTitle>
				<CardDescription>
					This package is currently private. Submit it for review to make it
					publicly available on the registry.
				</CardDescription>
			</CardHeader>
			<CardContent>
				{requestMutation.isSuccess ? (
					<div className="flex items-center gap-2 text-sm text-green-600">
						<Check className="h-4 w-4" />
						Publication review requested. We will review your package and notify
						you once a decision has been made.
					</div>
				) : (
					<div className="flex items-center gap-3">
						<Button
							onClick={() => requestMutation.mutate()}
							disabled={requestMutation.isPending}
						>
							{requestMutation.isPending ? (
								<Loader2 className="mr-2 h-4 w-4 animate-spin" />
							) : (
								<Send className="mr-2 h-4 w-4" />
							)}
							Request Publication Review
						</Button>
						{requestMutation.isError && (
							<p className="text-sm text-destructive">
								{requestMutation.error?.message ??
									"Failed to request publication"}
							</p>
						)}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

export interface PackageDetailViewProps {
	pkg: RegistryEntry | null | undefined;
	isLoading: boolean;
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
	onBuy?: () => void;
	onGetOrBuy?: () => void;
	onDeleteSuccess?: () => void;
	currentUserPermission?: number;
	fetcher?: GenericFetcher;
	auth?: unknown;
}

export function PackageDetailView(props: PackageDetailViewProps) {
	const {
		pkg,
		isLoading,
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
		onBuy,
		onGetOrBuy,
		onDeleteSuccess,
		currentUserPermission,
		fetcher,
		auth,
	} = props;

	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [showDeleteDialog, setShowDeleteDialog] = useState(false);

	const deleteMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data || !pkg?.id || !fetcher)
				throw new Error("Missing context");
			return fetcher<{ message: string }>(
				profile.data.hub_profile,
				`registry/package/${pkg.id}`,
				{ method: "DELETE" },
				auth,
			);
		},
		onSuccess: (data) => {
			toast.success(data.message);
			onDeleteSuccess?.();
		},
		onError: (err: Error) =>
			toast.error(`Failed to delete package: ${err.message}`),
	});

	const restoreMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data || !pkg?.id || !fetcher)
				throw new Error("Missing context");
			return fetcher<{ message: string }>(
				profile.data.hub_profile,
				`registry/package/${pkg.id}/restore`,
				{ method: "POST" },
				auth,
			);
		},
		onSuccess: (data) => {
			toast.success(data.message);
			queryClient.invalidateQueries({
				queryKey: ["registry-package", pkg?.id],
			});
			onDeleteSuccess?.();
		},
		onError: (err: Error) =>
			toast.error(`Failed to restore package: ${err.message}`),
	});

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

	if (isLoading || !pkg) {
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

	const manifest = pkg.manifest;
	const canManagePublication =
		currentUserPermission != null &&
		isMaintainer(currentUserPermission) &&
		!!fetcher;
	const hasPendingVersion = pkg.versions.some(
		(version) => version.status === PackageStatus.PendingReview,
	);
	const latestVersion =
		pkg.versions.find(
			(v) =>
				!v.yanked &&
				v.status !== PackageStatus.Rejected &&
				v.status !== PackageStatus.Disabled,
		)?.version ?? pkg.versions[0]?.version;
	const canInstallForReview =
		canManagePublication &&
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
			? "Pending review"
			: pkg.status === PackageStatus.Disabled
				? "Disabled"
				: pkg.status === PackageStatus.Rejected
					? "Rejected"
					: "Unavailable";
	const unavailableActionMessage = isInstalled
		? `Updates are unavailable while this package is ${unavailableActionLabel.toLowerCase()}.`
		: `Install is unavailable while this package is ${unavailableActionLabel.toLowerCase()}.`;
	const showPublicationAudit =
		canManagePublication &&
		(pkg.status !== PackageStatus.Active || hasPendingVersion);
	const showPublicationRequest =
		currentUserPermission != null &&
		isOwner(currentUserPermission) &&
		visibility === "private" &&
		pkg.status === PackageStatus.Active &&
		!!fetcher;

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-5xl space-y-6">
				{/* Back Button */}
				<Button variant="ghost" onClick={onBack} className="gap-2">
					<ArrowLeft className="h-4 w-4" />
					Back
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
												Disabled
											</Badge>
										)}
										{pkg.status === PackageStatus.PendingReview && (
											<Badge variant="secondary" className="gap-1">
												Pending Review
											</Badge>
										)}
										{pkg.status === PackageStatus.Rejected && (
											<Badge variant="destructive" className="gap-1">
												Rejected
											</Badge>
										)}
										{pkg.verified && (
											<Badge variant="secondary" className="gap-1">
												<Shield className="h-3 w-3" />
												Verified
											</Badge>
										)}
									</div>
									<CardDescription className="mt-1">
										{meta?.description || manifest.description}
									</CardDescription>
									<div className="flex items-center gap-4 mt-2 text-sm text-muted-foreground">
										<span className="flex items-center gap-1">
											<Tag className="h-4 w-4" />v{latestVersion}
										</span>
										<span className="flex items-center gap-1">
											<Download className="h-4 w-4" />
											{(pkg.downloadCount ?? 0).toLocaleString()} downloads
										</span>
										{(pkg.ratingCount ?? 0) > 0 && (
											<span className="flex items-center gap-1">
												<Star className="h-4 w-4 text-yellow-500 fill-yellow-500" />
												{(pkg.avgRating ?? 0).toFixed(1)}
												<span className="text-xs">({pkg.ratingCount})</span>
											</span>
										)}
										{price != null && price > 0 ? (
											<span className="flex items-center gap-1 font-medium">
												{priceLabel}
											</span>
										) : priceLabel ? (
											<span className="flex items-center gap-1 font-medium">
												Free
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
											Installed v{installedVersion}
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
												Update to v{latestVersion}
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
											Uninstall
										</Button>
									</>
								) : hasAccess === false && price != null && price > 0 ? (
									<Button onClick={onBuy} disabled={isPurchasing}>
										{isPurchasing ? (
											"Processing..."
										) : (
											<>
												<ShoppingCart className="mr-2 h-4 w-4" />
												{priceLabel || `€${(price / 100).toFixed(2)}`}
											</>
										)}
									</Button>
								) : hasAccess === false &&
									visibility === "public_request_access" ? (
									<Button onClick={onGetOrBuy} disabled={isRequesting}>
										{isRequesting ? (
											"Requesting..."
										) : (
											<>
												<KeyRound className="mr-2 h-4 w-4" />
												Request access
											</>
										)}
									</Button>
								) : hasAccess === false ? (
									<Button onClick={onGetOrBuy} disabled={isRequesting}>
										{isRequesting ? (
											"Processing..."
										) : (
											<>
												<Download className="mr-2 h-4 w-4" />
												Get
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
										{canInstallForReview ? "Install for testing" : "Install"}
									</Button>
								)}
							</div>
						</div>
					</CardHeader>
				</Card>

				{/* Main Content */}
				<Tabs defaultValue="overview" className="w-full">
					<TabsList className="h-auto flex-wrap justify-start">
						<TabsTrigger value="overview">Overview</TabsTrigger>
						<TabsTrigger value="nodes">
							Nodes ({pkg.nodes?.length ?? 0})
						</TabsTrigger>
						<TabsTrigger value="permissions">Permissions</TabsTrigger>
						<TabsTrigger value="versions">
							Versions ({pkg.versions.length})
						</TabsTrigger>
						<TabsTrigger value="reviews">Reviews</TabsTrigger>
						{currentUserPermission != null &&
							isMaintainer(currentUserPermission) &&
							fetcher && (
								<>
									<TabsTrigger value="access">Access Requests</TabsTrigger>
									<TabsTrigger value="users">Users</TabsTrigger>
									<TabsTrigger value="metadata" className="gap-1">
										<Settings className="h-3.5 w-3.5" />
										Metadata
									</TabsTrigger>
								</>
							)}
					</TabsList>

					<TabsContent value="overview" className="space-y-4">
						{/* Long Description */}
						{meta?.longDescription && (
							<Card className="gap-3">
								<CardHeader>
									<CardTitle className="text-base">About</CardTitle>
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
											<CardTitle className="text-base">Use Case</CardTitle>
											<CardDescription>
												Where this package fits best
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
										Package Information
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
												<h4 className="text-sm font-medium mb-2">Tags</h4>
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
											<h4 className="text-sm font-medium mb-2">Authors</h4>
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
											<h4 className="text-sm font-medium mb-2">License</h4>
											<Badge variant="outline">{manifest.license}</Badge>
										</div>
									)}
								</CardContent>
							</Card>

							{/* Links Card */}
							<Card>
								<CardHeader>
									<CardTitle className="text-base">Links</CardTitle>
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
											Repository
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
											Website
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
											Documentation
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
											Support
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{!manifest.repository &&
										!manifest.homepage &&
										!meta?.website &&
										!meta?.docsUrl &&
										!meta?.supportUrl && (
											<p className="text-sm text-muted-foreground">
												No external links provided
											</p>
										)}
								</CardContent>
							</Card>
						</div>

						{/* Stats Card */}
						<Card>
							<CardHeader>
								<CardTitle className="text-base">Statistics</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="grid grid-cols-2 md:grid-cols-4 gap-4">
									<div>
										<p className="text-2xl font-bold">
											{(pkg.downloadCount ?? 0).toLocaleString()}
										</p>
										<p className="text-sm text-muted-foreground">
											Total Downloads
										</p>
									</div>
									<div>
										<p className="text-2xl font-bold">{pkg.versions.length}</p>
										<p className="text-sm text-muted-foreground">Versions</p>
									</div>
									<div>
										<p className="text-2xl font-bold">
											{pkg.nodes?.length ?? 0}
										</p>
										<p className="text-sm text-muted-foreground">Nodes</p>
									</div>
									<div>
										<p className="text-2xl font-bold">
											{(pkg.ratingCount ?? 0) > 0
												? (pkg.avgRating ?? 0).toFixed(1)
												: "N/A"}
										</p>
										<p className="text-sm text-muted-foreground">
											Avg Rating
											{(pkg.ratingCount ?? 0) > 0 && ` (${pkg.ratingCount})`}
										</p>
									</div>
								</div>
							</CardContent>
						</Card>

						{/* Publication review state for maintainers */}
						{showPublicationAudit && fetcher && (
							<PublicationReviewCard
								packageId={pkg.id}
								status={pkg.status}
								fetcher={fetcher}
								auth={auth}
							/>
						)}

						{/* Request Publication - visible to owners of eligible private packages */}
						{showPublicationRequest && fetcher && (
							<PublicationRequestCard
								packageId={pkg.id}
								fetcher={fetcher}
								auth={auth}
							/>
						)}

						{/* Package Management - visible to owners */}
						{currentUserPermission != null &&
							isOwner(currentUserPermission) &&
							fetcher &&
							pkg.status === PackageStatus.Disabled && (
								<Card className="border-primary/30">
									<CardHeader>
										<CardTitle className="text-base flex items-center gap-2">
											<RotateCcw className="h-4 w-4" />
											Package Disabled
										</CardTitle>
									</CardHeader>
									<CardContent className="flex items-center justify-between">
										<div>
											<p className="text-sm font-medium">
												Restore this package
											</p>
											<p className="text-sm text-muted-foreground">
												This package is currently disabled and hidden from
												search. Restore it to make it active again.
											</p>
										</div>
										<Button
											size="sm"
											className="gap-1.5 shrink-0 ml-4"
											onClick={() => restoreMutation.mutate()}
											disabled={restoreMutation.isPending}
										>
											{restoreMutation.isPending ? (
												<Loader2 className="h-4 w-4 animate-spin" />
											) : (
												<RotateCcw className="h-4 w-4" />
											)}
											Restore
										</Button>
									</CardContent>
								</Card>
							)}

						{/* Delete Package - visible to owners, only when not already disabled */}
						{currentUserPermission != null &&
							isOwner(currentUserPermission) &&
							fetcher &&
							pkg.status !== PackageStatus.Disabled && (
								<Card className="border-destructive/30">
									<CardHeader>
										<CardTitle className="text-base text-destructive">
											Danger Zone
										</CardTitle>
									</CardHeader>
									<CardContent className="flex items-center justify-between">
										<div>
											<p className="text-sm font-medium">Delete this package</p>
											<p className="text-sm text-muted-foreground">
												The package will be disabled and hidden from search.
												Existing installs will keep working.
											</p>
										</div>
										<AlertDialog
											open={showDeleteDialog}
											onOpenChange={setShowDeleteDialog}
										>
											<AlertDialogTrigger asChild>
												<Button
													variant="destructive"
													size="sm"
													className="gap-1.5 shrink-0 ml-4"
												>
													<Trash2 className="h-4 w-4" />
													Delete
												</Button>
											</AlertDialogTrigger>
											<AlertDialogContent>
												<AlertDialogHeader>
													<AlertDialogTitle>Delete package?</AlertDialogTitle>
													<AlertDialogDescription>
														This will disable{" "}
														<strong>{meta?.name || manifest.name}</strong> and
														remove it from search results. Existing installs and
														offline projects will continue to work.
													</AlertDialogDescription>
												</AlertDialogHeader>
												<AlertDialogFooter>
													<AlertDialogCancel>Cancel</AlertDialogCancel>
													<AlertDialogAction
														className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
														onClick={() => deleteMutation.mutate()}
														disabled={deleteMutation.isPending}
													>
														{deleteMutation.isPending ? (
															<Loader2 className="mr-2 h-4 w-4 animate-spin" />
														) : (
															<Trash2 className="mr-2 h-4 w-4" />
														)}
														Delete Package
													</AlertDialogAction>
												</AlertDialogFooter>
											</AlertDialogContent>
										</AlertDialog>
									</CardContent>
								</Card>
							)}
					</TabsContent>

					<TabsContent value="nodes" className="space-y-4">
						{!pkg.nodes?.length ? (
							<Card className="p-8 text-center">
								<FileCode className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
								<h3 className="font-semibold">No nodes declared</h3>
								<p className="text-muted-foreground text-sm">
									This package doesn&apos;t have any extracted nodes yet
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

					<TabsContent value="permissions" className="space-y-4">
						<Card>
							<CardHeader>
								<CardTitle className="text-base">Resource Limits</CardTitle>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="grid grid-cols-2 gap-4">
									<div>
										<p className="text-sm font-medium">Memory</p>
										<Badge variant="outline" className="mt-1">
											{manifest.permissions?.memory}
										</Badge>
									</div>
									<div>
										<p className="text-sm font-medium">Timeout</p>
										<Badge variant="outline" className="mt-1">
											{manifest.permissions?.timeout}
										</Badge>
									</div>
								</div>
							</CardContent>
						</Card>

						<Card>
							<CardHeader>
								<CardTitle className="text-base">Capabilities</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="flex flex-wrap gap-2">
									<PermissionBadge
										label="HTTP Requests"
										enabled={manifest.permissions?.network?.httpEnabled}
									/>
									<PermissionBadge
										label="WebSocket"
										enabled={manifest.permissions?.network?.websocketEnabled}
									/>
									<PermissionBadge
										label="Node Storage"
										enabled={manifest.permissions?.filesystem?.nodeStorage}
									/>
									<PermissionBadge
										label="User Storage"
										enabled={manifest.permissions?.filesystem?.userStorage}
									/>
									<PermissionBadge
										label="Variables"
										enabled={manifest.permissions?.variables}
									/>
									<PermissionBadge
										label="Cache"
										enabled={manifest.permissions?.cache}
									/>
									<PermissionBadge
										label="Streaming"
										enabled={manifest.permissions?.streaming}
									/>
									<PermissionBadge
										label="A2UI"
										enabled={manifest.permissions?.a2ui}
									/>
									<PermissionBadge
										label="Models/LLM"
										enabled={manifest.permissions?.models}
									/>
								</div>

								{manifest.permissions?.network?.httpEnabled &&
									(manifest.permissions?.network?.allowedHosts?.length ?? 0) >
										0 && (
										<div className="mt-4">
											<p className="text-sm font-medium mb-2">Allowed Hosts</p>
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
										<p className="text-sm font-medium mb-2">OAuth Scopes</p>
										{manifest.permissions?.oauthScopes?.map((oauth) => (
											<div
												key={`${oauth.provider}:${oauth.scopes.join(",")}:${oauth.reason}`}
												className="p-3 rounded-lg bg-muted mt-2"
											>
												<div className="flex items-center gap-2">
													<Badge>{oauth.provider}</Badge>
													{oauth.required && (
														<Badge variant="destructive">Required</Badge>
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
					</TabsContent>

					<TabsContent value="versions" className="space-y-4">
						<Card>
							<CardHeader>
								<CardTitle className="text-base">Version History</CardTitle>
							</CardHeader>
							<CardContent>
								{pkg.versions.length === 0 ? (
									<p className="text-sm text-muted-foreground">
										No versions available
									</p>
								) : (
									<div className="divide-y">
										{pkg.versions.map((v, idx) => (
											<VersionRow
												key={v.version}
												version={v}
												isLatest={idx === 0}
												canInstall={canManagePublication}
												installedVersion={installedVersion}
												isInstalling={isInstalling}
												onInstall={onInstall}
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

					{currentUserPermission != null &&
						isMaintainer(currentUserPermission) &&
						fetcher && (
							<>
								<TabsContent value="access" className="space-y-4">
									<PackageAccessTab
										packageId={pkg.id}
										fetcher={fetcher}
										auth={auth}
									/>
								</TabsContent>
								<TabsContent value="users" className="space-y-4">
									<PackageUsersContainer
										packageId={pkg.id}
										fetcher={fetcher}
										auth={auth}
										currentUserPermission={currentUserPermission}
									/>
								</TabsContent>
								<TabsContent value="metadata" className="space-y-4">
									<PackageMetaTab
										packageId={pkg.id}
										fetcher={fetcher}
										auth={auth}
									/>
								</TabsContent>
							</>
						)}
				</Tabs>
			</div>
		</main>
	);
}
