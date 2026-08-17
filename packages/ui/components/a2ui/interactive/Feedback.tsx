"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2, ThumbsDown, ThumbsUp } from "lucide-react";
import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { pageLocalState } from "../../../lib/idb-storage";
import {
	getPageContextStorageId,
	queryParamsToRecord,
} from "../../../lib/page-context";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Label } from "../../ui/label";
import { Textarea } from "../../ui/textarea";
import {
	useActionContext,
	useComponentEventTrigger,
	useExecuteAction,
	useIsComponentTriggering,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveEventActions } from "../event-handlers";
import type { BoundValue, FeedbackComponent } from "../types";

type FeedbackMode = "icon" | "compact" | "segmented" | "rating" | "extended";
type FeedbackCommentMode = "none" | "inline" | "modal";

interface StoredFeedbackSelection {
	rating: number;
	comment: string;
	feedbackId: string;
	surfaceId: string;
	pathname: string;
	search: string;
	queryParams: Record<string, string | string[]>;
	updatedAt: string;
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function normalizeMode(mode: string | undefined): FeedbackMode {
	switch (mode) {
		case "icon":
		case "iconOnly":
			return "icon";
		case "segmented":
			return "segmented";
		case "rating":
		case "scale":
			return "rating";
		case "extended":
			return "extended";
		case "compact":
		default:
			return "compact";
	}
}

function normalizeCommentMode(
	mode: string | undefined,
	feedbackMode: FeedbackMode,
	showComment: boolean,
): FeedbackCommentMode {
	if (!showComment) return "none";

	switch (mode) {
		case "none":
		case "off":
			return "none";
		case "inline":
			return "inline";
		case "modal":
		case "dialog":
			return "modal";
		default:
			return feedbackMode === "extended" ? "inline" : "modal";
	}
}

function normalizeRating(value: unknown, fallback: number) {
	const rating = Number(value);
	return Number.isFinite(rating) ? rating : fallback;
}

function getRatingRange(lowValue: number, highValue: number) {
	let low = Math.round(Math.max(0, Math.min(5, Math.min(lowValue, highValue))));
	let high = Math.round(
		Math.max(0, Math.min(5, Math.max(lowValue, highValue))),
	);

	if (low === high) {
		if (high < 5) {
			high += 1;
		} else {
			low -= 1;
		}
	}

	return Array.from({ length: high - low + 1 }, (_, index) => low + index);
}

function getThumbButtonVariant(
	selected: boolean,
	iconOnly: boolean,
	segmented: boolean,
): "outline" | "secondary" | "ghost" {
	if (iconOnly || segmented) return "ghost";
	return selected ? "secondary" : "outline";
}

export function A2UIFeedback({
	component,
	style,
	componentId,
}: ComponentProps<FeedbackComponent>) {
	const { t } = useTranslation("common");
	const [comment, setComment] = useState("");
	const [pendingRating, setPendingRating] = useState<number | null>(null);
	const [isCommentDialogOpen, setIsCommentDialogOpen] = useState(false);
	const [selectedRating, setSelectedRating] = useState<number | null>(null);
	const [storedFeedback, setStoredFeedback] =
		useState<StoredFeedbackSelection | null>(null);
	const { executeAction, isPreviewMode } = useExecuteAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const { appId, eventId, surfaceId = "" } = useActionContext();
	const isTriggering = useIsComponentTriggering(componentId);
	const pathname = usePathname() ?? "/";
	const searchParams = useSearchParams();
	const search = searchParams?.toString() ?? "";

	const mode = normalizeMode(useResolved<string>(component.mode));
	const size = useResolved<string>(component.size) ?? "md";
	const disabled = useResolved<boolean>(component.disabled) ?? false;
	const showComment =
		useResolved<boolean>(component.showComment) ?? mode === "extended";
	const title = useResolved<string>(component.title) ?? t('wasThisHelpful', 'Was this helpful?');
	const description = useResolved<string>(component.description);
	const positiveLabel =
		useResolved<string>(component.positiveLabel) ??
		(mode === "compact" || mode === "icon"
			? "Yes"
			: mode === "rating"
				? "Great"
				: "Helpful");
	const negativeLabel =
		useResolved<string>(component.negativeLabel) ??
		(mode === "compact" || mode === "icon"
			? "No"
			: mode === "rating"
				? "Poor"
				: "Needs work");
	const commentLabel = useResolved<string>(component.commentLabel) ?? "Comment";
	const commentPlaceholder =
		useResolved<string>(component.commentPlaceholder) ?? "Optional comment";
	const commentMode = normalizeCommentMode(
		useResolved<string>(component.commentMode),
		mode,
		showComment,
	);
	const commentTitle =
		useResolved<string>(component.commentTitle) ?? t('addFeedback', 'Add feedback');
	const commentDescription =
		useResolved<string>(component.commentDescription) ??
		t('shareALittleMoreContextBeforeSubmitting', 'Share a little more context before submitting.');
	const commentSubmitLabel =
		useResolved<string>(component.commentSubmitLabel) ?? t('submitFeedback', 'Submit feedback');
	const commentCancelLabel =
		useResolved<string>(component.commentCancelLabel) ?? "Cancel";
	const feedbackId = useResolved<string>(component.feedbackId) || componentId;
	const positiveRating = normalizeRating(
		useResolved<unknown>(component.positiveRating),
		5,
	);
	const negativeRating = normalizeRating(
		useResolved<unknown>(component.negativeRating),
		1,
	);
	const includeState = useResolved<boolean>(component.includeState) ?? true;
	const pageContextMode =
		useResolved<string>(component.pageContextMode) ?? "path";
	const pageContextQueryParamAllowlist =
		useResolved<string>(component.pageContextQueryParamAllowlist) ?? "";
	const pageContextQueryParamDenylist =
		useResolved<string>(component.pageContextQueryParamDenylist) ?? "";
	const includePageHash =
		useResolved<boolean>(component.includePageHash) ?? false;
	const successMessage =
		useResolved<string>(component.successMessage) ?? t('thanksForTheFeedback', 'Thanks for the feedback.');
	const feedbackStoragePageId = useMemo(
		() => getPageContextStorageId(pathname, search),
		[pathname, search],
	);
	const feedbackStorageKey = useMemo(
		() => `feedback:${surfaceId}:${feedbackId}`,
		[surfaceId, feedbackId],
	);

	const buttonSize = size === "sm" ? "sm" : size === "lg" ? "lg" : "default";
	const iconButtonSizeClass =
		size === "sm" ? "size-8" : size === "lg" ? "size-10" : "size-9";
	const ratingValues = getRatingRange(negativeRating, positiveRating);

	useEffect(() => {
		if (!appId || !feedbackStoragePageId || !feedbackStorageKey) {
			setStoredFeedback(null);
			return;
		}

		let cancelled = false;
		pageLocalState
			.get<StoredFeedbackSelection>(
				appId,
				feedbackStoragePageId,
				feedbackStorageKey,
			)
			.then((stored) => {
				if (cancelled) return;
				setStoredFeedback(stored);
				if (!stored) {
					setSelectedRating(null);
					setComment("");
					return;
				}
				setSelectedRating(stored.rating);
				setComment(stored.comment ?? "");
			})
			.catch((error) => {
				if (!cancelled) {
					console.warn("[A2UI Feedback] Failed to load saved feedback:", error);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [appId, feedbackStoragePageId, feedbackStorageKey]);

	const submit = async (rating: number, nextComment = comment) => {
		if (disabled || isTriggering) return false;
		if (
			storedFeedback?.rating === rating &&
			(storedFeedback.comment ?? "") === nextComment
		) {
			return true;
		}

		const previousRating = selectedRating;
		setSelectedRating(rating);
		try {
			const feedbackContext = {
				rating,
				feedbackId,
				comment: nextComment,
				includeState,
				pageContextMode,
				pageContextQueryParamAllowlist,
				pageContextQueryParamDenylist,
				includePageHash,
				successMessage,
			};
			const actionResolution = resolveEventActions(
				component.eventHandlers,
				"submit",
				component.actions,
			);
			if (actionResolution.source === "none") {
				await executeAction(
					{
						name: "submit_feedback",
						context: feedbackContext,
					},
					componentId,
				);
			} else {
				await triggerEvent("submit", component, feedbackContext);
			}

			if (!appId || !eventId || !isPreviewMode) return true;

			const nextStoredFeedback: StoredFeedbackSelection = {
				rating,
				comment: nextComment,
				feedbackId,
				surfaceId,
				pathname,
				search,
				queryParams: queryParamsToRecord(search),
				updatedAt: new Date().toISOString(),
			};

			setStoredFeedback(nextStoredFeedback);
			try {
				await pageLocalState.set(
					appId,
					feedbackStoragePageId,
					feedbackStorageKey,
					nextStoredFeedback,
				);
			} catch (storageError) {
				console.warn(
					"[A2UI Feedback] Failed to save feedback locally:",
					storageError,
				);
			}
			return true;
		} catch (error) {
			setSelectedRating(previousRating);
			console.error("[A2UI Feedback] Failed to submit feedback:", error);
			return false;
		}
	};

	const requestSubmit = (rating: number) => {
		if (commentMode === "modal") {
			setPendingRating(rating);
			setIsCommentDialogOpen(true);
			return;
		}

		void submit(rating);
	};

	const submitPendingFeedback = () => {
		if (pendingRating === null) return;
		void submit(pendingRating).then((submitted) => {
			if (submitted) {
				setIsCommentDialogOpen(false);
				setPendingRating(null);
			}
		});
	};

	const renderThumbButtons = ({
		iconOnly = false,
		segmented = false,
	}: { iconOnly?: boolean; segmented?: boolean } = {}) => (
		<div
			role="group"
			aria-label={title}
			className={cn(
				"flex items-center",
				segmented
					? "inline-flex overflow-hidden rounded-full border border-border bg-background p-0.5 shadow-sm"
					: iconOnly
						? "inline-flex gap-1"
						: "flex-wrap gap-2",
			)}
		>
			<Button
				type="button"
				variant={getThumbButtonVariant(
					selectedRating === positiveRating,
					iconOnly,
					segmented,
				)}
				size={iconOnly ? "icon" : buttonSize}
				aria-label={positiveLabel}
				aria-pressed={selectedRating === positiveRating}
				disabled={disabled || isTriggering}
				onClick={() => requestSubmit(positiveRating)}
				className={cn(
					"gap-2 transition-all",
					iconOnly &&
						cn(
							iconButtonSizeClass,
							"rounded-full border border-transparent text-muted-foreground shadow-none hover:bg-muted/80 hover:text-foreground",
						),
					segmented &&
						"rounded-full border-0 px-3 text-muted-foreground shadow-none hover:bg-muted/80 hover:text-foreground",
					selectedRating === positiveRating &&
						"border-primary/20 bg-primary/10 text-primary ring-1 ring-primary/20 hover:bg-primary/15 hover:text-primary dark:bg-primary/20 dark:text-primary-foreground dark:hover:bg-primary/25",
				)}
			>
				{isTriggering && selectedRating === positiveRating ? (
					<Loader2 className="h-4 w-4 animate-spin" />
				) : (
					<ThumbsUp
						className={cn(
							"h-4 w-4",
							selectedRating === positiveRating && "fill-current",
						)}
					/>
				)}
				{!iconOnly && positiveLabel}
			</Button>
			<Button
				type="button"
				variant={getThumbButtonVariant(
					selectedRating === negativeRating,
					iconOnly,
					segmented,
				)}
				size={iconOnly ? "icon" : buttonSize}
				aria-label={negativeLabel}
				aria-pressed={selectedRating === negativeRating}
				disabled={disabled || isTriggering}
				onClick={() => requestSubmit(negativeRating)}
				className={cn(
					"gap-2 transition-all",
					iconOnly &&
						cn(
							iconButtonSizeClass,
							"rounded-full border border-transparent text-muted-foreground shadow-none hover:bg-muted/80 hover:text-foreground",
						),
					segmented &&
						"rounded-full border-0 px-3 text-muted-foreground shadow-none hover:bg-muted/80 hover:text-foreground",
					selectedRating === negativeRating &&
						"border-primary/20 bg-primary/10 text-primary ring-1 ring-primary/20 hover:bg-primary/15 hover:text-primary dark:bg-primary/20 dark:text-primary-foreground dark:hover:bg-primary/25",
				)}
			>
				{isTriggering && selectedRating === negativeRating ? (
					<Loader2 className="h-4 w-4 animate-spin" />
				) : (
					<ThumbsDown
						className={cn(
							"h-4 w-4",
							selectedRating === negativeRating && "fill-current",
						)}
					/>
				)}
				{!iconOnly && negativeLabel}
			</Button>
		</div>
	);

	const ratingControls = (
		<div className="w-full space-y-2">
			<div
				className="grid gap-2"
				style={{
					gridTemplateColumns: `repeat(${ratingValues.length}, minmax(0, 1fr))`,
				}}
			>
				{ratingValues.map((rating) => (
					<Button
						key={rating}
						type="button"
						variant={selectedRating === rating ? "default" : "outline"}
						size={buttonSize}
						aria-label={`${title}: ${rating}`}
						aria-pressed={selectedRating === rating}
						disabled={disabled || isTriggering}
						onClick={() => requestSubmit(rating)}
					>
						{isTriggering && selectedRating === rating ? (
							<Loader2 className="h-4 w-4 animate-spin" />
						) : (
							rating
						)}
					</Button>
				))}
			</div>
			<div className="flex justify-between gap-4 text-xs text-muted-foreground">
				<span>{negativeLabel}</span>
				<span>{positiveLabel}</span>
			</div>
		</div>
	);

	const controls =
		mode === "rating"
			? ratingControls
			: renderThumbButtons({
					iconOnly: mode === "icon",
					segmented: mode === "segmented",
				});
	const commentDialog = commentMode === "modal" && (
		<Dialog
			open={isCommentDialogOpen}
			onOpenChange={(open) => {
				setIsCommentDialogOpen(open);
				if (!open) setPendingRating(null);
			}}
		>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{commentTitle}</DialogTitle>
					{commentDescription && (
						<DialogDescription>{commentDescription}</DialogDescription>
					)}
				</DialogHeader>
				<DialogBody className="space-y-2">
					<Label>{commentLabel}</Label>
					<Textarea
						value={comment}
						onChange={(event) => setComment(event.target.value)}
						placeholder={commentPlaceholder}
						disabled={disabled || isTriggering}
						rows={4}
					/>
				</DialogBody>
				<DialogFooter>
					<Button
						type="button"
						variant="outline"
						disabled={isTriggering}
						onClick={() => {
							setIsCommentDialogOpen(false);
							setPendingRating(null);
						}}
					>
						{commentCancelLabel}
					</Button>
					<Button
						type="button"
						disabled={disabled || isTriggering || pendingRating === null}
						onClick={submitPendingFeedback}
					>
						{isTriggering ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
						{commentSubmitLabel}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);

	if (mode === "extended" || mode === "rating" || commentMode === "inline") {
		return (
			<>
				<div
					className={cn(
						"w-full max-w-md rounded-lg border border-border bg-card p-4 shadow-sm",
						resolveStyle(style),
					)}
					style={resolveInlineStyle(style)}
				>
					<div className="space-y-1">
						<p className="text-sm font-medium text-card-foreground">{title}</p>
						{description && (
							<p className="text-sm text-muted-foreground">{description}</p>
						)}
					</div>
					{commentMode === "inline" && (
						<div className="mt-3 space-y-1.5">
							<Label className="text-xs text-muted-foreground">
								{commentLabel}
							</Label>
							<Textarea
								value={comment}
								onChange={(event) => setComment(event.target.value)}
								placeholder={commentPlaceholder}
								disabled={disabled || isTriggering}
								rows={3}
							/>
						</div>
					)}
					<div
						className={cn(
							"mt-3",
							mode === "rating" ? "w-full" : "flex justify-end",
						)}
					>
						{controls}
					</div>
				</div>
				{commentDialog}
			</>
		);
	}

	if (mode === "icon") {
		return (
			<>
				<div
					className={cn(
						"inline-flex items-center rounded-full border border-border bg-background/95 p-1 shadow-sm ring-1 ring-black/5 dark:ring-white/5",
						resolveStyle(style),
					)}
					style={resolveInlineStyle(style)}
					aria-label={title}
				>
					{controls}
				</div>
				{commentDialog}
			</>
		);
	}

	if (mode === "segmented") {
		return (
			<>
				<div
					className={cn(
						"inline-flex items-center gap-3 rounded-md border border-border bg-background px-3 py-2 shadow-sm",
						resolveStyle(style),
					)}
					style={resolveInlineStyle(style)}
				>
					<span className="text-sm font-medium text-muted-foreground">
						{title}
					</span>
					{controls}
				</div>
				{commentDialog}
			</>
		);
	}

	return (
		<>
			<div
				className={cn(
					"inline-flex items-center gap-2 rounded-full border border-border bg-background px-3 py-2 shadow-sm",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				<span className="text-sm font-medium text-muted-foreground">
					{title}
				</span>
				{controls}
			</div>
			{commentDialog}
		</>
	);
}
