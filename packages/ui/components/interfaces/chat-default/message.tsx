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
} from "lucide-react";
import {
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { IRole, cn } from "../../../lib";
import {
	Badge,
	Button,
	Dialog,
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
import { FilePreview, type ProcessedAttachment } from "./attachment";
import {
	FileDialog,
	FileDialogPreview,
	canPreviewFile,
	downloadFile,
} from "./attachment-dialog";
import type { IAttachment, IMessage } from "./chat-db";
import { useProcessedAttachments } from "./hooks/use-processed-attachments";
import { PlanSteps } from "./plan-steps";
import { UsageStats } from "./usage-stats";

interface MessageProps {
	message: IMessage;
	loading?: boolean;
	onMessageUpdate?: (
		messageId: string,
		updates: Partial<IMessage>,
	) => void | Promise<void>;
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
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	content: string;
	onSave: (content: string) => void;
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
			<DialogContent className="sm:max-w-[500px]">
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
							className="min-h-[100px] resize-none"
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
	rating,
	gaveMoreFeedback,
	onThumbsUp,
	onThumbsDown,
	onFeedbackClick,
	onEdit,
	onCopy,
	allFiles,
	hiddenFilesCount,
	onFileClick,
}: {
	isUser: boolean;
	rating: number;
	gaveMoreFeedback: boolean;
	onThumbsUp: () => void;
	onThumbsDown: () => void;
	onFeedbackClick: () => void;
	onEdit: () => void;
	onCopy: () => void;
	allFiles: ProcessedAttachment[];
	hiddenFilesCount: number;
	onFileClick: (file: ProcessedAttachment) => void;
}) => (
	<div
		className={cn(
			"flex flex-row items-center gap-3 h-6 w-full mt-2",
			isUser ? "justify-end px-2 pt-2" : "justify-start",
		)}
	>
		{!isUser && (
			<>
				<FeedbackButton onClick={onThumbsUp} isActive={rating > 0} variant="positive">
					<ThumbsUpIcon className={cn("w-4 h-4", rating > 0 && "fill-current")} />
				</FeedbackButton>
				<FeedbackButton onClick={onThumbsDown} isActive={rating < 0} variant="negative">
					<ThumbsDownIcon className={cn("w-4 h-4", rating < 0 && "fill-current")} />
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
		{hiddenFilesCount > 0 && (
			<FileDialog files={allFiles} handleFileClick={onFileClick} />
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

const AttachmentSection = ({
	files,
	onFileClick,
	onFullscreen,
}: {
	files: ProcessedAttachment[];
	onFileClick: (file: ProcessedAttachment) => void;
	onFullscreen?: (file: ProcessedAttachment) => void;
}) => {
	const { visibleAudio, visibleImages, visibleVideo, visibleDocuments } =
		useMemo(() => {
			const audioFiles = files.filter((file) => file.type === "audio");
			const imageFiles = files.filter((file) => file.type === "image");
			const videoFiles = files.filter((file) => file.type === "video");
			const documentFiles = files.filter(
				(file) => !["audio", "image", "video"].includes(file.type),
			);

			return {
				visibleAudio: audioFiles.slice(0, 1),
				visibleImages: imageFiles.slice(0, 4),
				visibleVideo: videoFiles.slice(0, 1),
				visibleDocuments: documentFiles.slice(0, 3),
			};
		}, [files]);

	const getImageGridClassName = useCallback((count: number) => {
		if (count === 1) return "grid-cols-1";
		if (count === 2) return "grid-cols-2";
		if (count >= 3) return "grid-cols-2";
		return "grid-cols-1";
	}, []);

	return (
		<>
			{visibleAudio.length > 0 && (
				<div className="mt-2 max-w-md">
					{visibleAudio.map((file) => (
						<FilePreview key={file.url} file={file} onClick={onFileClick} />
					))}
				</div>
			)}

			{visibleImages.length > 0 && (
				<div
					className={cn(
						"mt-2 grid gap-1.5 max-w-md",
						getImageGridClassName(visibleImages.length),
					)}
				>
					{visibleImages.map((file) => (
						<FilePreview
							key={file.url}
							file={file}
							showFullscreenButton={true}
							onFullscreen={onFullscreen}
							inGrid={visibleImages.length > 1}
						/>
					))}
				</div>
			)}

			{visibleVideo.length > 0 && (
				<div className="mt-2 max-w-md">
					{visibleVideo.map((file) => (
						<FilePreview key={file.url} file={file} onClick={onFileClick} />
					))}
				</div>
			)}

			{visibleDocuments.length > 0 && (
				<div className="mt-2 flex flex-col gap-2 max-w-md">
					{visibleDocuments.map((file) => (
						<button
							key={file.url}
							onClick={() => onFileClick(file)}
							className="flex flex-col gap-1 p-3 rounded-lg border bg-muted/30 hover:bg-muted/50 transition-colors text-left"
						>
							<div className="flex items-center gap-2">
								<Badge variant="outline" className="text-xs capitalize">
									{file.type}
								</Badge>
								<span className="text-sm font-medium truncate flex-1">
									{file.name}
								</span>
								{file.pageNumber !== undefined && (
									<Badge variant="secondary" className="text-xs">
										Page {file.pageNumber}
									</Badge>
								)}
							</div>
							{file.previewText && (
								<p className="text-xs text-muted-foreground line-clamp-2">
									{file.previewText}
								</p>
							)}
						</button>
					))}
				</div>
			)}
		</>
	);
};

export const MessageComponent = memo(function MessageComponent({
	message,
	loading,
	onMessageUpdate,
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

	const maxCollapsedHeight = "4rem";

	const getDisplayFileName = useCallback((name: string) => {
		try {
			const decoded = decodeURIComponent(name);
			const parts = decoded.split(/[/\\]/);
			return parts[parts.length - 1];
		} catch {
			return name;
		}
	}, []);

	const messageContent = useMemo(() => {
		if (typeof message.inner.content === "string") {
			return { text: message.inner.content, attachments: message.files ?? [] };
		}

		let text = "";
		const attachments: IAttachment[] = [];

		for (const part of message.inner.content) {
			if (part.text) {
				text += `${part.text}\n`;
				continue;
			}
			if (part.image_url?.url) attachments.push(part.image_url?.url);
		}

		return { text, attachments: [...attachments, ...(message.files ?? [])] };
	}, [message.inner.content, message.files]);

	const processedAttachments = useProcessedAttachments(
		messageContent.attachments,
	);

	const hiddenFilesCount = useMemo(() => {
		const audioFiles = processedAttachments.filter(
			(file) => file.type === "audio",
		);
		const imageFiles = processedAttachments.filter(
			(file) => file.type === "image",
		);
		const videoFiles = processedAttachments.filter(
			(file) => file.type === "video",
		);
		const documentFiles = processedAttachments.filter(
			(file) => !["audio", "image", "video"].includes(file.type),
		);

		const hiddenAudio = audioFiles.slice(1);
		const hiddenImages = imageFiles.slice(4);
		const hiddenVideo = videoFiles.slice(1);
		const hiddenDocuments = documentFiles.slice(3);

		return (
			hiddenAudio.length +
			hiddenImages.length +
			hiddenVideo.length +
			hiddenDocuments.length
		);
	}, [processedAttachments]);

	useEffect(() => {
		if (!isUser || !contentRef.current) return;

		const el = contentRef.current;
		const maxHeight = Number.parseFloat(maxCollapsedHeight) * 16;

		if (el.scrollHeight > maxHeight) {
			setShowToggle(true);
			return;
		}

		const observer = new ResizeObserver(() => {
			if (el.scrollHeight > maxHeight) {
				setShowToggle(true);
				observer.disconnect();
			}
		});
		observer.observe(el);

		return () => observer.disconnect();
	}, [message.inner, isUser]);

	const handleFileClick = useCallback((file: ProcessedAttachment) => {
		if (canPreviewFile(file)) {
			// Open file dialog with this file selected
			setDialogSelectedFile(file);
			setShowFileDialog(true);
		} else {
			// Download non-previewable files
			downloadFile(file);
		}
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

	return (
		<>
			<div
				className={cn(
					"max-w-5xl flex gap-1 flex-col transition-all duration-300 ease-in-out",
					isUser ? "items-end" : "items-start",
				)}
			>
				<div
					className={cn(
						"rounded-xl rounded-tr-sm p-4 pt-2 whitespace-break-spaces transition-all duration-300 ease-in-out",
						isUser
							? "bg-muted dark:bg-muted/30 text-foreground max-w-3xl"
							: "bg-background text-foreground max-w-full w-full pb-0",
					)}
				>
					{!isUser && message.plan_steps && message.plan_steps.length > 0 && (
						<PlanSteps
							steps={message.plan_steps}
							currentStepId={message.current_step_id}
						/>
					)}
					<div
						ref={contentRef}
						className={cn(
							"text-sm leading-relaxed whitespace-break-spaces text-wrap max-w-full w-full",
							isUser && !isExpanded && "overflow-hidden",
						)}
						style={
							isUser && !isExpanded
								? { maxHeight: maxCollapsedHeight }
								: undefined
						}
					>
						{loading && !isUser && messageContent.text === "" ? (
							<div className="flex items-center gap-1.5 py-1">
								<div className="flex gap-1">
									<span className="w-1.5 h-1.5 rounded-full bg-muted-foreground/60 animate-bounce [animation-delay:0ms]" />
									<span className="w-1.5 h-1.5 rounded-full bg-muted-foreground/60 animate-bounce [animation-delay:150ms]" />
									<span className="w-1.5 h-1.5 rounded-full bg-muted-foreground/60 animate-bounce [animation-delay:300ms]" />
								</div>
								<span className="text-xs text-muted-foreground ml-1">Thinking...</span>
							</div>
						) : loading && !isUser && messageContent.text !== "" ? (
							<StreamingTextEditor content={messageContent.text} />
						) : (
							<TextEditor
								initialContent={messageContent.text}
								isMarkdown={true}
								editable={false}
							/>
						)}
					</div>{" "}
					{isUser && showToggle && (
						<Button
							variant="ghost"
							size="sm"
							onClick={() => setIsExpanded(!isExpanded)}
							className="h-auto p-0 text-xs text-foreground hover:text-foreground/80 mt-1"
						>
							{isExpanded ? (
								<>
									<ChevronUp className="w-3 h-3 mr-1" />
									Show less
								</>
							) : (
								<>
									<ChevronDown className="w-3 h-3 mr-1" />
									Show more
								</>
							)}
						</Button>
					)}
					<AttachmentSection
						files={processedAttachments}
						onFileClick={handleFileClick}
						onFullscreen={setFullscreenFile}
					/>
					{!isUser &&
						message.usage_stats &&
						message.usage_stats.length > 0 && (
							<UsageStats stats={message.usage_stats} className="mt-1" />
						)}
					{!loading && (
						<MessageActions
							isUser={isUser}
							rating={message.rating ?? 0}
							gaveMoreFeedback={gaveMoreFeedback}
							onThumbsUp={() => upsertFeedback(1)}
							onThumbsDown={() => upsertFeedback(-1)}
							onFeedbackClick={() => setShowFeedbackDialog(true)}

							onEdit={() => setShowEditDialog(true)}
							onCopy={copyToClipboard}
							allFiles={processedAttachments}
							hiddenFilesCount={hiddenFilesCount}
							onFileClick={handleFileClick}
						/>
					)}
				</div>
			</div>{" "}
			{fullscreenFile && (
				<Dialog
					open={!!fullscreenFile}
					onOpenChange={() => setFullscreenFile(null)}
				>
					<DialogContent className="w-screen h-screen max-w-none! max-h-none! p-0 bg-black border-0 rounded-none top-[50%]! left-[50%]! translate-x-[-50%]! translate-y-[-50%]!">
						<div className="relative w-full h-full flex flex-col">
							<div className="absolute top-0 left-0 right-0 z-10 flex items-center justify-start p-4 bg-linear-to-b from-black/80 to-transparent pointer-events-none">
								<p className="text-white text-sm font-medium truncate">
									{getDisplayFileName(fullscreenFile.name)}
								</p>
							</div>
							<div className="flex-1 flex items-center justify-center w-full h-full">
								<FileDialogPreview file={fullscreenFile} />
							</div>
						</div>
					</DialogContent>
				</Dialog>
			)}
			<FullscreenEditDialog
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
}, (prev, next) => {
	return (
		prev.message.inner.content === next.message.inner.content &&
		prev.message.files === next.message.files &&
		prev.message.rating === next.message.rating &&
		prev.message.ratingSettings === next.message.ratingSettings &&
		prev.message.plan_steps === next.message.plan_steps &&
		prev.message.current_step_id === next.message.current_step_id &&
		prev.message.usage_stats === next.message.usage_stats &&
		prev.loading === next.loading &&
		prev.onMessageUpdate === next.onMessageUpdate
	);
});
MessageComponent.displayName = "MessageComponent";
