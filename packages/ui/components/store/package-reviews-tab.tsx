"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatDistanceToNow } from "date-fns";
import { Loader2, MessageSquare, Star, Trash2 } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import type {
	PackageCommentItem,
	PackageCommentsResponse,
	UpsertPackageCommentRequest,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	Separator,
	Textarea,
} from "../ui";

const PAGE_SIZE = 10;

function StarRating({
	value,
	onChange,
	readonly = false,
}: {
	value: number;
	onChange?: (v: number) => void;
	readonly?: boolean;
}) {
	const [hovered, setHovered] = useState(0);
	return (
		<div className="flex items-center gap-0.5">
			{[1, 2, 3, 4, 5].map((star) => {
				const filled = readonly ? star <= value : star <= (hovered || value);
				return (
					<button
						key={star}
						type="button"
						disabled={readonly}
						className={
							readonly
								? "cursor-default"
								: "cursor-pointer hover:scale-110 transition-transform"
						}
						onMouseEnter={() => !readonly && setHovered(star)}
						onMouseLeave={() => !readonly && setHovered(0)}
						onClick={() => onChange?.(star)}
					>
						<Star
							className={`h-4 w-4 ${
								filled
									? "text-yellow-500 fill-yellow-500"
									: "text-muted-foreground"
							}`}
						/>
					</button>
				);
			})}
		</div>
	);
}

function ReviewItem({
	comment,
	onDelete,
	isDeleting,
}: {
	comment: PackageCommentItem;
	onDelete?: () => void;
	isDeleting?: boolean;
}) {
	return (
		<div className="flex gap-3 py-4">
			<Avatar className="h-8 w-8 shrink-0">
				{comment.userAvatar && (
					<AvatarImage src={comment.userAvatar} alt={comment.userName ?? ""} />
				)}
				<AvatarFallback className="text-xs">
					{(comment.userName ?? "U").charAt(0).toUpperCase()}
				</AvatarFallback>
			</Avatar>
			<div className="flex-1 min-w-0 space-y-1">
				<div className="flex items-center justify-between gap-2">
					<div className="flex items-center gap-2">
						<span className="text-sm font-medium">
							{comment.userName ?? "Anonymous"}
						</span>
						<StarRating value={comment.rating} readonly />
						<span className="text-xs text-muted-foreground">
							{formatDistanceToNow(new Date(comment.createdAt), {
								addSuffix: true,
							})}
						</span>
					</div>
					{onDelete && (
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7 text-muted-foreground hover:text-destructive"
							onClick={onDelete}
							disabled={isDeleting}
						>
							{isDeleting ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<Trash2 className="h-3.5 w-3.5" />
							)}
						</Button>
					)}
				</div>
				{comment.text && (
					<p className="text-sm text-muted-foreground">{comment.text}</p>
				)}
			</div>
		</div>
	);
}

export interface PackageReviewsTabProps {
	packageId: string;
}

export function PackageReviewsTab({ packageId }: PackageReviewsTabProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [page, setPage] = useState(0);
	const [rating, setRating] = useState(0);
	const [text, setText] = useState("");
	const [deletingId, setDeletingId] = useState<string | null>(null);

	const queryKey = ["package-comments", packageId, page];

	const { data, isLoading } = useQuery<PackageCommentsResponse>({
		queryKey,
		queryFn: () =>
			backend.registryState.getPackageComments(
				packageId,
				page * PAGE_SIZE,
				PAGE_SIZE,
			),
	});

	const upsertMutation = useMutation({
		mutationFn: (body: UpsertPackageCommentRequest) =>
			backend.registryState.upsertPackageComment(packageId, body),
		onSuccess: () => {
			toast.success("Review submitted");
			setRating(0);
			setText("");
			queryClient.invalidateQueries({
				queryKey: ["package-comments", packageId],
			});
			queryClient.invalidateQueries({
				queryKey: ["registry-package", packageId],
			});
		},
		onError: () => toast.error("Failed to submit review"),
	});

	const deleteMutation = useMutation({
		mutationFn: (commentId: string) =>
			backend.registryState.deletePackageComment(packageId, commentId),
		onSuccess: () => {
			toast.success("Review deleted");
			setDeletingId(null);
			queryClient.invalidateQueries({
				queryKey: ["package-comments", packageId],
			});
			queryClient.invalidateQueries({
				queryKey: ["registry-package", packageId],
			});
		},
		onError: () => {
			toast.error("Failed to delete review");
			setDeletingId(null);
		},
	});

	const handleSubmit = useCallback(() => {
		if (rating < 1) {
			toast.error("Please select a rating");
			return;
		}
		upsertMutation.mutate({ text, rating });
	}, [rating, text, upsertMutation]);

	const totalPages = Math.max(1, Math.ceil((data?.total ?? 0) / PAGE_SIZE));
	const comments = data?.comments ?? [];

	return (
		<div className="space-y-4">
			{/* Submit Review */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Write a Review</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<div className="flex items-center gap-2">
						<span className="text-sm text-muted-foreground">Rating:</span>
						<StarRating value={rating} onChange={setRating} />
					</div>
					<Textarea
						placeholder="Share your experience with this package (optional)"
						value={text}
						onChange={(e) => setText(e.target.value)}
						rows={3}
					/>
					<Button
						onClick={handleSubmit}
						disabled={upsertMutation.isPending || rating < 1}
						size="sm"
					>
						{upsertMutation.isPending && (
							<Loader2 className="mr-2 h-4 w-4 animate-spin" />
						)}
						Submit Review
					</Button>
				</CardContent>
			</Card>

			{/* Reviews List */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<MessageSquare className="h-4 w-4" />
						Reviews
						{data && (
							<span className="text-sm font-normal text-muted-foreground">
								({data.total})
							</span>
						)}
					</CardTitle>
				</CardHeader>
				<CardContent>
					{isLoading ? (
						<div className="flex items-center justify-center py-8">
							<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
						</div>
					) : comments.length === 0 ? (
						<div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
							<MessageSquare className="h-10 w-10 mb-2" />
							<p className="text-sm">No reviews yet. Be the first!</p>
						</div>
					) : (
						<div className="divide-y">
							{comments.map((comment) => (
								<ReviewItem
									key={comment.id}
									comment={comment}
									onDelete={() => {
										setDeletingId(comment.id);
										deleteMutation.mutate(comment.id);
									}}
									isDeleting={
										deletingId === comment.id && deleteMutation.isPending
									}
								/>
							))}
						</div>
					)}

					{totalPages > 1 && (
						<>
							<Separator className="my-4" />
							<div className="flex items-center justify-center gap-2">
								<Button
									variant="outline"
									size="sm"
									disabled={page === 0}
									onClick={() => setPage((p) => p - 1)}
								>
									Previous
								</Button>
								<span className="text-sm text-muted-foreground">
									Page {page + 1} of {totalPages}
								</span>
								<Button
									variant="outline"
									size="sm"
									disabled={page >= totalPages - 1}
									onClick={() => setPage((p) => p + 1)}
								>
									Next
								</Button>
							</div>
						</>
					)}
				</CardContent>
			</Card>
		</div>
	);
}
