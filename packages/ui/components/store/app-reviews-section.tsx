"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, MessageSquare, Star } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useBackend } from "../../state/backend-state";
import type {
	AppCommentItem,
	AppCommentsResponse,
	UpsertAppCommentRequest,
} from "../../state/backend-state/app-state";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	RelativeTime,
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
	onChange?: (value: number) => void;
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
								: "cursor-pointer transition-transform hover:scale-110"
						}
						onMouseEnter={() => !readonly && setHovered(star)}
						onMouseLeave={() => !readonly && setHovered(0)}
						onClick={() => onChange?.(star)}
					>
						<Star
							className={`h-4 w-4 ${
								filled
									? "fill-yellow-500 text-yellow-500"
									: "text-muted-foreground"
							}`}
						/>
					</button>
				);
			})}
		</div>
	);
}

function ReviewItem({ comment }: { comment: AppCommentItem }) {
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
			<div className="min-w-0 flex-1 space-y-1">
				<div className="flex flex-wrap items-center gap-2">
					<span className="text-sm font-medium">
						{comment.userName ?? "Anonymous"}
					</span>
					<StarRating value={comment.rating} readonly />
					<RelativeTime
						className="text-xs text-muted-foreground"
						value={comment.createdAt}
					/>
				</div>
				{comment.text && (
					<p className="text-sm text-muted-foreground">{comment.text}</p>
				)}
			</div>
		</div>
	);
}

export interface AppReviewsSectionProps {
	appId: string;
	onReviewChanged?: () => Promise<void> | void;
}

export function AppReviewsSection({
	appId,
	onReviewChanged,
}: AppReviewsSectionProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [page, setPage] = useState(0);
	const [rating, setRating] = useState(0);
	const [text, setText] = useState("");

	const offlineQuery = useQuery({
		queryKey: ["app-comments-offline", appId],
		queryFn: () => backend.isOffline(appId),
		enabled: !!appId,
	});

	const { data, isLoading } = useQuery<AppCommentsResponse>({
		queryKey: ["app-comments", appId, page],
		queryFn: () =>
			backend.appState.getAppComments(appId, page * PAGE_SIZE, PAGE_SIZE),
		enabled: !!appId && offlineQuery.data !== true,
	});

	const upsertMutation = useMutation({
		mutationFn: (body: UpsertAppCommentRequest) =>
			backend.appState.upsertAppComment(appId, body),
		onSuccess: async () => {
			toast.success("Review submitted");
			setRating(0);
			setText("");
			await queryClient.invalidateQueries({
				queryKey: ["app-comments", appId],
			});
			await onReviewChanged?.();
		},
		onError: (error) => {
			toast.error(
				error instanceof Error ? error.message : "Failed to submit review",
			);
		},
	});

	const handleSubmit = useCallback(() => {
		if (!appId) {
			toast.error("App not found");
			return;
		}

		if (rating < 1) {
			toast.error("Please select a rating");
			return;
		}

		upsertMutation.mutate({ text, rating });
	}, [rating, text, upsertMutation]);

	if (offlineQuery.data) {
		return (
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2 text-base">
						<MessageSquare className="h-4 w-4" />
						Reviews
					</CardTitle>
				</CardHeader>
				<CardContent>
					<p className="text-sm text-muted-foreground">
						Reviews are available once this app is published online.
					</p>
				</CardContent>
			</Card>
		);
	}

	const totalPages = Math.max(1, Math.ceil((data?.total ?? 0) / PAGE_SIZE));
	const comments = data?.comments ?? [];

	return (
		<div className="space-y-4">
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Rate This App</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<div className="flex items-center gap-2">
						<span className="text-sm text-muted-foreground">Rating:</span>
						<StarRating value={rating} onChange={setRating} />
					</div>
					<Textarea
						placeholder="Share your experience with this app (optional)"
						value={text}
						onChange={(event) => setText(event.target.value)}
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

			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2 text-base">
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
							<MessageSquare className="mb-2 h-10 w-10" />
							<p className="text-sm">No reviews yet. Be the first!</p>
						</div>
					) : (
						<div className="divide-y">
							{comments.map((comment) => (
								<ReviewItem key={comment.id} comment={comment} />
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
									onClick={() => setPage((currentPage) => currentPage - 1)}
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
									onClick={() => setPage((currentPage) => currentPage + 1)}
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
