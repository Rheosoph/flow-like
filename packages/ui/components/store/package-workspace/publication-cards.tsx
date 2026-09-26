"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, RefreshCw, Send } from "lucide-react";
import { asArray } from "../../../lib/response-shape";
import {
	type PackageReview,
	PackageStatus,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
} from "../../../lib/user-display";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	RelativeTime,
	Skeleton,
} from "../../ui";
import { invalidatePackageLists, useHubProfile } from "./use-workspace-data";

function formatReviewAction(action: PackageReview["action"]) {
	return action.replaceAll("_", " ");
}

function getReviewerLabel(review: PackageReview) {
	return userDisplayName(review.reviewer, review.reviewerId);
}

export function PublicationReviewCard({
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
	const { t } = useTranslation("store");
	const profile = useHubProfile();

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

	const reviews = asArray(reviewQuery.data);
	const statusLabel =
		status === PackageStatus.PendingReview
			? t("pendingReview", "Pending Review")
			: status === PackageStatus.Disabled
				? t("reviewOutcomeAvailable", "Review outcome available")
				: t("reviewHistory", "Review history");

	return (
		<Card className="border-amber-500/30 bg-amber-500/5">
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<RefreshCw className="h-4 w-4" />
					{t("publicationReview", "Publication Review")}
				</CardTitle>
				<CardDescription>
					{t(
						"currentStatusStatuslabelSubmissionEventsAndAuditorCommentsAppearHereForPackageMaintainers",
						"Current status: {{statusLabel}}. Submission events and auditor comments appear here for package maintainers.",
						{ statusLabel },
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{reviewQuery.isLoading ? (
					<Skeleton className="h-24 w-full" />
				) : reviewQuery.isError ? (
					<p className="text-sm text-destructive">
						{reviewQuery.error?.message ??
							t("failedToLoadReviewHistory", "Failed to load review history")}
					</p>
				) : reviews.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t(
							"noPublicationReviewEventsRecordedYet",
							"No publication review events recorded yet.",
						)}
					</p>
				) : (
					<div className="space-y-3">
						{reviews.map((review) => {
							const reviewerLabel = getReviewerLabel(review);
							const reviewerAvatar = userAvatarUrl(review.reviewer);

							return (
								<div
									key={review.id}
									className="rounded-lg border bg-background/80 p-4"
								>
									<div className="flex items-start gap-3">
										<Avatar className="h-9 w-9">
											{reviewerAvatar ? (
												<AvatarImage src={reviewerAvatar} alt={reviewerLabel} />
											) : null}
											<AvatarFallback>
												{userInitials(review.reviewer)}
											</AvatarFallback>
										</Avatar>
										<div className="min-w-0 flex-1 space-y-1">
											<div className="flex flex-wrap items-center gap-2">
												<span className="font-medium capitalize">
													{formatReviewAction(review.action)}
												</span>
												<span className="text-sm text-muted-foreground">
													{t("byReviewerlabel", "by {{reviewerLabel}}", {
														reviewerLabel,
													})}
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

export function PublicationRequestCard({
	packageId,
	fetcher,
	auth,
}: {
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
}) {
	const { t } = useTranslation("store");
	const queryClient = useQueryClient();
	const profile = useHubProfile();

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
			invalidatePackageLists(queryClient, packageId);
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
					{t("requestPublication", "Request Publication")}
				</CardTitle>
				<CardDescription>
					{t(
						"thisPackageIsCurrentlyPrivateSubmitItForReviewToMakeItPubliclyAvailableOnTheRegistry",
						"This package is currently private. Submit it for review to make it publicly available on the registry.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{requestMutation.isSuccess ? (
					<div className="flex items-center gap-2 text-sm text-green-600">
						<Check className="h-4 w-4" />
						{t(
							"publicationReviewRequestedWeWillReviewYourPackageAndNotifyYouOnceADecisionHasBeenMade",
							"Publication review requested. We will review your package and notify you once a decision has been made.",
						)}
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
							{t("requestPublicationReview", "Request Publication Review")}
						</Button>
						{requestMutation.isError && (
							<p className="text-sm text-destructive">
								{requestMutation.error?.message ??
									t(
										"failedToRequestPublication",
										"Failed to request publication",
									)}
							</p>
						)}
					</div>
				)}
			</CardContent>
		</Card>
	);
}
