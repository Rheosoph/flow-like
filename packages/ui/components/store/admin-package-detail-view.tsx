"use client";

import {
	ArrowLeft,
	CheckCircle,
	Clock,
	Download,
	ExternalLink,
	MessageSquare,
	Package,
	Shield,
	XCircle,
} from "lucide-react";
import { useCallback, useState } from "react";
import type {
	AdminPackageDetailResponse,
	PackageAdminStatus,
	PackageReview,
	PackageReviewer,
	ReviewRequest,
} from "../../lib/schema/wasm";
import { formatAbsoluteDateValue } from "../../lib";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Skeleton,
	Slider,
	RelativeTime,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
} from "../ui";

const statusBadgeVariant: Record<
	PackageAdminStatus,
	"default" | "secondary" | "destructive" | "outline"
> = {
	pending_review: "secondary",
	active: "default",
	rejected: "destructive",
	deprecated: "outline",
	disabled: "outline",
};

function formatDownloadCount(count: number | null | undefined) {
	return (count ?? 0).toLocaleString();
}

function ReviewItem({ review }: { review: PackageReview }) {
	const actionIcon = {
		submitted: <Package className="h-4 w-4" />,
		approve: <CheckCircle className="h-4 w-4 text-green-500" />,
		reject: <XCircle className="h-4 w-4 text-red-500" />,
		request_changes: <Clock className="h-4 w-4 text-yellow-500" />,
		comment: <MessageSquare className="h-4 w-4 text-blue-500" />,
		flag: <Shield className="h-4 w-4 text-orange-500" />,
	};

	const reviewer = review.reviewer;
	const reviewerLabel =
		reviewer?.name ?? reviewer?.username ?? review.reviewerId;
	const reviewerInitial = reviewerLabel.charAt(0).toUpperCase();

	return (
		<div className="flex gap-4 p-4 border rounded-lg">
			<div className="shrink-0 flex flex-col items-center gap-3">
				<div>{actionIcon[review.action]}</div>
				<div className="flex items-center gap-2 text-xs text-muted-foreground">
					<div className="flex h-8 w-8 items-center justify-center overflow-hidden rounded-full border bg-muted text-foreground">
						{reviewer?.avatar ? (
							<img
								src={reviewer.avatar}
								alt={reviewerLabel}
								className="h-full w-full object-cover"
							/>
						) : (
							reviewerInitial
						)}
					</div>
				</div>
			</div>
			<div className="flex-1">
				<div className="flex items-center gap-2">
					<span className="font-medium capitalize">
						{review.action.replace("_", " ")}
					</span>
					<span className="text-sm text-muted-foreground">
						by {reviewerLabel}
					</span>
					<span className="text-sm text-muted-foreground">
						<RelativeTime value={review.createdAt} />
					</span>
				</div>
				{review.comment && (
					<p className="mt-2 text-sm text-muted-foreground">{review.comment}</p>
				)}
				{(review.securityScore ||
					review.codeQualityScore ||
					review.documentationScore) && (
					<div className="flex gap-4 mt-2">
						{review.securityScore && (
							<div className="text-sm">
								<span className="text-muted-foreground">Security:</span>{" "}
								<span className="font-medium">{review.securityScore}/10</span>
							</div>
						)}
						{review.codeQualityScore && (
							<div className="text-sm">
								<span className="text-muted-foreground">Code Quality:</span>{" "}
								<span className="font-medium">
									{review.codeQualityScore}/10
								</span>
							</div>
						)}
						{review.documentationScore && (
							<div className="text-sm">
								<span className="text-muted-foreground">Documentation:</span>{" "}
								<span className="font-medium">
									{review.documentationScore}/10
								</span>
							</div>
						)}
					</div>
				)}
			</div>
		</div>
	);
}

export interface AdminPackageDetailViewProps {
	packageDetail: AdminPackageDetailResponse | null | undefined;
	isLoading: boolean;
	onBack: () => void;
	onSubmitReview: (review: ReviewRequest) => void;
	onUpdatePackage: (data: { status?: string; verified?: boolean }) => void;
	isSubmittingReview?: boolean;
	isUpdatingPackage?: boolean;
}

export function AdminPackageDetailView({
	packageDetail,
	isLoading,
	onBack,
	onSubmitReview,
	onUpdatePackage,
	isSubmittingReview,
	isUpdatingPackage,
}: AdminPackageDetailViewProps) {
	const [reviewAction, setReviewAction] =
		useState<ReviewRequest["action"]>("comment");
	const [reviewComment, setReviewComment] = useState("");
	const [securityScore, setSecurityScore] = useState<number[]>([5]);
	const [codeQualityScore, setCodeQualityScore] = useState<number[]>([5]);
	const [documentationScore, setDocumentationScore] = useState<number[]>([5]);

	const handleSubmitReview = useCallback(() => {
		const review: ReviewRequest = {
			action: reviewAction,
			comment: reviewComment || undefined,
			securityScore: reviewAction === "approve" ? securityScore[0] : undefined,
			codeQualityScore:
				reviewAction === "approve" ? codeQualityScore[0] : undefined,
			documentationScore:
				reviewAction === "approve" ? documentationScore[0] : undefined,
		};
		onSubmitReview(review);
		setReviewComment("");
	}, [
		reviewAction,
		reviewComment,
		securityScore,
		codeQualityScore,
		documentationScore,
		onSubmitReview,
	]);

	const pkg = packageDetail?.package;
	const reviews = packageDetail?.reviews ?? [];

	if (isLoading) {
		return (
			<div className="container mx-auto py-6 space-y-6">
				<Skeleton className="h-8 w-48" />
				<Skeleton className="h-64 w-full" />
			</div>
		);
	}

	if (!pkg) {
		return (
			<div className="container mx-auto py-6">
				<p>Package not found</p>
			</div>
		);
	}

	return (
		<div className="container mx-auto py-6 space-y-6">
			<div className="flex items-center gap-4">
				<Button variant="ghost" size="sm" onClick={onBack}>
					<ArrowLeft className="h-4 w-4 mr-2" />
					Back
				</Button>
			</div>

			<div className="flex items-start justify-between">
				<div>
					<h1 className="text-3xl font-bold flex items-center gap-3">
						{pkg.name}
						{pkg.verified && <Shield className="h-6 w-6 text-blue-500" />}
					</h1>
					<p className="text-muted-foreground mt-1">{pkg.description}</p>
					<div className="flex items-center gap-4 mt-2">
						<Badge variant={statusBadgeVariant[pkg.status]}>
							{pkg.status.replace("_", " ")}
						</Badge>
						<span className="text-sm text-muted-foreground">
							v{pkg.version}
						</span>
						<span className="text-sm text-muted-foreground flex items-center gap-1">
							<Download className="h-3 w-3" />
							{formatDownloadCount(pkg.downloadCount)} downloads
						</span>
					</div>
				</div>
				<div className="flex gap-2">
					{pkg.repository && (
						<Button variant="outline" size="sm" asChild>
							<a
								href={pkg.repository}
								target="_blank"
								rel="noopener noreferrer"
							>
								<ExternalLink className="h-4 w-4 mr-2" />
								Repository
							</a>
						</Button>
					)}
				</div>
			</div>

			<div className="grid gap-6 md:grid-cols-3">
				<div className="md:col-span-2 space-y-6">
					<Tabs defaultValue="details">
						<TabsList>
							<TabsTrigger value="details">Details</TabsTrigger>
							<TabsTrigger value="permissions">Permissions</TabsTrigger>
							<TabsTrigger value="nodes">
								Nodes ({(pkg.nodes as unknown[]).length})
							</TabsTrigger>
							<TabsTrigger value="reviews">
								Reviews ({reviews.length})
							</TabsTrigger>
						</TabsList>

						<TabsContent value="details" className="space-y-4 mt-4">
							<Card>
								<CardHeader>
									<CardTitle>Package Information</CardTitle>
								</CardHeader>
								<CardContent className="space-y-4">
									<div className="grid grid-cols-2 gap-4">
										<div>
											<Label className="text-muted-foreground">ID</Label>
											<p className="font-mono text-sm">{pkg.id}</p>
										</div>
										<div>
											<Label className="text-muted-foreground">Version</Label>
											<p>{pkg.version}</p>
										</div>
										<div>
											<Label className="text-muted-foreground">Authors</Label>
											<p>{pkg.authors.join(", ") || "Unknown"}</p>
										</div>
										<div>
											<Label className="text-muted-foreground">License</Label>
											<p>{pkg.license || "Not specified"}</p>
										</div>
										<div>
											<Label className="text-muted-foreground">WASM Size</Label>
											<p>{(pkg.wasmSize / 1024).toFixed(2)} KB</p>
										</div>
										<div>
											<Label className="text-muted-foreground">Submitter</Label>
											<p>{pkg.submitterId || "Unknown"}</p>
										</div>
										<div>
											<Label className="text-muted-foreground">Created</Label>
											<p>{formatAbsoluteDateValue(pkg.createdAt, "Unknown")}</p>
										</div>
										<div>
											<Label className="text-muted-foreground">Published</Label>
											<p>
												{pkg.publishedAt
													? formatAbsoluteDateValue(pkg.publishedAt, "Not published")
													: "Not published"}
											</p>
										</div>
									</div>
									{pkg.keywords.length > 0 && (
										<div>
											<Label className="text-muted-foreground">Keywords</Label>
											<div className="flex flex-wrap gap-1 mt-1">
												{pkg.keywords.map((kw) => (
													<Badge key={kw} variant="secondary">
														{kw}
													</Badge>
												))}
											</div>
										</div>
									)}
								</CardContent>
							</Card>
						</TabsContent>

						<TabsContent value="permissions" className="mt-4">
							<Card>
								<CardHeader>
									<CardTitle>Requested Permissions</CardTitle>
									<CardDescription>
										Review the permissions this package requests
									</CardDescription>
								</CardHeader>
								<CardContent>
									<pre className="text-sm bg-muted p-4 rounded-lg overflow-auto max-h-96">
										{JSON.stringify(pkg.permissions, null, 2)}
									</pre>
								</CardContent>
							</Card>
						</TabsContent>

						<TabsContent value="nodes" className="mt-4">
							<Card>
								<CardHeader>
									<CardTitle>Exported Nodes</CardTitle>
									<CardDescription>
										Custom nodes provided by this package
									</CardDescription>
								</CardHeader>
								<CardContent>
									<pre className="text-sm bg-muted p-4 rounded-lg overflow-auto max-h-96">
										{JSON.stringify(pkg.nodes, null, 2)}
									</pre>
								</CardContent>
							</Card>
						</TabsContent>

						<TabsContent value="reviews" className="mt-4 space-y-4">
							{reviews.length === 0 ? (
								<p className="text-muted-foreground">No reviews yet</p>
							) : (
								reviews.map((review) => (
									<ReviewItem key={review.id} review={review} />
								))
							)}
						</TabsContent>
					</Tabs>
				</div>

				<div className="space-y-6">
					<Card>
						<CardHeader>
							<CardTitle>Actions</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="space-y-2">
								<Label>Quick Actions</Label>
								<div className="flex flex-col gap-2">
									<Button
										variant="default"
										className="w-full"
										disabled={pkg.status === "active" || isSubmittingReview}
										onClick={() => onSubmitReview({ action: "approve" })}
									>
										<CheckCircle className="h-4 w-4 mr-2" />
										Approve
									</Button>
									<Button
										variant="destructive"
										className="w-full"
										disabled={pkg.status === "rejected" || isSubmittingReview}
										onClick={() => onSubmitReview({ action: "reject" })}
									>
										<XCircle className="h-4 w-4 mr-2" />
										Reject
									</Button>
								</div>
							</div>

							<Separator />

							<div className="space-y-2">
								<Label>Verified Status</Label>
								<Button
									variant="outline"
									className="w-full"
									disabled={isUpdatingPackage}
									onClick={() => onUpdatePackage({ verified: !pkg.verified })}
								>
									<Shield className="h-4 w-4 mr-2" />
									{pkg.verified ? "Remove Verified" : "Mark as Verified"}
								</Button>
							</div>
						</CardContent>
					</Card>

					<Card>
						<CardHeader>
							<CardTitle>Submit Review</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="space-y-2">
								<Label>Action</Label>
								<Select
									value={reviewAction}
									onValueChange={(v) =>
										setReviewAction(v as ReviewRequest["action"])
									}
								>
									<SelectTrigger>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="approve">Approve</SelectItem>
										<SelectItem value="reject">Reject</SelectItem>
										<SelectItem value="request_changes">
											Request Changes
										</SelectItem>
										<SelectItem value="comment">Comment</SelectItem>
										<SelectItem value="flag">Flag for Review</SelectItem>
									</SelectContent>
								</Select>
							</div>

							<div className="space-y-2">
								<Label>Comment</Label>
								<Textarea
									value={reviewComment}
									onChange={(e) => setReviewComment(e.target.value)}
									placeholder="Add your review comments..."
									rows={4}
								/>
							</div>

							{reviewAction === "approve" && (
								<>
									<div className="space-y-2">
										<Label>Security Score: {securityScore[0]}/10</Label>
										<Slider
											value={securityScore}
											onValueChange={setSecurityScore}
											max={10}
											min={1}
											step={1}
										/>
									</div>
									<div className="space-y-2">
										<Label>Code Quality Score: {codeQualityScore[0]}/10</Label>
										<Slider
											value={codeQualityScore}
											onValueChange={setCodeQualityScore}
											max={10}
											min={1}
											step={1}
										/>
									</div>
									<div className="space-y-2">
										<Label>
											Documentation Score: {documentationScore[0]}/10
										</Label>
										<Slider
											value={documentationScore}
											onValueChange={setDocumentationScore}
											max={10}
											min={1}
											step={1}
										/>
									</div>
								</>
							)}

							<Button
								className="w-full"
								onClick={handleSubmitReview}
								disabled={isSubmittingReview}
							>
								Submit Review
							</Button>
						</CardContent>
					</Card>
				</div>
			</div>
		</div>
	);
}
