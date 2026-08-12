"use client";

import {
	CheckIcon,
	ChevronDown,
	ChevronUp,
	CopyIcon,
	EditIcon,
	MessageSquareIcon,
	ThumbsDownIcon,
	ThumbsUpIcon,
	XIcon,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { IRole, cn } from "../../../lib";
import { FLOWPILOT_DEBUG_ENABLED } from "../../../lib/flowpilot-debug";
import { observeResize } from "../../../lib/observe-resize";
import {
	Badge,
	Button,
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Label,
	Switch,
	TextEditor,
	Textarea,
} from "../../ui";
import { StreamingTextEditor } from "../../ui/streaming-text-editor";
import { AgentDebugReport } from "./agent-debug-report";
import { AppReferences } from "./app-references";
import { type ProcessedAttachment, getDisplayFileName } from "./attachment";
import {
	FileDialog,
	FileDialogPreview,
	canPreviewFile,
	downloadFile,
} from "./attachment-dialog";
import { AttachmentStrip } from "./attachment-strip";
import type { IAttachment, IMessage } from "./chat-db";
import { useProcessedAttachments } from "./hooks/use-processed-attachments";
import { buildInlineSegments } from "./inline-segments";
import { MessageWidgets } from "./message-widgets";
import { InlineStepGroup, PlanSteps } from "./plan-steps";
import { UsageStats } from "./usage-stats";

/** Lines of a sent message shown before it collapses behind "Show more". */
const COLLAPSED_ASK_LINES = 6;

function ThinkingIndicator() {
	return (
		<div className="flex items-center gap-1.5 py-1">
			<div className="flex gap-1">
				<span className="w-1.5 h-1.5 rounded-full bg-muted-foreground/60 animate-bounce [animation-delay:0ms]" />
				<span className="w-1.5 h-1.5 rounded-full bg-muted-foreground/60 animate-bounce [animation-delay:150ms]" />
				<span className="w-1.5 h-1.5 rounded-full bg-muted-foreground/60 animate-bounce [animation-delay:300ms]" />
			</div>
			<span className="text-xs text-muted-foreground ml-1">Thinking...</span>
		</div>
	);
}

interface MessageProps {
	message: IMessage;
	loading?: boolean;
	onMessageUpdate?: (
		messageId: string,
		updates: Partial<IMessage>,
	) => void | Promise<void>;
	/** App id owning the chat — needed to render + trigger embedded widgets. */
	appId?: string;
	/** Board id of the chat event — target for widget action workflows. */
	boardId?: string;
	/** Chat event id — forwarded to embedded widget surfaces. */
	eventId?: string;
}

const MessageActionButton = ({
	onClick,
	children,
	className,
	title,
}: {
	onClick: () => void;
	children: React.ReactNode;
	className?: string;
	title?: string;
}) => (
	<button
		onClick={onClick}
		className={cn(
			"text-muted-foreground hover:text-foreground transition-colors",
			className,
		)}
		title={title}
	>
		{children}
	</button>
);

const FeedbackButton = ({
	onClick,
	isActive,
	variant = "positive",
	children,
}: {
	onClick: () => void;
	isActive: boolean;
	variant?: "positive" | "negative";
	children: React.ReactNode;
}) => (
	<button
		onClick={onClick}
		className={cn(
			"transition-colors",
			isActive
				? variant === "positive"
					? "text-emerald-500 dark:text-emerald-400"
					: "text-red-500 dark:text-red-400"
				: "text-muted-foreground hover:text-foreground",
		)}
	>
		{children}
	</button>
);

const FullscreenEditDialog = ({
	open,
	onOpenChange,
	content,
	onSave,
	appId,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	content: string;
	onSave: (content: string) => void;
	appId?: string;
}) => {
	const [editedContent, setEditedContent] = useState(content);

	useEffect(() => {
		if (open) {
			setEditedContent(content);
			document.body.style.overflow = "hidden";
		} else {
			document.body.style.overflow = "";
		}

		return () => {
			document.body.style.overflow = "";
		};
	}, [open, content]);

	const handleSave = useCallback(() => {
		onSave(editedContent);
		onOpenChange(false);
	}, [editedContent, onSave, onOpenChange]);

	const handleCancel = useCallback(() => {
		setEditedContent(content);
		onOpenChange(false);
	}, [content, onOpenChange]);

	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				handleCancel();
			}
			if (e.key === "s" && (e.metaKey || e.ctrlKey)) {
				e.preventDefault();
				handleSave();
			}
		};

		if (open) {
			document.addEventListener("keydown", handleKeyDown);
		}

		return () => {
			document.removeEventListener("keydown", handleKeyDown);
		};
	}, [open, handleCancel, handleSave]);

	if (!open) return null;

	return (
		<div className="absolute inset-0 z-50 bg-background flex flex-col">
			<div className="flex items-center justify-between px-6 py-4 border-b bg-background">
				<h2 className="text-xl font-semibold">Edit Message</h2>
				<div className="flex gap-2">
					<Button variant="outline" onClick={handleCancel}>
						Cancel
					</Button>
					<Button onClick={handleSave}>
						<CheckIcon className="w-4 h-4 mr-2" />
						Save Changes
					</Button>
				</div>
			</div>
			<div className="flex-1 p-6 ">
				<div className="relative h-full border border-border rounded-lg">
					<TextEditor
						appId={appId}
						initialContent={content}
						onChange={setEditedContent}
						isMarkdown={true}
						editable={true}
					/>
				</div>
			</div>
		</div>
	);
};

const FeedbackDialog = ({
	open,
	onOpenChange,
	initialComment,
	initialIncludeChatHistory,
	initialCanContact,
	onSubmit,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	initialComment: string;
	initialIncludeChatHistory: boolean;
	initialCanContact: boolean;
	onSubmit: (data: {
		comment: string;
		includeChatHistory: boolean;
		canContact: boolean;
	}) => void;
}) => {
	const [feedbackComment, setFeedbackComment] = useState(initialComment);
	const [includeChatHistory, setIncludeChatHistory] = useState(
		initialIncludeChatHistory,
	);
	const [canContact, setCanContact] = useState(initialCanContact);

	useEffect(() => {
		if (open) {
			setFeedbackComment(initialComment);
			setIncludeChatHistory(initialIncludeChatHistory);
			setCanContact(initialCanContact);
		}
	}, [open, initialComment, initialIncludeChatHistory, initialCanContact]);

	const handleSubmit = useCallback(() => {
		onSubmit({ comment: feedbackComment, includeChatHistory, canContact });
		onOpenChange(false);
	}, [feedbackComment, includeChatHistory, canContact, onSubmit, onOpenChange]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-125">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<MessageSquareIcon className="w-5 h-5 text-primary" />
						Share Additional Feedback
					</DialogTitle>
					<DialogDescription>
						Help us improve by sharing more details about your experience with
						this response.
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label>Your feedback</Label>
						<Textarea
							placeholder="Tell us what you think about this response..."
							value={feedbackComment}
							onChange={(e) => setFeedbackComment(e.target.value)}
							className="min-h-25 resize-none"
						/>
					</div>

					<div className="space-y-3">
						<div className="flex items-center space-x-2">
							<Switch
								id="chat-history"
								checked={includeChatHistory}
								onCheckedChange={setIncludeChatHistory}
							/>
							<Label htmlFor="chat-history">
								Include chat history with feedback
							</Label>
						</div>

						<div className="flex items-center space-x-2">
							<Switch
								id="can-contact"
								checked={canContact}
								onCheckedChange={setCanContact}
							/>
							<Label htmlFor="can-contact">
								You may contact me about this feedback
							</Label>
						</div>
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						Cancel
					</Button>
					<Button onClick={handleSubmit}>Submit Feedback</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};

const MessageActions = ({
	isUser,
	hasFooterContent,
	compact = false,
	rating,
	gaveMoreFeedback,
	onThumbsUp,
	onThumbsDown,
	onFeedbackClick,
	onEdit,
	onCopy,
}: {
	isUser: boolean;
	hasFooterContent: boolean;
	compact?: boolean;
	rating: number;
	gaveMoreFeedback: boolean;
	onThumbsUp: () => void;
	onThumbsDown: () => void;
	onFeedbackClick: () => void;
	onEdit: () => void;
	onCopy: () => void;
}) => (
	<div
		className={cn(
			"flex flex-row items-center gap-3",
			compact
				? "absolute bottom-3 right-4 z-10 h-auto w-auto gap-2"
				: cn(
						"h-6 w-full",
						isUser
							? hasFooterContent
								? "justify-end px-2 pt-2 mt-2"
								: "justify-end px-2 mt-0.5"
							: hasFooterContent
								? "justify-start mt-2"
								: "justify-start mt-0.5",
					),
		)}
	>
		{!isUser && (
			<>
				<FeedbackButton
					onClick={onThumbsUp}
					isActive={rating > 0}
					variant="positive"
				>
					<ThumbsUpIcon
						className={cn("w-4 h-4", rating > 0 && "fill-current")}
					/>
				</FeedbackButton>
				<FeedbackButton
					onClick={onThumbsDown}
					isActive={rating < 0}
					variant="negative"
				>
					<ThumbsDownIcon
						className={cn("w-4 h-4", rating < 0 && "fill-current")}
					/>
				</FeedbackButton>
			</>
		)}
		{rating !== 0 && (
			<button onClick={onFeedbackClick}>
				<Badge
					variant={gaveMoreFeedback ? "outline" : "default"}
					className="h-6 rounded-full"
				>
					{gaveMoreFeedback ? "✅ Feedback provided" : "Provide feedback"}
				</Badge>
			</button>
		)}
		{!isUser && (
			<MessageActionButton onClick={onEdit} title="Edit message">
				<EditIcon className="w-4 h-4" />
			</MessageActionButton>
		)}
		<MessageActionButton onClick={onCopy} title="Copy message">
			<CopyIcon className="w-4 h-4" />
		</MessageActionButton>
	</div>
);

export const MessageComponent = memo(
	function MessageComponent({
		message,
		loading,
		onMessageUpdate,
		appId,
		boardId,
		eventId,
	}: Readonly<MessageProps>) {
		const isUser = message.inner.role === IRole.User;
		const [isExpanded, setIsExpanded] = useState(false);
		const [showToggle, setShowToggle] = useState(false);
		const [fullscreenFile, setFullscreenFile] =
			useState<ProcessedAttachment | null>(null);
		const [showFeedbackDialog, setShowFeedbackDialog] = useState(false);
		const [showEditDialog, setShowEditDialog] = useState(false);
		const [showFileDialog, setShowFileDialog] = useState(false);
		const [dialogSelectedFile, setDialogSelectedFile] =
			useState<ProcessedAttachment | null>(null);
		const contentRef = useRef<HTMLDivElement>(null);
		const [collapsedMaxHeight, setCollapsedMaxHeight] = useState(
			COLLAPSED_ASK_LINES * 22,
		);

		const messageContent = useMemo(() => {
			if (typeof message.inner.content === "string") {
				return {
					text: message.inner.content,
					attachments: message.files ?? [],
				};
			}

			let text = "";
			const attachments: IAttachment[] = [];

			for (const part of message.inner.content) {
				if (part.text) {
					text += `${part.text}\n`;
					continue;
				}
				if (part.image_url?.url)
					attachments.push({
						url: part.image_url.url,
						type: part.image_url.media_type ?? "image/*",
					});
				else if (part.audio_url)
					attachments.push({
						url: part.audio_url,
						type: part.media_type,
					});
				else if (part.video_url)
					attachments.push({
						url: part.video_url,
						type: part.media_type,
					});
				else if (part.document_url)
					attachments.push({
						url: part.document_url,
						type: part.media_type,
					});
			}

			const uniqueAttachments = new Map<string, IAttachment>();
			for (const attachment of [...attachments, ...(message.files ?? [])]) {
				const url =
					typeof attachment === "string" ? attachment : attachment.url;
				const existing = uniqueAttachments.get(url);
				if (!existing || typeof attachment !== "string") {
					uniqueAttachments.set(url, attachment);
				}
			}

			return { text, attachments: [...uniqueAttachments.values()] };
		}, [message.inner.content, message.files]);

		const processedAttachments = useProcessedAttachments(
			messageContent.attachments,
		);

		// A long paste should not push the answer off screen. The cap is measured
		// from the rendered line height rather than assumed, and the observer stays
		// attached so the toggle disappears again if the content shrinks.
		useEffect(() => {
			if (!isUser) return;
			const el = contentRef.current;
			if (!el) return;

			const evaluate = () => {
				const lineHeight =
					Number.parseFloat(window.getComputedStyle(el).lineHeight) || 22;
				const cap = Math.round(lineHeight * COLLAPSED_ASK_LINES);
				setCollapsedMaxHeight(cap);
				setShowToggle(el.scrollHeight > cap + 4);
			};

			evaluate();
			// Collapsing caps the height of the observed element itself, so the
			// re-measure has to happen a frame after the notification rather than
			// inside it.
			return observeResize([el], evaluate);
		}, [message.inner, isUser]);

		const handleFileClick = useCallback((file: ProcessedAttachment) => {
			// A cited page is a destination, not a file — downloading its markup is
			// never what the external-link affordance promised.
			if (file.type === "website") {
				window.open(file.url, "_blank", "noopener,noreferrer");
			} else if (canPreviewFile(file)) {
				// Open file dialog with this file selected
				setDialogSelectedFile(file);
				setShowFileDialog(true);
			} else {
				// Download non-previewable files
				downloadFile(file);
			}
		}, []);

		const showAllAttachments = useCallback(() => {
			setDialogSelectedFile(null);
			setShowFileDialog(true);
		}, []);

		const copyToClipboard = useCallback(() => {
			if (messageContent.text) {
				navigator.clipboard
					.writeText(messageContent.text)
					.then(() => toast.success("Message copied to clipboard"))
					.catch((err) => console.error("Failed to copy message: ", err));
			}
		}, [messageContent.text]);

		const upsertFeedback = useCallback(
			async (rating: number) => {
				if (!onMessageUpdate) return;

				const currentRating = message.rating ?? 0;
				const newRating = currentRating === rating ? 0 : rating;

				try {
					await onMessageUpdate(message.id, {
						rating: newRating,
						ratingSettings:
							newRating === 0 ? undefined : message.ratingSettings,
					});

					if (newRating > 0) {
						toast.success("Thanks for the feedback! ❤️");
					} else if (newRating < 0) {
						setShowFeedbackDialog(true);
					}
				} catch (e) {
					console.error("[Chat] Failed to update feedback:", e);
					toast.error("Failed to submit feedback");
				}
			},
			[message.id, message.rating, message.ratingSettings, onMessageUpdate],
		);

		const handleFeedbackSubmit = useCallback(
			async (data: {
				comment: string;
				includeChatHistory: boolean;
				canContact: boolean;
			}) => {
				if (!onMessageUpdate) return;

				try {
					await onMessageUpdate(message.id, {
						ratingSettings: {
							comment: data.comment.trim(),
							includeChatHistory: data.includeChatHistory,
							canContact: data.canContact,
						},
					});
					toast.success("Feedback submitted successfully!");
				} catch (e) {
					console.error("[Chat] Failed to submit feedback:", e);
					toast.error("Failed to submit feedback");
				}
			},
			[message.id, onMessageUpdate],
		);

		const handleEditSave = useCallback(
			(content: string) => {
				if (!onMessageUpdate) return;

				const trimmedContent = content.trim();
				if (trimmedContent !== messageContent.text) {
					onMessageUpdate(message.id, {
						inner: {
							...message.inner,
							content: trimmedContent,
						},
					});
					toast.success("Message updated successfully!");
				}
			},
			[messageContent.text, message.id, message.inner, onMessageUpdate],
		);

		const gaveMoreFeedback = useMemo(() => {
			return Boolean(
				message.ratingSettings &&
					(message.ratingSettings.comment ||
						message.ratingSettings.includeChatHistory ||
						message.ratingSettings.canContact),
			);
		}, [message.ratingSettings]);

		const planSteps = useMemo(() => {
			if (isUser || !message.plan_steps) {
				return [];
			}

			return message.plan_steps.filter((step) => {
				const isEmptyFallbackStep =
					step.id === "step-0" &&
					step.title === "Thinking" &&
					!step.description?.trim() &&
					!step.reasoning?.trim();

				return !isEmptyFallbackStep;
			});
		}, [isUser, message.plan_steps]);

		const currentPlanStepId =
			message.current_step_id &&
			planSteps.some((step) => step.id === message.current_step_id)
				? message.current_step_id
				: undefined;

		// Anchors index into the RAW content string — only trust them when content is that string
		// (array-form content is re-joined for display, which shifts offsets).
		const inlineSegments = useMemo(
			() =>
				isUser || typeof message.inner.content !== "string"
					? null
					: buildInlineSegments(messageContent.text, planSteps),
			[isUser, message.inner.content, messageContent.text, planSteps],
		);

		// Which segments are still moving while the turn streams: the growing text tail (not
		// necessarily the last segment — an action anchored at the end sorts after it) and the one
		// action group that can still gain steps.
		const { lastTextSegmentIndex, liveSegmentIndex } = useMemo(() => {
			let lastText = -1;
			let lastSteps = -1;
			let withCurrent = -1;
			inlineSegments?.forEach((segment, index) => {
				if (segment.steps) {
					lastSteps = index;
					if (
						currentPlanStepId &&
						segment.steps.some((step) => step.id === currentPlanStepId)
					) {
						withCurrent = index;
					}
					return;
				}
				lastText = index;
			});
			return {
				lastTextSegmentIndex: lastText,
				liveSegmentIndex: withCurrent === -1 ? lastSteps : withCurrent,
			};
		}, [inlineSegments, currentPlanStepId]);

		const usageStats = !isUser ? (message.usage_stats ?? []) : [];
		const hasUsageStats = usageStats.length > 0;
		const hasFooterContent =
			hasUsageStats ||
			processedAttachments.length > 0 ||
			Boolean(FLOWPILOT_DEBUG_ENABLED && !isUser && message.debug_report);
		const compactUserActions = isUser && !hasFooterContent;

		return (
			<>
				<div
					className="flex w-full flex-col items-start gap-1 transition-all duration-300 ease-in-out"
					style={{
						maxWidth:
							"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
					}}
				>
					<div
						className={cn(
							"whitespace-break-spaces transition-all duration-300 ease-in-out",
							compactUserActions && "relative",
							isUser
								? "w-full border-l-2 py-2 pr-4 pl-3.5"
								: "w-full max-w-full p-4 pt-2 pb-0",
						)}
						data-fl-chat-message={isUser ? "user" : "assistant"}
						style={{
							backgroundColor: isUser
								? "var(--fl-chat-ask-background, transparent)"
								: "var(--fl-chat-ai-message-background, var(--background))",
							borderLeftColor: isUser
								? "var(--fl-chat-ask-rule, var(--primary))"
								: undefined,
							borderRadius: isUser
								? "0 var(--fl-chat-message-radius, 0.75rem) var(--fl-chat-message-radius, 0.75rem) 0"
								: "var(--fl-chat-message-radius, 0.75rem)",
							color: isUser
								? "var(--fl-chat-user-message-foreground, var(--foreground))"
								: "var(--fl-chat-ai-message-foreground, var(--foreground))",
							maxWidth: isUser ? "var(--fl-chat-measure, 38rem)" : undefined,
						}}
					>
						{isUser && (
							<span className="mb-1.5 block text-[10px] font-semibold uppercase tracking-widest text-muted-foreground/70">
								Asked
							</span>
						)}
						{!isUser && !inlineSegments && planSteps.length > 0 && (
							<PlanSteps
								steps={planSteps}
								currentStepId={currentPlanStepId}
								loading={loading}
							/>
						)}
						<div
							ref={contentRef}
							className={cn(
								"w-full max-w-full whitespace-break-spaces text-wrap text-sm leading-relaxed",
								compactUserActions && "pr-10",
								isUser && showToggle && !isExpanded && "overflow-hidden",
							)}
							style={
								// Only clamp + fade a message that genuinely overflows. Applying
								// the mask unconditionally faded every short question.
								isUser && showToggle && !isExpanded
									? {
											maxHeight: collapsedMaxHeight,
											WebkitMaskImage:
												"linear-gradient(to bottom, #000 calc(100% - 2rem), transparent)",
											maskImage:
												"linear-gradient(to bottom, #000 calc(100% - 2rem), transparent)",
										}
									: undefined
							}
						>
							{inlineSegments ? (
								<>
									{inlineSegments.map((segment, index) =>
										segment.steps ? (
											<InlineStepGroup
												key={segment.key}
												steps={segment.steps}
												// Only the group owning the active step gets the live
												// flags. Earlier groups are frozen (anchors never move
												// backwards), so handing them the bubble's `loading`
												// would spin them at "Working…" for the whole turn.
												currentStepId={
													segment.steps.some(
														(step) => step.id === currentPlanStepId,
													)
														? currentPlanStepId
														: undefined
												}
												loading={loading && index === liveSegmentIndex}
												// …but every group stays expanded until the whole
												// turn settles, or finished groups fold away one by
												// one mid-stream and the run looks frozen.
												turnActive={loading}
											/>
										) : loading && index === lastTextSegmentIndex ? (
											<div key={segment.key} data-fl-chat-prose>
												<StreamingTextEditor content={segment.text ?? ""} />
											</div>
										) : (
											<div key={segment.key} data-fl-chat-prose>
												<TextEditor
													initialContent={segment.text ?? ""}
													isMarkdown={true}
													editable={false}
												/>
											</div>
										),
									)}
									{loading &&
										Boolean(
											inlineSegments[inlineSegments.length - 1]?.steps,
										) && <ThinkingIndicator />}
								</>
							) : loading && !isUser && messageContent.text === "" ? (
								<ThinkingIndicator />
							) : loading && !isUser && messageContent.text !== "" ? (
								<div data-fl-chat-prose>
									<StreamingTextEditor content={messageContent.text} />
								</div>
							) : isUser ? (
								<TextEditor
									initialContent={messageContent.text}
									isMarkdown={true}
									editable={false}
								/>
							) : (
								<div data-fl-chat-prose>
									<TextEditor
										initialContent={messageContent.text}
										isMarkdown={true}
										editable={false}
									/>
								</div>
							)}
						</div>
						{isUser && showToggle && (
							<Button
								variant="ghost"
								size="sm"
								onClick={() => setIsExpanded(!isExpanded)}
								className="-ml-1 mt-1 h-auto gap-1 p-1 text-xs text-muted-foreground hover:text-foreground"
							>
								{isExpanded ? (
									<>
										<ChevronUp className="w-3 h-3" />
										Show less
									</>
								) : (
									<>
										<ChevronDown className="w-3 h-3" />
										Show more
									</>
								)}
							</Button>
						)}
						<AttachmentStrip
							files={processedAttachments}
							onFileClick={handleFileClick}
							onFullscreen={setFullscreenFile}
							onShowAll={showAllAttachments}
						/>
						{(message.widgets?.length ?? 0) > 0 && (
							<MessageWidgets
								widgets={message.widgets}
								appId={appId}
								boardId={boardId}
								eventId={eventId}
							/>
						)}
						{!isUser && (message.app_refs?.length ?? 0) > 0 && (
							<AppReferences appIds={message.app_refs ?? []} />
						)}
						{hasUsageStats && (
							<UsageStats stats={usageStats} className="mt-2" />
						)}
						{FLOWPILOT_DEBUG_ENABLED && !isUser && message.debug_report && (
							<AgentDebugReport report={message.debug_report} />
						)}
						{!loading && (
							<MessageActions
								isUser={isUser}
								hasFooterContent={hasFooterContent}
								compact={compactUserActions}
								rating={message.rating ?? 0}
								gaveMoreFeedback={gaveMoreFeedback}
								onThumbsUp={() => upsertFeedback(1)}
								onThumbsDown={() => upsertFeedback(-1)}
								onFeedbackClick={() => setShowFeedbackDialog(true)}
								onEdit={() => setShowEditDialog(true)}
								onCopy={copyToClipboard}
							/>
						)}
					</div>
				</div>
				{fullscreenFile && (
					<Dialog
						open={!!fullscreenFile}
						onOpenChange={() => setFullscreenFile(null)}
					>
						<DialogContent
							showCloseButton={false}
							className="w-dvw h-dvh max-w-none! max-h-none! p-0 gap-0 overflow-hidden bg-black text-white border-0 rounded-none top-[50%]! left-[50%]! translate-x-[-50%]! translate-y-[-50%]!"
						>
							<div className="relative w-full h-full flex flex-col">
								<div
									className="absolute top-0 left-0 right-0 z-10 flex items-center justify-between gap-2 px-3 pb-6 bg-linear-to-b from-black/80 to-transparent pointer-events-none"
									style={{
										paddingTop:
											"calc(var(--fl-safe-top, env(safe-area-inset-top, 0px)) + 0.75rem)",
									}}
								>
									<p className="min-w-0 flex-1 text-white text-sm font-medium truncate">
										{getDisplayFileName(fullscreenFile.name)}
									</p>
									<DialogClose asChild>
										<Button
											variant="ghost"
											size="icon"
											className="pointer-events-auto size-10 shrink-0 rounded-full bg-black/40 text-white hover:bg-black/60 hover:text-white"
										>
											<XIcon className="size-5" />
											<span className="sr-only">Close</span>
										</Button>
									</DialogClose>
								</div>
								<div
									className="flex-1 min-h-0 flex items-center justify-center w-full"
									style={{
										paddingBottom:
											"var(--fl-safe-bottom, env(safe-area-inset-bottom, 0px))",
									}}
								>
									<FileDialogPreview file={fullscreenFile} />
								</div>
							</div>
						</DialogContent>
					</Dialog>
				)}
				<FullscreenEditDialog
					appId={appId}
					open={showEditDialog}
					onOpenChange={setShowEditDialog}
					content={messageContent.text}
					onSave={handleEditSave}
				/>
				<FeedbackDialog
					open={showFeedbackDialog}
					onOpenChange={setShowFeedbackDialog}
					initialComment={message.ratingSettings?.comment ?? ""}
					initialIncludeChatHistory={
						message.ratingSettings?.includeChatHistory ?? false
					}
					initialCanContact={message.ratingSettings?.canContact ?? false}
					onSubmit={handleFeedbackSubmit}
				/>
				{processedAttachments.length > 0 && (
					<FileDialog
						files={processedAttachments}
						handleFileClick={handleFileClick}
						open={showFileDialog}
						onOpenChange={setShowFileDialog}
						initialSelectedFile={dialogSelectedFile}
						trigger={null}
					/>
				)}
			</>
		);
	},
	(prev, next) => {
		return (
			prev.message.inner.content === next.message.inner.content &&
			prev.message.files === next.message.files &&
			prev.message.rating === next.message.rating &&
			prev.message.ratingSettings === next.message.ratingSettings &&
			prev.message.plan_steps === next.message.plan_steps &&
			prev.message.current_step_id === next.message.current_step_id &&
			prev.message.usage_stats === next.message.usage_stats &&
			prev.message.debug_report === next.message.debug_report &&
			prev.message.app_refs === next.message.app_refs &&
			prev.message.widgets === next.message.widgets &&
			prev.appId === next.appId &&
			prev.boardId === next.boardId &&
			prev.eventId === next.eventId &&
			prev.loading === next.loading &&
			prev.onMessageUpdate === next.onMessageUpdate
		);
	},
);
MessageComponent.displayName = "MessageComponent";
